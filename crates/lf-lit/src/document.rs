//! The document model shared by both containers: a flat list of objects, each
//! with an id, a class, an owner, an optional own value (`state`) and numbered
//! slots (`references`).

use uuid::Uuid;

use crate::error::Result;
use crate::registry::Registry;
use crate::values::Values;
use crate::{binary, json};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Container {
    /// "Lightform Binary Document", written by Creator from 2022 (data version 23).
    Binary,
    /// "JSON Document", written by Creator in 2020 (data version 9).
    Json,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Header {
    pub format_version: u32,
    /// Binary container only; Creator writes `0xFFFFFFFF`.
    pub max_bytes_per_line: Option<i64>,
    /// JSON container only; Creator writes `"byte-array"`.
    pub blob_text_format: Option<String>,
    /// Schema version of the object graph: 9 in 2020 projects, 23 in 2022 projects.
    pub data_version: u32,
    /// The schema version the document was first created at.
    pub original_data_version: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Object {
    pub id: Uuid,
    pub class_id: Uuid,
    /// The parent object. `None` only on roots.
    pub owner_id: Option<Uuid>,
    pub name: Option<String>,
    pub state: Option<State>,
    pub references: Option<Vec<Reference>>,
}

/// One filled slot: `references[local_id]` points at `id`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Reference {
    pub local_id: u32,
    pub id: Uuid,
}

/// An object's own value. Value controllers hold their data here.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct State {
    /// The enum description, for enum controllers.
    pub subtype_id: Option<Uuid>,
    pub is_array: Option<bool>,
    /// Element count of an array-valued slot (components ÷ element width).
    pub count: Option<i64>,
    pub values: Option<Values>,
    /// Name and description of a dynamic property, such as a custom shader
    /// input. Keys are kebab-case (`value-type`, `default-value`, ...); every
    /// value is a string.
    pub dynamic_description: Option<Vec<(String, String)>>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Document {
    pub container: Container,
    pub header: Header,
    pub objects: Vec<Object>,
    /// The binary container's interned UUID table, in file order. Kept so
    /// writing a read document reproduces it; new UUIDs are appended.
    pub(crate) id_table: Vec<Uuid>,
}

impl Document {
    /// A new binary-container document. UUIDs are interned in writing order.
    pub fn new(header: Header, objects: Vec<Object>) -> Document {
        Document {
            container: Container::Binary,
            header,
            objects,
            id_table: Vec::new(),
        }
    }

    /// Read either container, decoding values with the built-in class registry.
    pub fn read(bytes: &[u8]) -> Result<Document> {
        Document::read_with(bytes, Registry::builtin())
    }

    pub fn read_with(bytes: &[u8], registry: &Registry) -> Result<Document> {
        match bytes.iter().find(|b| !b.is_ascii_whitespace()) {
            Some(b'{') => json::read(bytes, registry),
            _ => binary::read(bytes, registry),
        }
    }

    /// Write the binary container.
    pub fn to_binary(&self) -> Result<Vec<u8>> {
        binary::write(self)
    }

    /// The binary container's UUID table as read. Empty for JSON documents.
    pub fn id_table(&self) -> &[Uuid] {
        &self.id_table
    }

    pub fn object(&self, id: &Uuid) -> Option<&Object> {
        self.objects.iter().find(|o| o.id == *id)
    }

    /// Objects whose values couldn't be decoded because their class isn't known.
    pub fn undecoded_objects(&self) -> impl Iterator<Item = &Object> {
        self.objects.iter().filter(|o| {
            o.state
                .as_ref()
                .and_then(|s| s.values.as_ref())
                .is_some_and(Values::is_undecoded)
        })
    }
}
