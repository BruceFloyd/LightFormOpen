//! The MetaRPC control session: one bidirectional gRPC stream
//! (`/metarpc.MetaRPC/Talk`) carrying every command, reply and state push.
//!
//! - Commands go out as `serverRequest`; the device answers with a
//!   `serverReply` carrying the same `messageId`. Request and reply variants
//!   share their hashed oneof tag, which the client checks.
//! - After `Subscribe`, the device pushes state as `clientRequest` messages.
//!   The client answers each with an empty `clientReply` of the same id, as
//!   Creator's envelope suggests. That ack is inferred, not observed, so it can
//!   be turned off with [`Options::ack_state_pushes`].
//!
//! Connection details follow Creator 2.2.12: port 50050, then 50051; no TLS;
//! metadata `api-version: 1.3.2`; HTTP/2 keepalive every 10 s with a 5 s
//! timeout, also while idle.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use prost::Message;
use tokio::sync::{broadcast, mpsc, oneshot};
use tokio_stream::wrappers::ReceiverStream;
use tonic::codegen::http::uri::PathAndQuery;
use tonic::transport::Endpoint;

use crate::error::{Error, Result};
use crate::frame::{Frame, RawCodec, first_field};
use crate::proto::{
    self, DataFlag, ErrorType, MetaRpcClientReplyMessage, MetaRpcMessage,
    MetaRpcServerReplyMessage, MetaRpcServerRequestMessage, meta_rpc_message,
};

/// The API version Creator 2.2.12 announces.
pub const API_VERSION: &str = "1.3.2";
/// Control ports, in the order Creator tries them.
pub const PORTS: [u16; 2] = [50050, 50051];
const TALK: &str = "/metarpc.MetaRPC/Talk";

/// A command sent to the device.
pub type Command = proto::meta_rpc_server_request_message::Data;
/// The device's reply to a command.
pub type Reply = proto::meta_rpc_server_reply_message::Data;
/// A state push from the device.
pub type StateUpdate = proto::meta_rpc_client_request_message::Data;

#[derive(Debug, Clone)]
pub struct Options {
    /// Ports to try, in order.
    pub ports: Vec<u16>,
    pub connect_timeout: Duration,
    /// How long to wait for each reply. Required: most replies are empty, so a
    /// failed command may never be answered.
    pub reply_timeout: Duration,
    pub api_version: String,
    pub ack_state_pushes: bool,
    /// Fail `connect` when the opening `TestConnection` isn't answered as
    /// the schema expects. Turn off to keep a session whose replies don't
    /// decode, for diagnosis; the failure is then in [`Device::first_contact_error`].
    pub require_test_connection: bool,
}

impl Default for Options {
    fn default() -> Options {
        Options {
            ports: PORTS.to_vec(),
            connect_timeout: Duration::from_secs(5),
            reply_timeout: Duration::from_secs(5),
            api_version: API_VERSION.to_owned(),
            ack_state_pushes: true,
            require_test_connection: true,
        }
    }
}

/// Something the device sent that isn't a reply to a pending command.
#[derive(Debug, Clone)]
pub enum Event {
    /// A state push the schema recognizes. If `frame.is_lossless()` is false,
    /// some of its fields didn't decode.
    State {
        update: Box<StateUpdate>,
        frame: Frame,
    },
    /// A frame that decoded to no known message, or a reply nobody was waiting
    /// for. `frame.raw` holds the bytes.
    Unrecognized(Frame),
    /// The stream ended, with the error if there was one.
    Closed(Option<String>),
}

type Pending = Arc<Mutex<HashMap<u32, oneshot::Sender<Frame>>>>;

/// A connected device.
pub struct Device {
    target: String,
    server_api_version: Option<String>,
    outgoing: mpsc::Sender<MetaRpcMessage>,
    pending: Pending,
    next_id: AtomicU32,
    events: broadcast::Sender<Event>,
    reply_timeout: Duration,
    first_contact_error: Option<Error>,
}

impl Device {
    /// Connect to `host`, trying each of `options.ports` in turn, and confirm
    /// the session with a `TestConnection` round trip.
    pub async fn connect(host: &str, options: &Options) -> Result<Device> {
        let mut last = None;
        for &port in &options.ports {
            match Device::connect_port(host, port, options).await {
                Ok(device) => return Ok(device),
                Err(e) => last = Some(e),
            }
        }
        Err(last.unwrap_or_else(|| Error::Connect {
            target: host.to_owned(),
            reason: "no ports to try".into(),
        }))
    }

