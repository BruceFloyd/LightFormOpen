//! LAN discovery, as Creator 2.2.12 does it: a small JSON query sent to UDP
//! port 7350, answered on port 7351. Despite using the mDNS multicast group
//! (224.0.0.251), the payload is not DNS.
//!
//! Query (Creator sends it every 15 s):
//! `{"api": "0.0", "hostname": "<local host name>", "timestamp": "<string>"}`
//!
//! Reply [keys observed, nesting inferred, value formats unknown]:
//! `{"data": {"serialNumber", "name", "partNumber", "manufactureDate", "version", "services": {...}}}`
//!
//! Replies are kept raw as well, since their exact format hasn't been seen yet.

use std::collections::BTreeMap;
use std::io::ErrorKind;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, SocketAddrV4};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::Value;
use socket2::{Domain, Protocol, Socket, Type};
use tokio::net::UdpSocket;
use tokio::time::Instant;

use crate::error::{Error, Result};

pub const QUERY_PORT: u16 = 7350;
pub const REPLY_PORT: u16 = 7351;
pub const MULTICAST_GROUP: Ipv4Addr = Ipv4Addr::new(224, 0, 0, 251);

#[derive(Debug, Clone)]
pub struct Options {
    pub query_port: u16,
    /// Port to listen on. Replies may go to the query's source port or to
    /// 7351, so queries are sent from this port too. 0 picks any free port.
    pub reply_port: u16,
    pub timeout: Duration,
    /// How often to resend the query while waiting; UDP can drop it.
    pub resend_every: Duration,
    /// Also query these addresses directly (for example a known device IP, or
    /// 192.168.0.1 when joined to a device's pairing network).
    pub hosts: Vec<IpAddr>,
    pub multicast: bool,
    /// Send to the broadcast address of every IPv4 interface that is up.
    pub broadcast: bool,
}

impl Default for Options {
    fn default() -> Options {
        Options {
            query_port: QUERY_PORT,
            reply_port: REPLY_PORT,
            timeout: Duration::from_secs(3),
            resend_every: Duration::from_secs(1),
            hosts: Vec::new(),
            multicast: true,
            broadcast: true,
        }
    }
}

/// A device that answered. Any host on the network can answer, and the
/// strings are as received: escape control characters before printing them
/// to a terminal.
#[derive(Debug, Clone, PartialEq)]
pub struct Discovered {
    /// Where the reply came from.
    pub address: SocketAddr,
    pub serial_number: Option<String>,
    pub name: Option<String>,
    pub part_number: Option<String>,
    pub manufacture_date: Option<String>,
    pub version: Option<String>,
    /// Service name → value. Creator builds `http://` URLs from the values.
    pub services: BTreeMap<String, String>,
    /// The whole reply, parsed as JSON if it was JSON, else as a string.
    pub raw: Value,
}

impl Discovered {
    fn parse(address: SocketAddr, datagram: &[u8]) -> Discovered {
        let raw = serde_json::from_slice(datagram)
            .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(datagram).into_owned()));
        let data = raw.get("data").unwrap_or(&raw);
        let text = |key: &str| match data.get(key)? {
            Value::String(s) => Some(s.trim().to_owned()),
            Value::Null => None,
            other => Some(other.to_string()),
        };
        let services = data
            .get("services")
            .and_then(Value::as_object)
            .map(|m| {
                m.iter()
                    .map(|(k, v)| {
                        (
                            k.clone(),
                            v.as_str()
                                .map_or_else(|| v.to_string(), |s| s.trim().to_owned()),
                        )
                    })
                    .collect()
            })
            .unwrap_or_default();
        Discovered {
            address,
            serial_number: text("serialNumber"),
            name: text("name"),
            part_number: text("partNumber"),
            manufacture_date: text("manufactureDate"),
            version: text("version"),
            services,
            raw,
        }
    }

    fn key(&self) -> String {
        self.serial_number
            .clone()
            .unwrap_or_else(|| self.address.ip().to_string())
    }
}

/// The query Creator sends. The timestamp format isn't known; this sends
/// Unix seconds as a string.
pub fn query() -> Vec<u8> {
    let hostname = gethostname::gethostname().to_string_lossy().into_owned();
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    serde_json::json!({"api": "0.0", "hostname": hostname, "timestamp": timestamp.to_string()})
        .to_string()
        .into_bytes()
}

