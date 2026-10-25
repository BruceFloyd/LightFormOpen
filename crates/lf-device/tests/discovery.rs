//! Discovery against a fake device on localhost, using ephemeral ports.

use std::time::Duration;

use lf_device::discovery::{Options, discover};
use serde_json::Value;
use tokio::net::UdpSocket;

#[tokio::test]
async fn finds_a_device_that_answers() {
    let fake = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let query_port = fake.local_addr().unwrap().port();
    tokio::spawn(async move {
        let mut buf = [0u8; 2048];
        // Answer every query: discovery resends, and must still list one device.
        while let Ok((len, from)) = fake.recv_from(&mut buf).await {
            let query: Value = serde_json::from_slice(&buf[..len]).unwrap();
            assert_eq!(query["api"], "0.0");
            let reply = br#"{"data":{"serialNumber":"SN1","name":"Fake","partNumber":"LF2","services":{"meta":"50050"}}}"#;
            fake.send_to(reply, from).await.unwrap();
        }
    });
    let options = Options {
        query_port,
        reply_port: 0,
        timeout: Duration::from_millis(600),
        resend_every: Duration::from_millis(150),
        hosts: vec!["127.0.0.1".parse().unwrap()],
        multicast: false,
        broadcast: false,
    };
    let found = discover(&options).await.unwrap();
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].name.as_deref(), Some("Fake"));
    assert_eq!(
        found[0].services.get("meta").map(String::as_str),
        Some("50050")
    );
}

#[tokio::test]
async fn silence_means_no_devices() {
    let quiet = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let options = Options {
        query_port: quiet.local_addr().unwrap().port(),
        reply_port: 0,
        timeout: Duration::from_millis(200),
        resend_every: Duration::from_millis(100),
        hosts: vec!["127.0.0.1".parse().unwrap()],
        multicast: false,
        broadcast: false,
    };
    assert!(discover(&options).await.unwrap().is_empty());
}