    async fn connect_port(host: &str, port: u16, options: &Options) -> Result<Device> {
        let target = if host.contains(':') {
            format!("[{host}]:{port}")
        } else {
            format!("{host}:{port}")
        };
        let fail = |reason: String| Error::Connect {
            target: target.clone(),
            reason,
        };
        let endpoint = Endpoint::from_shared(format!("http://{target}"))
            .map_err(|e| fail(e.to_string()))?
            .connect_timeout(options.connect_timeout)
            .http2_keep_alive_interval(Duration::from_secs(10))
            .keep_alive_timeout(Duration::from_secs(5))
            .keep_alive_while_idle(true);
        let channel = endpoint
            .connect()
            .await
            .map_err(|e| fail(error_chain(&e)))?;

        let (outgoing, requests) = mpsc::channel(64);
        let pending: Pending = Arc::default();
        let (events, _) = broadcast::channel(256);

        // Queue the first command before opening the stream, in case the
        // server holds its response headers until a message arrives.
        let test = Command::TestConnectionRequest(Default::default());
        let check = Expected::of(&test);
        let reply = register(&pending, 1);
        outgoing
            .send(request_message(1, test))
            .await
            .map_err(|_| Error::Disconnected)?;

        let mut request = tonic::Request::new(ReceiverStream::new(requests));
        let version = options
            .api_version
            .parse()
            .map_err(|_| fail("invalid api-version".into()))?;
        request.metadata_mut().insert("api-version", version);
        let mut grpc = tonic::client::Grpc::new(channel);
        let open = async {
            grpc.ready().await.map_err(|e| fail(error_chain(&e)))?;
            grpc.streaming(request, PathAndQuery::from_static(TALK), RawCodec)
                .await
                .map_err(Error::Rpc)
        };
        let response = tokio::time::timeout(options.connect_timeout, open)
            .await
            .map_err(|_| fail("timed out opening the control stream".into()))??;
        let server_api_version = response
            .metadata()
            .get("api-version")
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned);

