//! `lf device ...`: discovery and the safe playback commands. Commands that
//! change device configuration (reboot, reset, Wi-Fi, firmware, scan) are
//! deliberately not exposed.

use std::net::IpAddr;
use std::time::Duration;

use lf_device::{Device, Event, Options, discovery};
use tokio::sync::broadcast::error::RecvError;

type CliResult = Result<bool, Box<dyn std::error::Error>>;

/// `None` means the arguments didn't match a command.
pub fn run(args: &[&str]) -> Option<CliResult> {
    let runtime = match tokio::runtime::Runtime::new() {
        Ok(runtime) => runtime,
        Err(e) => return Some(Err(e.into())),
    };
    let result = match args {
        ["discover", flags @ ..] => runtime.block_on(discover(flags)),
        ["status", host, flags @ ..] => runtime.block_on(status(host, flags)),
        ["slide", host, index] => match index.parse() {
            Ok(index) => runtime.block_on(command(host, |d| {
                Box::pin(async move { d.set_slide_index(index).await })
            })),
            Err(_) => return None,
        },
        ["brightness", host, value] => match value.parse::<f32>() {
            Ok(v) if (0.0..=1.0).contains(&v) => runtime.block_on(command(host, |d| {
                Box::pin(async move { d.set_brightness(v).await })
            })),
            _ => return None,
        },
        [verb, host] => {
            let action: fn(&Device) -> Pending<'_> = match *verb {
                "play" => |d| Box::pin(d.play()),
                "pause" => |d| Box::pin(d.pause()),
                "stop" => |d| Box::pin(d.stop()),
                "next" => |d| Box::pin(d.next_slide()),
                "prev" => |d| Box::pin(d.prev_slide()),
                _ => return None,
            };
            runtime.block_on(command(host, action))
        }
        _ => return None,
    };
    Some(result)
}

type Pending<'a> = std::pin::Pin<Box<dyn Future<Output = lf_device::Result<()>> + 'a>>;

async fn discover(flags: &[&str]) -> CliResult {
    let mut options = discovery::Options::default();
    let mut rest = flags.iter();
    while let Some(flag) = rest.next() {
        match (*flag, rest.next()) {
            ("--timeout", Some(v)) => options.timeout = Duration::from_secs_f64(v.parse()?),
            ("--host", Some(v)) => options.hosts.push(v.parse::<IpAddr>()?),
            _ => return Err(format!("unknown option {flag}").into()),
        }
    }
    let found = discovery::discover(&options).await?;
    if found.is_empty() {
        println!(
            "no devices answered within {:.1} s",
            options.timeout.as_secs_f64()
        );
        return Ok(false);
    }
    // Any host on the network can answer, so its strings are escaped.
    for device in &found {
        let field = |v: &Option<String>| v.as_deref().map_or_else(|| "?".into(), clean);
        println!(
            "{}  name={} serial={} part={} version={}",
            device.address.ip(),
            field(&device.name),
            field(&device.serial_number),
            field(&device.part_number),
            field(&device.version),
        );
        println!("    services: {:?}", device.services);
        println!("    raw reply: {}", clean(&device.raw.to_string()));
    }
    Ok(true)
}

/// Subscribe and print what the device pushes. Built to be useful at the first
/// live connection: a first reply that doesn't decode is reported with its
/// bytes instead of ending the session.
async fn status(host: &str, flags: &[&str]) -> CliResult {
    let mut seconds = 10.0;
    let mut trace = false;
    let mut options = Options {
        require_test_connection: false,
        ..Options::default()
    };
    let mut rest = flags.iter();
    while let Some(flag) = rest.next() {
        match *flag {
            "--trace" => trace = true,
            "--no-ack" => options.ack_state_pushes = false,
            "--seconds" => seconds = rest.next().ok_or("--seconds needs a value")?.parse()?,
            _ => return Err(format!("unknown option {flag}").into()),
        }
    }
    let device = connect(host, options).await?;
    println!("connected to {}", device.target());
    println!(
        "device api-version: {}",
        device
            .server_api_version()
            .map_or_else(|| "(not sent)".into(), clean)
    );
    if let Some(e) = device.first_contact_error() {
        report("TestConnection", e);
    }
    let mut events = device.events();
    if let Err(e) = device.subscribe_all().await {
        report("Subscribe", &e);
    }
    let deadline = tokio::time::Instant::now() + Duration::from_secs_f64(seconds);
    while let Ok(received) = tokio::time::timeout_at(deadline, events.recv()).await {
        let event = match received {
            Ok(event) => event,
            Err(RecvError::Lagged(n)) => {
                println!("(dropped {n} events: the device sent faster than they printed)");
                continue;
            }
            Err(RecvError::Closed) => break,
        };
        match event {
            Event::State { update, frame } => {
                println!("{update:?}");
                if trace && !frame.is_lossless() {
                    println!(
                        "    frame has fields the schema doesn't know: {}",
                        frame.hex()
                    );
                }
            }
            Event::Unrecognized(frame) => {
                println!("unrecognized frame ({} bytes)", frame.raw.len());
                if trace {
                    println!("    {}", frame.hex());
                }
            }
            Event::Closed(reason) => {
                let reason = reason
                    .map(|r| format!(": {}", clean(&r)))
                    .unwrap_or_default();
                println!("device closed the connection{reason}");
                return Ok(false);
            }
        }
    }
    Ok(true)
}

/// Print a failed request, with the raw reply when there was one.
fn report(request: &str, error: &lf_device::Error) {
    println!("{request} failed: {}", clean(&error.to_string()));
    if let lf_device::Error::UnexpectedReply { raw, .. } = error {
        let hex: String = raw.iter().map(|b| format!("{b:02x}")).collect();
        println!("    reply bytes: {hex}");
    }
}

async fn command(host: &str, action: impl for<'a> Fn(&'a Device) -> Pending<'a>) -> CliResult {
    let device = connect(host, Options::default()).await?;
    action(&device).await?;
    println!("ok");
    Ok(true)
}

async fn connect(host: &str, mut options: Options) -> lf_device::Result<Device> {
    let (name, port) = match host.rsplit_once(':') {
        Some((name, port)) if !name.contains(':') => (name, port.parse().ok()),
        _ => (host, None),
    };
    if let Some(port) = port {
        options.ports = vec![port];
    }
    Device::connect(name, &options).await
}

/// Escape control characters in text that came from the network, so it can't
/// drive the terminal.
pub(crate) fn clean(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        if c.is_control() {
            out.extend(c.escape_default());
        } else {
            out.push(c);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::clean;

    #[test]
    fn escapes_control_characters() {
        assert_eq!(clean("Studio\u{1b}[2J"), "Studio\\u{1b}[2J");
        assert_eq!(clean("café LF2"), "café LF2");
        // C1 controls such as CSI survive JSON serialization, so they need escaping too.
        assert_eq!(clean("a\u{9b}2J"), "a\\u{9b}2J");
    }
}
