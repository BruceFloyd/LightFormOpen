//! Discovery and control of Lightform LF1/LF2 projectors over the local
//! network, using the protocol of Lightform Creator 2.2.12.
//!
//! - [`discovery`]: find devices with Creator's UDP query.
//! - [`Device`]: the MetaRPC control session (one bidirectional gRPC stream):
//!   commands, replies and state pushes.
//! - [`proto`]: every message in the recovered schema (`proto/MetaRPC.proto`).
//!
//! Protocol notes: `docs/findings/device-protocol.md`.
//!
//! The tests run against a mock device built from the recovered schema. They
//! show the client agrees with that schema, not that a real device does. That
//! is what the first live connection checks, which is why every received frame
//! keeps its raw bytes ([`Frame`]).

mod client;
pub mod discovery;
mod error;
mod frame;

pub mod proto {
    //! Generated from `proto/MetaRPC.proto`.
    #![allow(clippy::all, missing_docs)]
    tonic::include_proto!("metarpc");
}

pub use client::{API_VERSION, Command, Device, Event, Options, PORTS, Reply, StateUpdate};
pub use error::{Error, Result};
pub use frame::Frame;

#[cfg(test)]
mod tests {
    /// The build compiles `crates/lf-device/proto/MetaRPC.proto`; the copy in
    /// the repository's `proto/` folder is the published one. They must match.
    #[test]
    fn proto_matches_the_published_copy() {
        let published = concat!(env!("CARGO_MANIFEST_DIR"), "/../../proto/MetaRPC.proto");
        if let Ok(text) = std::fs::read_to_string(published) {
            assert!(
                text == include_str!("../proto/MetaRPC.proto"),
                "crates/lf-device/proto/MetaRPC.proto differs from {published}"
            );
        }
    }
}