        tokio::spawn(read_frames(
            response.into_inner(),
            pending.clone(),
            events.clone(),
            outgoing.clone(),
            options.ack_state_pushes,
        ));
        let mut device = Device {
            target,
            server_api_version,
            outgoing,
            pending,
            next_id: AtomicU32::new(2),
            events,
            reply_timeout: options.reply_timeout,
            first_contact_error: None,
        };
        match device.await_reply(1, check, reply).await {
            Ok(_) => {}
            // The stream is open, so a garbled or missing reply still leaves a
            // session worth watching when the caller asked to keep it.
            Err(e @ (Error::UnexpectedReply { .. } | Error::Timeout { .. }))
                if !options.require_test_connection =>
            {
                device.first_contact_error = Some(e);
            }
            Err(e) => return Err(e),
        }
        Ok(device)
    }

    /// Why the opening `TestConnection` failed, when
    /// [`Options::require_test_connection`] was off and it did.
    pub fn first_contact_error(&self) -> Option<&Error> {
        self.first_contact_error.as_ref()
    }

    /// The `host:port` this session is connected to.
    pub fn target(&self) -> &str {
        &self.target
    }

    /// The `api-version` the device sent when the stream opened, if any.
    pub fn server_api_version(&self) -> Option<&str> {
        self.server_api_version.as_deref()
    }

    /// State pushes and other unsolicited frames, from now on.
    pub fn events(&self) -> broadcast::Receiver<Event> {
        self.events.subscribe()
    }

    /// Send a command and wait for its reply.
    pub async fn request(&self, command: Command) -> Result<Reply> {
        let check = Expected::of(&command);
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let reply = register(&self.pending, id);
        if self
            .outgoing
            .send(request_message(id, command))
            .await
            .is_err()
        {
            self.pending.lock().unwrap().remove(&id);
            return Err(Error::Disconnected);
        }
        self.await_reply(id, check, reply).await
    }

    async fn await_reply(
        &self,
        id: u32,
        check: Expected,
        reply: oneshot::Receiver<Frame>,
    ) -> Result<Reply> {
        let frame = match tokio::time::timeout(self.reply_timeout, reply).await {
            Ok(Ok(frame)) => frame,
            Ok(Err(_)) => return Err(Error::Disconnected),
            Err(_) => {
                self.pending.lock().unwrap().remove(&id);
                return Err(Error::Timeout {
                    request: check.name,
                    seconds: self.reply_timeout.as_secs_f64(),
                });
            }
        };
        check.reply(frame)
    }

    pub async fn test_connection(&self) -> Result<()> {
        self.request(Command::TestConnectionRequest(Default::default()))
            .await
            .map(drop)
    }

    /// Ask for state pushes. `flags` is a mask of [`DataFlag`] bits.
    pub async fn subscribe(&self, flags: u32) -> Result<()> {
        self.request(Command::SubscribeRequest(proto::SubscribeRequestMessage {
            data_flags: flags,
        }))
        .await
        .map(drop)
    }

    /// Ask for every state push, as Creator does.
    pub async fn subscribe_all(&self) -> Result<()> {
        self.subscribe(DataFlag::DataflagAll as u32).await
    }

    pub async fn play(&self) -> Result<()> {
        self.request(Command::PlayRequest(Default::default()))
            .await
            .map(drop)
    }

    pub async fn pause(&self) -> Result<()> {
        self.request(Command::PauseRequest(Default::default()))
            .await
            .map(drop)
    }

    pub async fn stop(&self) -> Result<()> {
        self.request(Command::StopRequest(Default::default()))
            .await
            .map(drop)
    }

    pub async fn next_slide(&self) -> Result<()> {
        self.request(Command::NextSlideRequest(Default::default()))
            .await
            .map(drop)
    }

    pub async fn prev_slide(&self) -> Result<()> {
        self.request(Command::PrevSlideRequest(Default::default()))
            .await
            .map(drop)
    }

    pub async fn set_slide_index(&self, index: u32) -> Result<()> {
        let message = proto::SetSlideIndexRequestMessage { index };
        self.request(Command::SetSlideIndexRequest(message))
            .await
            .map(drop)
    }

    /// Brightness from 0.0 to 1.0 [I: range taken from Creator's slider].
    pub async fn set_brightness(&self, brightness: f32) -> Result<()> {
        let message = proto::SetGlobalBrightnessRequestMessage { brightness };
        self.request(Command::SetGlobalBrightnessRequest(message))
            .await
            .map(drop)
    }
}

/// What a command's reply must look like.
struct Expected {
    name: String,
    tag: Option<u32>,
}

impl Expected {
    fn of(command: &Command) -> Expected {
        let wrapper = MetaRpcServerRequestMessage {
            data: Some(command.clone()),
        };
        Expected {
            name: variant_name(command),
            tag: first_field(&wrapper.encode_to_vec()),
        }
    }

    /// Accept a reply frame only if it is a `serverReply` whose variant has the
    /// request's tag, then surface any error it carries.
    fn reply(self, frame: Frame) -> Result<Reply> {
        let reply = match frame.message.as_ref().and_then(|m| m.data.as_ref()) {
            Some(meta_rpc_message::Data::ServerReply(MetaRpcServerReplyMessage {
                data: Some(reply),
            })) => reply,
            _ => {
                return Err(Error::UnexpectedReply {
                    request: self.name,
                    raw: frame.raw,
                });
            }
        };
        let wrapper = MetaRpcServerReplyMessage {
            data: Some(reply.clone()),
        };
        if first_field(&wrapper.encode_to_vec()) != self.tag {
            return Err(Error::UnexpectedReply {
                request: self.name,
                raw: frame.raw,
            });
        }
        if let Some(error) = response_error(reply) {
            return Err(error);
        }
        Ok(reply.clone())
    }
}

