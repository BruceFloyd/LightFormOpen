//! The binary container ("Lightform Binary Document"). Fields are read in the
//! fixed order Creator writes them, and any field this module doesn't know is
//! an error that names the chunk path, so format surprises surface early.
//!
//! ```text
//! magic="litb" format="Lightform Binary Document" format-version max-bytes-per-line
//! header{ data-version-number original-data-version-number }
//! id-lookup{ count values=[uuid × count] }
//! objects{ count object{ [name] id class-id [owner-id] [state{…}] [references{…}] } × count }
//! ```
//!
//! Every UUID after `id-lookup` is a `u32` index into that table.

use std::collections::{HashMap, HashSet};

use uuid::Uuid;

use crate::chunk::{self, Node, kind};
use crate::document::{Container, Document, Header, Object, Reference, State};
use crate::error::{Error, Result, malformed};
use crate::keys::{self, key_hash, key_name};
use crate::registry::Registry;
use crate::values::{self, Values, encoding_for};

pub const MAGIC: &[u8; 4] = b"litb";
pub const FORMAT: &str = "Lightform Binary Document";

pub(crate) fn read(bytes: &[u8], registry: &Registry) -> Result<Document> {
    let nodes = chunk::parse(bytes)?;
    let mut top = Cursor::new(&nodes, "document");
    if top.property(keys::MAGIC)? != MAGIC {
        return malformed("not a Lightform binary document (bad magic)");
    }
    let format = string(top.property(keys::FORMAT)?, "document/format")?;
    if format != FORMAT {
        return malformed(format!("unsupported binary format {format:?}"));
    }
    let format_version = u32_of(
        top.property(keys::FORMAT_VERSION)?,
        "document/format-version",
    )?;
    let max_bytes_per_line = Some(i64_of(
        top.property(keys::MAX_BYTES_PER_LINE)?,
        "document/max-bytes-per-line",
    )?);

    let mut header = Cursor::new(top.chunk(kind::HEADER)?, "header");
    let data_version = u32_of(
        header.property(keys::DATA_VERSION)?,
        "header/data-version-number",
    )?;
    let original_data_version = u32_of(
        header.property(keys::ORIGINAL_DATA_VERSION)?,
        "header/original-data-version-number",
    )?;
    header.finish()?;

    let id_table = read_id_table(top.chunk(kind::ID_LOOKUP)?)?;
    let ids = Ids(&id_table);

    let mut list = Cursor::new(top.chunk(kind::OBJECTS)?, "objects");
    let count = u32_of(list.property(keys::COUNT)?, "objects/count")? as usize;
    // Size from the chunks present, not the stored count, which a crafted file controls.
    let mut objects = Vec::with_capacity(list.remaining());
    while let Some(body) = list.optional_chunk(kind::OBJECT) {
        objects.push(read_object(body, &ids, registry)?);
    }
    list.finish()?;
    let expected = graph_object_count(&objects);
    if count != expected {
        return malformed(format!(
            "objects/count is {count}, expected {expected} ({} objects written plus elided properties)",
            objects.len()
        ));
    }
    top.finish()?;

    Ok(Document {
        container: Container::Binary,
        header: Header {
            format_version,
            max_bytes_per_line,
            blob_text_format: None,
            data_version,
            original_data_version,
        },
        objects,
        id_table,
    })
}

/// `objects/count` is the size of Creator's in-memory object graph, not the
/// number of object chunks: it also counts the static Property objects that
/// data version 23 doesn't write. Those survive only as the `owner-id` of
/// their value controllers, so they are the owners with no object of their own.
fn graph_object_count(objects: &[Object]) -> usize {
    let written: HashSet<Uuid> = objects.iter().map(|o| o.id).collect();
    let elided: HashSet<Uuid> = objects
        .iter()
        .filter_map(|o| o.owner_id)
        .filter(|owner| !written.contains(owner))
        .collect();
    objects.len() + elided.len()
}

fn read_id_table(nodes: &[Node]) -> Result<Vec<Uuid>> {
    let mut c = Cursor::new(nodes, "id-lookup");
    let count = u32_of(c.property(keys::COUNT)?, "id-lookup/count")?;
    let values = c.array(keys::VALUES)?;
    c.finish()?;
    match Values::decode(Some(values::Encoding::Uuids), values)? {
        Values::Uuids(ids) if ids.len() == count as usize => Ok(ids),
        _ => malformed(format!(
            "id-lookup/count is {count} but the table size differs"
        )),
    }
}

