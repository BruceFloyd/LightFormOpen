//! The client against an in-process mock device built from the recovered
//! schema. This proves the client and the schema agree, not that a real
//! device behaves this way.

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use lf_device::proto::meta_rpc_message::Data as Envelope;
use lf_device::proto::meta_rpc_server::{MetaRpc, MetaRpcServer};
use lf_device::proto::meta_rpc_server_reply_message::Data as R;
use lf_device::proto::meta_rpc_server_request_message::Data as C;
use lf_device::proto::{self, MetaRpcMessage};
use lf_device::{Device, Error, Event, Options, StateUpdate};
use tokio::sync::mpsc;
use tokio_stream::wrappers::{ReceiverStream, TcpListenerStream};
use tonic::{Request, Response, Status, Streaming};

#[derive(Default)]
struct Log {
    api_version: Option<String>,
    acks: Vec<u32>,
    commands: Vec<String>,
}

#[derive(Clone, Default)]
struct Mock {
    log: Arc<Mutex<Log>>,
    /// Answer TestConnection with the wrong reply, as firmware with different
    /// field numbers would appear to.
    garble_test_connection: bool,
}

const PUSH_ID: u32 = 9001;

/// How the mock answers each command. `None` means it never replies.
fn answer(command: &C) -> Option<R> {
    Some(match command {
        C::TestConnectionRequest(_) => R::TestConnectionReply(Default::default()),
        C::SubscribeRequest(_) => R::SubscribeReply(Default::default()),
        C::PlayRequest(_) => R::PlayReply(Default::default()),
        // Wrong reply type, to check the client rejects it.
        C::PauseRequest(_) => R::PlayReply(Default::default()),
        C::SetSlideIndexRequest(_) => R::SetSlideIndexReply(Default::default()),
        C::SetGlobalVolumeRequest(_) => {
            R::SetGlobalVolumeReply(proto::SetGlobalVolumeReplyMessage {
                result: Some(proto::Response {
                    result: None,
                    error: Some(proto::Error {
                        r#type: proto::ErrorType::ErrortypeNoProjectLoaded as i32,
                        message: "no project".into(),
                    }),
                }),
            })
        }
        _ => return None,
    })
}

fn reply(id: u32, data: R) -> MetaRpcMessage {
    let data = proto::MetaRpcServerReplyMessage { data: Some(data) };
    MetaRpcMessage {
        message_id: id,
        data: Some(Envelope::ServerReply(data)),
    }
}

fn playback_push() -> MetaRpcMessage {
    let state = proto::PlaybackStateChangedRequestMessage {
        status: proto::PlaybackStatus::PlaybackstatusPlaying as i32,
        slide: 2,
        global_brightness: 0.75,
        ..Default::default()
    };
    let push = proto::MetaRpcClientRequestMessage {
        data: Some(
            proto::meta_rpc_client_request_message::Data::PlaybackStateChangedRequest(state),
        ),
    };
    MetaRpcMessage {
        message_id: PUSH_ID,
        data: Some(Envelope::ClientRequest(push)),
    }
}

#[tonic::async_trait]
impl MetaRpc for Mock {
    type TalkStream = ReceiverStream<Result<MetaRpcMessage, Status>>;

    async fn talk(
        &self,
        request: Request<Streaming<MetaRpcMessage>>,
    ) -> Result<Response<Self::TalkStream>, Status> {
        let version = request
            .metadata()
            .get("api-version")
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned);
        self.log.lock().unwrap().api_version = version;
        let mut inbound = request.into_inner();
        let (tx, rx) = mpsc::channel(16);
        let log = self.log.clone();
        let garble = self.garble_test_connection;
        tokio::spawn(async move {
            while let Ok(Some(message)) = inbound.message().await {
                match message.data {
                    Some(Envelope::ServerRequest(proto::MetaRpcServerRequestMessage {
                        data: Some(command),
                    })) => {
                        log.lock().unwrap().commands.push(format!("{command:?}"));
                        let garble = garble && matches!(command, C::TestConnectionRequest(_));
                        let answer = if garble {
                            Some(R::PlayReply(Default::default()))
                        } else {
                            answer(&command)
                        };
                        if let Some(data) = answer {
                            let _ = tx.send(Ok(reply(message.message_id, data))).await;
                        }
                        if matches!(command, C::SubscribeRequest(_)) {
                            let _ = tx.send(Ok(playback_push())).await;
                        }
                    }
                    Some(Envelope::ClientReply(_)) => {
                        log.lock().unwrap().acks.push(message.message_id)
                    }
                    _ => {}
                }
            }
        });
        let mut response = Response::new(ReceiverStream::new(rx));
        response
            .metadata_mut()
            .insert("api-version", "9.9.9".parse().unwrap());
        Ok(response)
    }
}

async fn start() -> (SocketAddr, Mock) {
    start_mock(Mock::default()).await
}

async fn start_mock(mock: Mock) -> (SocketAddr, Mock) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let service = MetaRpcServer::new(mock.clone());
    tokio::spawn(async move {
        tonic::transport::Server::builder()
            .add_service(service)
            .serve_with_incoming(TcpListenerStream::new(listener))
            .await
            .unwrap();
    });
    (addr, mock)
}

fn options(ports: Vec<u16>) -> Options {
    Options {
        ports,
        connect_timeout: Duration::from_secs(2),
        reply_timeout: Duration::from_millis(300),
        ..Options::default()
    }
}

