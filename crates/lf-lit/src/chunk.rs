//! The binary container's chunk stream, without interpretation. Parsing and
//! writing a stream with this module reproduces it byte for byte.
//!
//! | Tag | Layout | Meaning |
//! |---|---|---|
//! | `01` | `u32 len=16`, `uuid[16]` | begin a nested chunk of the given kind |
//! | `02` | | end the innermost nested chunk |
//! | `03` | | end of document |
//! | `04` | `u32 len`, `u32 key`, `value[len-4]` | single-valued property |
//! | `05` | `u32 len`, `u32 key`, `u32 n`, `elements` | array property |
//!
//! Integers are little-endian. UUIDs are in RFC 4122 byte order.

use uuid::Uuid;

use crate::error::{Error, Result};

const TAG_BEGIN: u8 = 0x01;
const TAG_END: u8 = 0x02;
const TAG_DOCUMENT_END: u8 = 0x03;
const TAG_PROPERTY: u8 = 0x04;
const TAG_ARRAY: u8 = 0x05;

/// Chunk kinds. The UUID names what a chunk is, not which instance it is.
pub mod kind {
    use uuid::Uuid;

    pub const HEADER: Uuid = Uuid::from_u128(0x8e73ed04_344f_45f0_8821_85fa77dcb9b5);
    pub const ID_LOOKUP: Uuid = Uuid::from_u128(0x4505e8fb_03b9_45ae_bb0d_94c83bc2944d);
    pub const OBJECTS: Uuid = Uuid::from_u128(0x54fa5fd0_d3cb_472f_b660_3bb67f89eef4);
    pub const OBJECT: Uuid = Uuid::from_u128(0x787ea4ea_18e1_4e76_97d4_2795d6fe8f50);
    pub const STATE: Uuid = Uuid::from_u128(0x0c5a8d0c_d533_47b7_bc25_4e21443714e1);
    pub const REFERENCES: Uuid = Uuid::from_u128(0x95623259_f9de_4a79_922b_e4eef38436dd);
    pub const REFERENCE: Uuid = Uuid::from_u128(0x607eb63c_c47e_4a87_97b3_135429656984);
    pub const DYNAMIC_DESCRIPTION: Uuid = Uuid::from_u128(0x6ce5a4ab_d5c1_4d94_ac26_0fe82546a1cf);
    pub const PROPERTIES: Uuid = Uuid::from_u128(0xcb30171e_c4bc_4682_901d_1e2cbc802498);
    pub const PROPERTY: Uuid = Uuid::from_u128(0x15a78dd1_1e93_44aa_af02_f02085d75711);

    /// The chunk's name as the JSON container spells it, if known.
    pub fn name(kind: &Uuid) -> Option<&'static str> {
        Some(match *kind {
            HEADER => "header",
            ID_LOOKUP => "id-lookup",
            OBJECTS => "objects",
            OBJECT => "object",
            STATE => "state",
            REFERENCES => "references",
            REFERENCE => "reference",
            DYNAMIC_DESCRIPTION => "dynamic-description",
            PROPERTIES => "properties",
            PROPERTY => "property",
            _ => return None,
        })
    }
}

/// One element of a chunk stream.
#[derive(Debug, Clone, PartialEq)]
pub enum Node {
    Chunk {
        kind: Uuid,
        children: Vec<Node>,
    },
    /// Tag `04`. `value` excludes the key.
    Property {
        key: u32,
        value: Vec<u8>,
    },
    /// Tag `05`. `value` excludes the key and starts with the `u32` element count.
    Array {
        key: u32,
        value: Vec<u8>,
    },
}

/// Parse a whole document. The stream must end with the document-end tag and
/// nothing after it.
pub fn parse(bytes: &[u8]) -> Result<Vec<Node>> {
    let mut reader = Reader { bytes, pos: 0 };
    let nodes = reader.nodes(0)?;
    if reader.pos != bytes.len() {
        return Err(Error::Malformed(format!(
            "{} bytes after the document end",
            bytes.len() - reader.pos
        )));
    }
    Ok(nodes)
}

/// Write a whole document. Nested chunks end with `02`; the stream ends with `03`.
pub fn write(nodes: &[Node]) -> Vec<u8> {
    let mut out = Vec::new();
    write_nodes(nodes, &mut out);
    out.push(TAG_DOCUMENT_END);
    out
}

fn write_nodes(nodes: &[Node], out: &mut Vec<u8>) {
    for node in nodes {
        match node {
            Node::Chunk { kind, children } => {
                out.push(TAG_BEGIN);
                out.extend_from_slice(&16u32.to_le_bytes());
                out.extend_from_slice(kind.as_bytes());
                write_nodes(children, out);
                out.push(TAG_END);
            }
            Node::Property { key, value } => write_keyed(TAG_PROPERTY, *key, value, out),
            Node::Array { key, value } => write_keyed(TAG_ARRAY, *key, value, out),
        }
    }
}