fn read_object(nodes: &[Node], ids: &Ids, registry: &Registry) -> Result<Object> {
    let mut c = Cursor::new(nodes, "object");
    let name = c
        .optional_property(keys::NAME)
        .map(|v| string(v, "object/name"))
        .transpose()?;
    let id = ids.get(c.property(keys::ID)?, "object/id")?;
    let path = format!("object {id}");
    let class_id = ids.get(c.property(keys::CLASS_ID)?, &format!("{path}/class-id"))?;
    let owner_id = c
        .optional_property(keys::OWNER_ID)
        .map(|v| ids.get(v, &format!("{path}/owner-id")))
        .transpose()?;
    let class_type = registry.get(&class_id).map(|c| c.type_name.as_str());
    let state = c
        .optional_chunk(kind::STATE)
        .map(|n| read_state(n, ids, class_type, &path))
        .transpose()?;
    let references = c
        .optional_chunk(kind::REFERENCES)
        .map(|n| read_references(n, ids, &path))
        .transpose()?;
    c.path = path;
    c.finish()?;
    Ok(Object {
        id,
        class_id,
        owner_id,
        name,
        state,
        references,
    })
}

fn read_state(nodes: &[Node], ids: &Ids, class_type: Option<&str>, path: &str) -> Result<State> {
    let path = format!("{path}/state");
    let mut c = Cursor::new(nodes, &path);
    let subtype_id = c
        .optional_property(keys::SUBTYPE_ID)
        .map(|v| ids.get(v, &format!("{path}/subtype-id")))
        .transpose()?;
    let is_array = c
        .optional_property(keys::IS_ARRAY)
        .map(|v| match v {
            [0] => Ok(false),
            [1] => Ok(true),
            _ => malformed(format!("{path}/is-array is not a 0/1 byte")),
        })
        .transpose()?;
    let count = c
        .optional_property(keys::COUNT)
        .map(|v| i64_of(v, &format!("{path}/count")))
        .transpose()?;
    let shape = class_type.and_then(encoding_for);
    let values = c
        .optional_array(keys::VALUES)
        .map(|v| Values::decode(shape.map(|(e, _)| e), v))
        .transpose()
        .map_err(|e| Error::Malformed(format!("{path}/values: {e}")))?;
    values::check_count(&path, count, values.as_ref(), shape.map(|(_, w)| w))?;
    let dynamic_description = c
        .optional_chunk(kind::DYNAMIC_DESCRIPTION)
        .map(|n| read_dynamic_description(n, &path))
        .transpose()?;
    c.finish()?;
    Ok(State {
        subtype_id,
        is_array,
        count,
        values,
        dynamic_description,
    })
}

fn read_dynamic_description(nodes: &[Node], path: &str) -> Result<Vec<(String, String)>> {
    let path = format!("{path}/dynamic-description");
    nodes
        .iter()
        .map(|node| match node {
            Node::Property { key, value } => {
                let name = key_name(*key)
                    .ok_or_else(|| Error::Malformed(format!("{path}: unknown key {key:#010x}")))?;
                Ok((name.to_owned(), string(value, &format!("{path}/{name}"))?))
            }
            _ => malformed(format!("{path}: unexpected {}", describe(node))),
        })
        .collect()
}

fn read_references(nodes: &[Node], ids: &Ids, path: &str) -> Result<Vec<Reference>> {
    let path = format!("{path}/references");
    let mut c = Cursor::new(nodes, &path);
    let count = u32_of(c.property(keys::COUNT)?, &format!("{path}/count"))? as usize;
    let mut refs = Vec::with_capacity(c.remaining());
    while let Some(body) = c.optional_chunk(kind::REFERENCE) {
        let mut r = Cursor::new(body, &path);
        let local_id = u32_of(r.property(keys::LOCAL_ID)?, &format!("{path}/local-id"))?;
        let id = ids.get(r.property(keys::ID)?, &format!("{path}/id"))?;
        r.finish()?;
        refs.push(Reference { local_id, id });
    }
    c.finish()?;
    if refs.len() != count {
        return malformed(format!(
            "{path}/count is {count} but {} references follow",
            refs.len()
        ));
    }
    Ok(refs)
}