/// Only five reply messages carry a `Response`; the rest are empty.
fn response_error(reply: &Reply) -> Option<Error> {
    let response = match reply {
        Reply::SetGlobalVolumeReply(r) => r.result.as_ref(),
        Reply::SetObjectCoordinatesReply(r) => r.result.as_ref(),
        Reply::OaklightOnReply(r) => r.result.as_ref(),
        Reply::OaklightSetObjectReply(r) => r.result.as_ref(),
        Reply::OaklightSetCustomObjectReply(r) => r.result.as_ref(),
        _ => None,
    }?;
    let error = response.error.as_ref()?;
    let kind = ErrorType::try_from(error.r#type).unwrap_or(ErrorType::ErrortypeUnknown);
    (kind != ErrorType::ErrortypeNoError).then(|| Error::Device {
        kind,
        message: error.message.clone(),
    })
}

fn variant_name(command: &Command) -> String {
    let debug = format!("{command:?}");
    let name = debug.split(['(', ' ', '{']).next().unwrap_or("command");
    name.strip_suffix("Request").unwrap_or(name).to_owned()
}

fn request_message(id: u32, command: Command) -> MetaRpcMessage {
    MetaRpcMessage {
        message_id: id,
        data: Some(meta_rpc_message::Data::ServerRequest(
            MetaRpcServerRequestMessage {
                data: Some(command),
            },
        )),
    }
}

fn register(pending: &Pending, id: u32) -> oneshot::Receiver<Frame> {
    let (tx, rx) = oneshot::channel();
    pending.lock().unwrap().insert(id, tx);
    rx
}

async fn read_frames(
    mut inbound: tonic::Streaming<Frame>,
    pending: Pending,
    events: broadcast::Sender<Event>,
    outgoing: mpsc::Sender<MetaRpcMessage>,
    ack: bool,
) {
    let reason = loop {
        match inbound.message().await {
            Ok(Some(frame)) => route(frame, &pending, &events, &outgoing, ack).await,
            Ok(None) => break None,
            Err(status) => break Some(status.to_string()),
        }
    };
    // Dropping the waiters' senders fails their requests with Disconnected.
    pending.lock().unwrap().clear();
    let _ = events.send(Event::Closed(reason));
}

async fn route(
    frame: Frame,
    pending: &Pending,
    events: &broadcast::Sender<Event>,
    outgoing: &mpsc::Sender<MetaRpcMessage>,
    ack: bool,
) {
    let Some(message) = frame.message.clone() else {
        let _ = events.send(Event::Unrecognized(frame));
        return;
    };
    match message.data {
        Some(meta_rpc_message::Data::ServerReply(_)) => {
            let waiter = pending.lock().unwrap().remove(&message.message_id);
            match waiter {
                Some(tx) => {
                    let _ = tx.send(frame);
                }
                None => {
                    let _ = events.send(Event::Unrecognized(frame));
                }
            }
        }
        Some(meta_rpc_message::Data::ClientRequest(push)) if push.data.is_some() => {
            if ack {
                let reply = MetaRpcMessage {
                    message_id: message.message_id,
                    data: Some(meta_rpc_message::Data::ClientReply(
                        MetaRpcClientReplyMessage {},
                    )),
                };
                let _ = outgoing.send(reply).await;
            }
            let update = Box::new(push.data.expect("checked above"));
            let _ = events.send(Event::State { update, frame });
        }
        _ => {
            let _ = events.send(Event::Unrecognized(frame));
        }
    }
}

/// tonic's transport errors hide the useful part (refused, timed out) in their sources.
fn error_chain(error: &dyn std::error::Error) -> String {
    let mut text = error.to_string();
    let mut source = error.source();
    while let Some(e) = source {
        let part = e.to_string();
        // Wrappers often repeat their source's message; skip the repeats.
        if !text.ends_with(&part) {
            text.push_str(": ");
            text.push_str(&part);
        }
        source = e.source();
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_commands_for_errors() {
        assert_eq!(
            variant_name(&Command::PlayRequest(Default::default())),
            "Play"
        );
        let slide = Command::SetSlideIndexRequest(proto::SetSlideIndexRequestMessage { index: 3 });
        assert_eq!(variant_name(&slide), "SetSlideIndex");
    }

    #[test]
    fn expects_the_matching_reply_tag() {
        let check = Expected::of(&Command::PlayRequest(Default::default()));
        assert_eq!(check.tag, Some(424_021_114));
    }

    #[test]
    fn surfaces_errors_from_replies_that_carry_a_response() {
        let reply = Reply::SetGlobalVolumeReply(proto::SetGlobalVolumeReplyMessage {
            result: Some(proto::Response {
                result: None,
                error: Some(proto::Error {
                    r#type: ErrorType::ErrortypeNoProjectLoaded as i32,
                    message: "none".into(),
                }),
            }),
        });
        assert!(matches!(
            response_error(&reply),
            Some(Error::Device {
                kind: ErrorType::ErrortypeNoProjectLoaded,
                ..
            })
        ));
    }
}