fn write_keyed(tag: u8, key: u32, value: &[u8], out: &mut Vec<u8>) {
    out.push(tag);
    out.extend_from_slice(&(value.len() as u32 + 4).to_le_bytes());
    out.extend_from_slice(&key.to_le_bytes());
    out.extend_from_slice(value);
}

/// Real documents nest about 6 levels deep. The limit keeps a crafted file
/// from exhausting the stack.
const MAX_DEPTH: usize = 64;

struct Reader<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl Reader<'_> {
    fn nodes(&mut self, depth: usize) -> Result<Vec<Node>> {
        let mut nodes = Vec::new();
        loop {
            let offset = self.pos;
            let tag = *self.bytes.get(offset).ok_or(Error::UnexpectedEof(offset))?;
            self.pos += 1;
            match tag {
                TAG_END if depth > 0 => return Ok(nodes),
                TAG_DOCUMENT_END if depth == 0 => return Ok(nodes),
                TAG_END | TAG_DOCUMENT_END => {
                    return Err(Error::Malformed(format!(
                        "chunk end tag {tag:#04x} at depth {depth}, byte {offset}"
                    )));
                }
                TAG_BEGIN => {
                    let body = self.sized()?;
                    let kind = Uuid::from_slice(body).map_err(|_| {
                        Error::Malformed(format!("chunk kind at byte {offset} is not 16 bytes"))
                    })?;
                    if depth >= MAX_DEPTH {
                        return Err(Error::Malformed(format!(
                            "chunks nest deeper than {MAX_DEPTH} levels at byte {offset}"
                        )));
                    }
                    let children = self.nodes(depth + 1)?;
                    nodes.push(Node::Chunk { kind, children });
                }
                TAG_PROPERTY | TAG_ARRAY => {
                    let body = self.sized()?;
                    if body.len() < 4 {
                        return Err(Error::Malformed(format!(
                            "property at byte {offset} is shorter than its key"
                        )));
                    }
                    let key = u32::from_le_bytes(body[..4].try_into().unwrap());
                    let value = body[4..].to_vec();
                    nodes.push(if tag == TAG_PROPERTY {
                        Node::Property { key, value }
                    } else {
                        Node::Array { key, value }
                    });
                }
                _ => return Err(Error::UnknownTag { tag, offset }),
            }
        }
    }

    fn sized(&mut self) -> Result<&[u8]> {
        let len = self.take(4)?;
        let len = u32::from_le_bytes(len.try_into().unwrap()) as usize;
        self.take(len)
    }

    fn take(&mut self, n: usize) -> Result<&[u8]> {
        let end = self.pos.checked_add(n).filter(|&e| e <= self.bytes.len());
        let end = end.ok_or(Error::UnexpectedEof(self.bytes.len()))?;
        let slice = &self.bytes[self.pos..end];
        self.pos = end;
        Ok(slice)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_nested_chunks() {
        let nodes = vec![
            Node::Property {
                key: 7,
                value: b"litb".to_vec(),
            },
            Node::Chunk {
                kind: kind::HEADER,
                children: vec![
                    Node::Array {
                        key: 9,
                        value: vec![2, 0, 0, 0, 1, 2],
                    },
                    Node::Chunk {
                        kind: kind::STATE,
                        children: vec![],
                    },
                ],
            },
        ];
        let bytes = write(&nodes);
        assert_eq!(*bytes.last().unwrap(), TAG_DOCUMENT_END);
        assert_eq!(parse(&bytes).unwrap(), nodes);
    }

    #[test]
    fn rejects_truncated_stream() {
        let bytes = write(&[Node::Property {
            key: 1,
            value: vec![0; 8],
        }]);
        assert!(parse(&bytes[..bytes.len() - 3]).is_err());
    }

    #[test]
    fn rejects_unknown_tag() {
        assert!(matches!(
            parse(&[0x09]),
            Err(Error::UnknownTag {
                tag: 0x09,
                offset: 0
            })
        ));
    }

    #[test]
    fn rejects_excessive_nesting() {
        let mut bytes = Vec::new();
        for _ in 0..=MAX_DEPTH {
            bytes.push(TAG_BEGIN);
            bytes.extend_from_slice(&16u32.to_le_bytes());
            bytes.extend_from_slice(kind::STATE.as_bytes());
        }
        let err = parse(&bytes).unwrap_err();
        assert!(err.to_string().contains("nest deeper"), "{err}");
    }

    #[test]
    fn rejects_unbalanced_end() {
        assert!(parse(&[TAG_END]).is_err());
    }
}
