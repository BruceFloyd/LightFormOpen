//! Reader and writer for Lightform Creator `.lit` project files. The same
//! container also holds `.litsettings` files.
//!
//! Two containers exist: the binary "Lightform Binary Document" Creator wrote
//! from 2022 (data version 23), and the "JSON Document" it wrote in 2020
//! (data version 9). Both hold the same object graph, so both read into
//! [`Document`]. Writing produces the binary container, and writing a binary
//! document that was read reproduces the file byte for byte.
//!
//! Not yet proven: that Creator opens a document this crate has edited.
//! Writing a JSON (data version 9) document as binary exercises the codec but
//! is not an upgrade: the result keeps data version 9, which Creator never
//! wrote as binary.
//!
//! ```no_run
//! let bytes = std::fs::read("Project.lit")?;
//! let doc = lf_lit::Document::read(&bytes)?;
//! println!("{} objects, data version {}", doc.objects.len(), doc.header.data_version);
//! assert_eq!(doc.to_binary()?, bytes);
//! # Ok::<(), lf_lit::Error>(())
//! ```
//!
//! Format notes: `docs/findings/lit-format.md`. The Python reader in
//! `tools/re/litdump.py` is the reference this crate is checked against.

mod binary;
pub mod chunk;
mod document;
pub mod dump;
mod error;
mod json;
pub mod keys;
mod registry;
pub mod time;
pub mod values;

pub use document::{Container, Document, Header, Object, Reference, State};
pub use error::{Error, Result};
pub use registry::{ClassInfo, Registry};
pub use values::Values;