#[tokio::test]
async fn connects_and_exchanges_versions() {
    let (addr, mock) = start().await;
    let device = Device::connect("127.0.0.1", &options(vec![addr.port()]))
        .await
        .unwrap();
    assert_eq!(device.server_api_version(), Some("9.9.9"));
    assert_eq!(
        mock.log.lock().unwrap().api_version.as_deref(),
        Some(lf_device::API_VERSION)
    );
    assert!(mock.log.lock().unwrap().commands[0].starts_with("TestConnectionRequest"));
}

#[tokio::test]
async fn falls_back_to_the_next_port() {
    let (addr, _mock) = start().await;
    // Bind and drop a listener to get a port that refuses connections.
    let closed = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let device = Device::connect("127.0.0.1", &options(vec![closed, addr.port()]))
        .await
        .unwrap();
    assert!(device.target().ends_with(&addr.port().to_string()));
}

#[tokio::test]
async fn commands_get_their_replies() {
    let (addr, mock) = start().await;
    let device = Device::connect("127.0.0.1", &options(vec![addr.port()]))
        .await
        .unwrap();
    device.play().await.unwrap();
    device.set_slide_index(3).await.unwrap();
    let commands = mock.log.lock().unwrap().commands.clone();
    assert!(
        commands.iter().any(|c| c.contains("index: 3")),
        "{commands:?}"
    );
}

#[tokio::test]
async fn subscribe_delivers_state_and_acks_it() {
    let (addr, mock) = start().await;
    let device = Device::connect("127.0.0.1", &options(vec![addr.port()]))
        .await
        .unwrap();
    let mut events = device.events();
    device.subscribe_all().await.unwrap();
    let event = tokio::time::timeout(Duration::from_secs(2), events.recv())
        .await
        .unwrap()
        .unwrap();
    let Event::State { update, frame } = event else {
        panic!("expected a state push, got {event:?}")
    };
    let StateUpdate::PlaybackStateChangedRequest(state) = *update else {
        panic!("wrong push: {update:?}")
    };
    assert_eq!((state.slide, state.global_brightness), (2, 0.75));
    assert!(frame.is_lossless());
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(mock.log.lock().unwrap().acks, vec![PUSH_ID]);
}

#[tokio::test]
async fn unanswered_commands_time_out() {
    let (addr, _mock) = start().await;
    let device = Device::connect("127.0.0.1", &options(vec![addr.port()]))
        .await
        .unwrap();
    let err = device.stop().await.unwrap_err();
    assert!(
        matches!(err, Error::Timeout { ref request, .. } if request == "Stop"),
        "{err}"
    );
    // The session is still usable afterwards.
    device.play().await.unwrap();
}

#[tokio::test]
async fn rejects_a_reply_of_the_wrong_type() {
    let (addr, _mock) = start().await;
    let device = Device::connect("127.0.0.1", &options(vec![addr.port()]))
        .await
        .unwrap();
    let err = device.pause().await.unwrap_err();
    assert!(
        matches!(err, Error::UnexpectedReply { ref request, .. } if request == "Pause"),
        "{err}"
    );
}

#[tokio::test]
async fn surfaces_device_errors() {
    let (addr, _mock) = start().await;
    let device = Device::connect("127.0.0.1", &options(vec![addr.port()]))
        .await
        .unwrap();
    let volume = C::SetGlobalVolumeRequest(proto::SetGlobalVolumeRequestMessage { volume: 0.5 });
    let err = device.request(volume).await.unwrap_err();
    assert!(
        matches!(
            err,
            Error::Device {
                kind: proto::ErrorType::ErrortypeNoProjectLoaded,
                ..
            }
        ),
        "{err}"
    );
}

#[tokio::test]
async fn refused_connection_is_an_error() {
    let closed = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let err = Device::connect("127.0.0.1", &options(vec![closed]))
        .await
        .err()
        .unwrap();
    assert!(matches!(err, Error::Connect { .. }), "{err}");
}

#[tokio::test]
async fn garbled_first_reply_fails_connect_by_default() {
    let (addr, _mock) = start_mock(Mock {
        garble_test_connection: true,
        ..Mock::default()
    })
    .await;
    let err = Device::connect("127.0.0.1", &options(vec![addr.port()]))
        .await
        .err()
        .unwrap();
    assert!(matches!(err, Error::UnexpectedReply { .. }), "{err}");
}

#[tokio::test]
async fn garbled_first_reply_can_keep_the_session_for_diagnosis() {
    let (addr, _mock) = start_mock(Mock {
        garble_test_connection: true,
        ..Mock::default()
    })
    .await;
    let options = Options {
        require_test_connection: false,
        ..options(vec![addr.port()])
    };
    let device = Device::connect("127.0.0.1", &options).await.unwrap();
    let Some(Error::UnexpectedReply { raw, .. }) = device.first_contact_error() else {
        panic!(
            "expected the garbled reply to be kept, got {:?}",
            device.first_contact_error()
        );
    };
    assert!(!raw.is_empty(), "the raw reply is available for --trace");
    // The session still works.
    let mut events = device.events();
    device.subscribe_all().await.unwrap();
    let event = tokio::time::timeout(Duration::from_secs(2), events.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(event, Event::State { .. }), "{event:?}");
}