/// Query the network and collect replies until `options.timeout`.
pub async fn discover(options: &Options) -> Result<Vec<Discovered>> {
    let socket = bind(options)?;
    let targets = targets(options);
    if targets.is_empty() {
        return Err(Error::Discovery("nowhere to send the query".into()));
    }
    let query = query();
    let deadline = Instant::now() + options.timeout;
    let mut found: Vec<Discovered> = Vec::new();
    let mut buf = vec![0u8; 65_536];
    loop {
        send_all(&socket, &query, &targets).await?;
        let next_send = (Instant::now() + options.resend_every).min(deadline);
        while let Ok(received) =
            tokio::time::timeout_at(next_send, socket.recv_from(&mut buf)).await
        {
            let (len, from) = received?;
            let device = Discovered::parse(from, &buf[..len]);
            if !found.iter().any(|d| d.key() == device.key()) {
                found.push(device);
            }
        }
        if Instant::now() >= deadline {
            return Ok(found);
        }
    }
}

fn bind(options: &Options) -> Result<UdpSocket> {
    let socket = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))?;
    // Creator binds 7351 with ShareAddress, so it may be running alongside.
    socket.set_reuse_address(true)?;
    #[cfg(unix)]
    socket.set_reuse_port(true)?;
    socket.set_broadcast(true)?;
    socket.set_nonblocking(true)?;
    socket.bind(&SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, options.reply_port).into())?;
    if options.multicast {
        // Creator joins the group, which suggests replies may arrive by multicast.
        socket.join_multicast_v4(&MULTICAST_GROUP, &Ipv4Addr::UNSPECIFIED)?;
        socket.set_multicast_loop_v4(false)?;
    }
    Ok(UdpSocket::from_std(socket.into())?)
}

fn targets(options: &Options) -> Vec<SocketAddr> {
    let port = options.query_port;
    let mut targets: Vec<SocketAddr> = Vec::new();
    if options.multicast {
        targets.push(SocketAddr::new(MULTICAST_GROUP.into(), port));
    }
    if options.broadcast {
        for interface in if_addrs::get_if_addrs().unwrap_or_default() {
            if let if_addrs::IfAddr::V4(v4) = &interface.addr
                && !v4.ip.is_loopback()
                && let Some(broadcast) = v4.broadcast
            {
                targets.push(SocketAddr::new(broadcast.into(), port));
            }
        }
    }
    targets.extend(options.hosts.iter().map(|&ip| SocketAddr::new(ip, port)));
    targets.sort();
    targets.dedup();
    targets
}

/// Send to every target. Failing on some is normal (an interface without a
/// route); failing on all is not, and on macOS usually means Local Network
/// access hasn't been granted to the app running this.
async fn send_all(socket: &UdpSocket, query: &[u8], targets: &[SocketAddr]) -> Result<()> {
    let mut failures = Vec::new();
    for target in targets {
        if let Err(e) = socket.send_to(query, target).await {
            failures.push((target, e));
        }
    }
    if failures.len() < targets.len() {
        return Ok(());
    }
    let blocked = failures.iter().any(|(_, e)| {
        matches!(
            e.kind(),
            ErrorKind::HostUnreachable
                | ErrorKind::NetworkUnreachable
                | ErrorKind::PermissionDenied
        )
    });
    let detail: Vec<String> = failures.iter().map(|(t, e)| format!("{t}: {e}")).collect();
    let hint = if blocked && cfg!(target_os = "macos") {
        " (on macOS, check System Settings > Privacy & Security > Local Network for the app running this)"
    } else {
        ""
    };
    Err(Error::Discovery(format!(
        "couldn't send the query to any address{hint}: {}",
        detail.join("; ")
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_expected_reply_shape() {
        let reply = br#"{"data":{"serialNumber":" LF2-0001 ","name":"Studio","partNumber":"LF2",
            "version":"2.1.0","services":{"meta":"50050","upload":"7346"}}}"#;
        let device = Discovered::parse("10.0.0.5:7351".parse().unwrap(), reply);
        assert_eq!(device.serial_number.as_deref(), Some("LF2-0001"));
        assert_eq!(device.name.as_deref(), Some("Studio"));
        assert_eq!(
            device.services.get("meta").map(String::as_str),
            Some("50050")
        );
    }

    #[test]
    fn keeps_unexpected_replies() {
        let device = Discovered::parse("10.0.0.5:7351".parse().unwrap(), b"hello");
        assert_eq!(device.raw, Value::String("hello".into()));
        assert_eq!(device.serial_number, None);
        assert_eq!(device.key(), "10.0.0.5");
    }

    #[test]
    fn query_matches_creator() {
        let query: Value = serde_json::from_slice(&query()).unwrap();
        assert_eq!(query["api"], "0.0");
        assert!(query["hostname"].is_string() && query["timestamp"].is_string());
    }
}
