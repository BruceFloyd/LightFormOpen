mod device;

use std::path::Path;
use std::process::ExitCode;

use lf_lit::{Container, Document, Registry};

const USAGE: &str = "\
usage:
  lf lit dump FILE                 print a .lit/.litsettings document as JSON
  lf lit roundtrip FILE...         read each file, write it back, and compare

  lf device discover [--timeout SECONDS] [--host IP]...
                                   find devices on the local network
  lf device status HOST [--seconds N] [--trace] [--no-ack]
                                   connect, subscribe and print the state the
                                   device pushes; --trace hex-dumps frames
                                   that don't decode cleanly; --no-ack skips
                                   acknowledging pushes (the ack is inferred)
  lf device play|pause|stop|next|prev HOST
  lf device slide HOST INDEX
  lf device brightness HOST VALUE  0.0 to 1.0

HOST is an IP address or name, optionally with :PORT (default: 50050, then 50051).";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let result = match args.as_slice() {
        ["lit", "dump", file] => dump(Path::new(file)),
        ["lit", "roundtrip", files @ ..] if !files.is_empty() => roundtrip(files),
        ["device", rest @ ..] => match device::run(rest) {
            Some(result) => result,
            None => {
                eprintln!("{USAGE}");
                return ExitCode::from(2);
            }
        },
        _ => {
            eprintln!("{USAGE}");
            return ExitCode::from(2);
        }
    };
    match result {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(e) => {
            eprintln!("lf: {}", device::clean(&e.to_string()));
            ExitCode::FAILURE
        }
    }
}

fn dump(path: &Path) -> Result<bool, Box<dyn std::error::Error>> {
    let doc = Document::read(&std::fs::read(path)?)?;
    let json = lf_lit::dump::to_json(&doc, Registry::builtin());
    println!("{}", serde_json::to_string_pretty(&json)?);
    Ok(true)
}

/// Binary documents must come back byte for byte. JSON documents are written
/// as binary and read again, and must come back with the same objects.
fn roundtrip(files: &[&str]) -> Result<bool, Box<dyn std::error::Error>> {
    let mut all_ok = true;
    for file in files {
        let outcome = check(Path::new(file));
        match &outcome {
            Ok(note) => println!("ok    {file}: {note}"),
            Err(e) => println!("FAIL  {file}: {e}"),
        }
        all_ok &= outcome.is_ok();
    }
    Ok(all_ok)
}

fn check(path: &Path) -> Result<String, Box<dyn std::error::Error>> {
    let bytes = std::fs::read(path)?;
    let doc = Document::read(&bytes)?;
    let undecoded = doc.undecoded_objects().count();
    if undecoded > 0 {
        return Err(format!("{undecoded} objects have values of an unknown class").into());
    }
    let written = doc.to_binary()?;
    let summary = format!(
        "{} container, data version {}, {} objects",
        match doc.container {
            Container::Binary => "binary",
            Container::Json => "JSON",
        },
        doc.header.data_version,
        doc.objects.len()
    );
    match doc.container {
        Container::Binary if written == bytes => Ok(format!("{summary}, byte-identical")),
        Container::Binary => {
            let at = written
                .iter()
                .zip(&bytes)
                .position(|(a, b)| a != b)
                .unwrap_or(written.len().min(bytes.len()));
            Err(format!("written file differs from the original at byte {at}").into())
        }
        Container::Json => {
            let reread = Document::read(&written)?;
            if reread.objects != doc.objects {
                return Err("objects changed after writing as binary and reading back".into());
            }
            if reread.to_binary()? != written {
                return Err("the binary rewrite is not stable".into());
            }
            Ok(format!("{summary}, same objects after binary rewrite"))
        }
    }
}