pub(crate) fn write(doc: &Document) -> Result<Vec<u8>> {
    let mut ids = Interner::new(&doc.id_table);
    let mut objects = vec![prop(
        keys::COUNT,
        (graph_object_count(&doc.objects) as u32).to_le_bytes(),
    )];
    for object in &doc.objects {
        objects.push(Node::Chunk {
            kind: kind::OBJECT,
            children: write_object(object, &mut ids)?,
        });
    }
    let table: Vec<u8> = ids.table.iter().flat_map(|u| *u.as_bytes()).collect();
    let mut table_value = (ids.table.len() as u32).to_le_bytes().to_vec();
    table_value.extend_from_slice(&table);

    let h = &doc.header;
    let nodes = vec![
        prop(keys::MAGIC, *MAGIC),
        prop(keys::FORMAT, values::write_string(FORMAT)),
        prop(keys::FORMAT_VERSION, h.format_version.to_le_bytes()),
        prop(
            keys::MAX_BYTES_PER_LINE,
            h.max_bytes_per_line.unwrap_or(0xFFFF_FFFF).to_le_bytes(),
        ),
        Node::Chunk {
            kind: kind::HEADER,
            children: vec![
                prop(keys::DATA_VERSION, h.data_version.to_le_bytes()),
                prop(
                    keys::ORIGINAL_DATA_VERSION,
                    h.original_data_version.to_le_bytes(),
                ),
            ],
        },
        Node::Chunk {
            kind: kind::ID_LOOKUP,
            children: vec![
                prop(keys::COUNT, (ids.table.len() as u32).to_le_bytes()),
                Node::Array {
                    key: keys::VALUES,
                    value: table_value,
                },
            ],
        },
        Node::Chunk {
            kind: kind::OBJECTS,
            children: objects,
        },
    ];
    Ok(chunk::write(&nodes))
}

fn write_object(o: &Object, ids: &mut Interner) -> Result<Vec<Node>> {
    let mut n = Vec::new();
    if let Some(name) = &o.name {
        n.push(prop(keys::NAME, values::write_string(name)));
    }
    n.push(prop(keys::ID, ids.index(o.id).to_le_bytes()));
    n.push(prop(keys::CLASS_ID, ids.index(o.class_id).to_le_bytes()));
    if let Some(owner) = o.owner_id {
        n.push(prop(keys::OWNER_ID, ids.index(owner).to_le_bytes()));
    }
    if let Some(state) = &o.state {
        n.push(Node::Chunk {
            kind: kind::STATE,
            children: write_state(state, ids)?,
        });
    }
    if let Some(refs) = &o.references {
        let mut children = vec![prop(keys::COUNT, (refs.len() as u32).to_le_bytes())];
        for r in refs {
            children.push(Node::Chunk {
                kind: kind::REFERENCE,
                children: vec![
                    prop(keys::LOCAL_ID, r.local_id.to_le_bytes()),
                    prop(keys::ID, ids.index(r.id).to_le_bytes()),
                ],
            });
        }
        n.push(Node::Chunk {
            kind: kind::REFERENCES,
            children,
        });
    }
    Ok(n)
}

fn write_state(s: &State, ids: &mut Interner) -> Result<Vec<Node>> {
    let mut n = Vec::new();
    if let Some(subtype) = s.subtype_id {
        n.push(prop(keys::SUBTYPE_ID, ids.index(subtype).to_le_bytes()));
    }
    if let Some(is_array) = s.is_array {
        n.push(prop(keys::IS_ARRAY, [u8::from(is_array)]));
    }
    if let Some(count) = s.count {
        n.push(prop(keys::COUNT, count.to_le_bytes()));
    }
    if let Some(values) = &s.values {
        n.push(Node::Array {
            key: keys::VALUES,
            value: values.encode()?,
        });
    }
    if let Some(desc) = &s.dynamic_description {
        let children = desc
            .iter()
            .map(|(k, v)| prop(key_hash(k), values::write_string(v)))
            .collect();
        n.push(Node::Chunk {
            kind: kind::DYNAMIC_DESCRIPTION,
            children,
        });
    }
    Ok(n)
}

fn prop(key: u32, value: impl AsRef<[u8]>) -> Node {
    Node::Property {
        key,
        value: value.as_ref().to_vec(),
    }
}

/// UUID interning for the writer, seeded with the table as read.
struct Interner {
    table: Vec<Uuid>,
    index: HashMap<Uuid, u32>,
}

impl Interner {
    fn new(seed: &[Uuid]) -> Interner {
        let mut interner = Interner {
            table: Vec::new(),
            index: HashMap::new(),
        };
        for id in seed {
            interner.index(*id);
        }
        interner
    }

    fn index(&mut self, id: Uuid) -> u32 {
        *self.index.entry(id).or_insert_with(|| {
            self.table.push(id);
            self.table.len() as u32 - 1
        })
    }
}

struct Ids<'a>(&'a [Uuid]);

impl Ids<'_> {
    fn get(&self, value: &[u8], path: &str) -> Result<Uuid> {
        let index = u32_of(value, path)? as usize;
        self.0.get(index).copied().ok_or_else(|| {
            Error::Malformed(format!(
                "{path}: id index {index} is outside the id-lookup table"
            ))
        })
    }
}

/// Reads a chunk's children in order, so fields must appear in the order asked.
struct Cursor<'a> {
    nodes: &'a [Node],
    pos: usize,
    path: String,
}

impl<'a> Cursor<'a> {
    fn new(nodes: &'a [Node], path: &str) -> Cursor<'a> {
        Cursor {
            nodes,
            pos: 0,
            path: path.to_owned(),
        }
    }

    fn property(&mut self, key: u32) -> Result<&'a [u8]> {
        self.optional_property(key).ok_or_else(|| self.missing(key))
    }

    fn optional_property(&mut self, key: u32) -> Option<&'a [u8]> {
        match self.nodes.get(self.pos) {
            Some(Node::Property { key: k, value }) if *k == key => {
                self.pos += 1;
                Some(value)
            }
            _ => None,
        }
    }

    fn array(&mut self, key: u32) -> Result<&'a [u8]> {
        self.optional_array(key).ok_or_else(|| self.missing(key))
    }

    fn optional_array(&mut self, key: u32) -> Option<&'a [u8]> {
        match self.nodes.get(self.pos) {
            Some(Node::Array { key: k, value }) if *k == key => {
                self.pos += 1;
                Some(value)
            }
            _ => None,
        }
    }

    fn chunk(&mut self, kind: Uuid) -> Result<&'a [Node]> {
        self.optional_chunk(kind).ok_or_else(|| {
            let name = kind::name(&kind).unwrap_or("chunk");
            Error::Malformed(format!(
                "{}: expected {name} here, found {}",
                self.path,
                self.here()
            ))
        })
    }

    fn optional_chunk(&mut self, kind: Uuid) -> Option<&'a [Node]> {
        match self.nodes.get(self.pos) {
            Some(Node::Chunk { kind: k, children }) if *k == kind => {
                self.pos += 1;
                Some(children)
            }
            _ => None,
        }
    }

    fn remaining(&self) -> usize {
        self.nodes.len() - self.pos
    }

    fn finish(&self) -> Result<()> {
        if self.pos == self.nodes.len() {
            Ok(())
        } else {
            malformed(format!("{}: unexpected {}", self.path, self.here()))
        }
    }

    fn missing(&self, key: u32) -> Error {
        let name = key_name(key).unwrap_or("property");
        Error::Malformed(format!(
            "{}: expected {name} here, found {}",
            self.path,
            self.here()
        ))
    }

    fn here(&self) -> String {
        self.nodes
            .get(self.pos)
            .map_or_else(|| "the end of the chunk".into(), describe)
    }
}

fn describe(node: &Node) -> String {
    match node {
        Node::Chunk { kind, .. } => match kind::name(kind) {
            Some(name) => format!("{name} chunk"),
            None => format!("chunk of unknown kind {kind}"),
        },
        Node::Property { key, .. } | Node::Array { key, .. } => match key_name(*key) {
            Some(name) => format!("{name} field"),
            None => format!("field with unknown key {key:#010x}"),
        },
    }
}

fn string(value: &[u8], path: &str) -> Result<String> {
    match values::read_string(value) {
        Ok((s, [])) => Ok(s),
        Ok(_) => malformed(format!("{path}: bytes left after the string")),
        Err(e) => malformed(format!("{path}: {e}")),
    }
}

fn u32_of(value: &[u8], path: &str) -> Result<u32> {
    value
        .try_into()
        .map(u32::from_le_bytes)
        .map_err(|_| Error::Malformed(format!("{path}: expected 4 bytes, found {}", value.len())))
}

fn i64_of(value: &[u8], path: &str) -> Result<i64> {
    value
        .try_into()
        .map(i64::from_le_bytes)
        .map_err(|_| Error::Malformed(format!("{path}: expected 8 bytes, found {}", value.len())))
}
