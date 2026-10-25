//! The JSON container ("JSON Document"), as Creator wrote it in 2020.
//!
//! It is not strict JSON: the writer leaves trailing commas. Keys are
//! camelCase, a nested chunk is written as `"<name>|<kind-uuid>": {…}`, and
//! UUIDs are strings with no id table.
//!
//! ```text
//! {"format":"JSON Document","formatVersion":1,"blobTextFormat":"byte-array",
//!  "header|…":{"dataVersionNumber":9,"originalDataVersionNumber":9},
//!  "objects|…":{"count":N,"0":{"name","id","classId","ownerId","state|…","references|…"},…}}
//! ```

use serde_json::{Map, Value};
use uuid::Uuid;

use crate::chunk::kind;
use crate::document::{Container, Document, Header, Object, Reference, State};
use crate::error::{Error, Result, malformed};
use crate::registry::Registry;
use crate::time::iso_to_filetime;
use crate::values::{self, Encoding, Values, encoding_for};

pub const FORMAT: &str = "JSON Document";

pub(crate) fn read(bytes: &[u8], registry: &Registry) -> Result<Document> {
    let text = std::str::from_utf8(bytes)
        .map_err(|e| Error::Malformed(format!("JSON document is not valid UTF-8: {e}")))?;
    let root: Value = serde_json::from_str(&strip_trailing_commas(text))?;
    let root = object(&root, "document")?;

    let mut format = None;
    let mut format_version = None;
    let mut blob_text_format = None;
    let mut versions = None;
    let mut objects = None;
    for (key, value) in root {
        match chunk_key(key, "document")? {
            ("format", None) => format = Some(str_of(value, "document/format")?),
            ("formatVersion", None) => {
                format_version = Some(u32_of(value, "document/formatVersion")?)
            }
            ("blobTextFormat", None) => {
                blob_text_format = Some(str_of(value, "document/blobTextFormat")?.to_owned())
            }
            ("header", Some(k)) if k == kind::HEADER => versions = Some(read_header(value)?),
            ("objects", Some(k)) if k == kind::OBJECTS => {
                objects = Some(read_objects(value, registry)?)
            }
            _ => return malformed(format!("document: unexpected key {key:?}")),
        }
    }
    if format != Some(FORMAT) {
        return malformed(format!("unsupported JSON format {format:?}"));
    }
    let (data_version, original_data_version) =
        versions.ok_or_else(|| Error::Malformed("document: no header".into()))?;
    Ok(Document {
        container: Container::Json,
        header: Header {
            format_version: format_version
                .ok_or_else(|| Error::Malformed("document: no formatVersion".into()))?,
            max_bytes_per_line: None,
            blob_text_format,
            data_version,
            original_data_version,
        },
        objects: objects.ok_or_else(|| Error::Malformed("document: no objects".into()))?,
        id_table: Vec::new(),
    })
}

fn read_header(value: &Value) -> Result<(u32, u32)> {
    let header = object(value, "header")?;
    let mut data = None;
    let mut original = None;
    for (key, value) in header {
        match key.as_str() {
            "dataVersionNumber" => data = Some(u32_of(value, "header/dataVersionNumber")?),
            "originalDataVersionNumber" => {
                original = Some(u32_of(value, "header/originalDataVersionNumber")?)
            }
            _ => return malformed(format!("header: unexpected key {key:?}")),
        }
    }
    match (data, original) {
        (Some(d), Some(o)) => Ok((d, o)),
        _ => malformed("header: missing a data version number"),
    }
}

fn read_objects(value: &Value, registry: &Registry) -> Result<Vec<Object>> {
    let list = object(value, "objects")?;
    let mut count = None;
    let mut objects = Vec::new();
    for (key, value) in list {
        if key == "count" {
            count = Some(u32_of(value, "objects/count")? as usize);
        } else if key.parse::<usize>().is_ok() {
            objects.push(read_object(value, registry, &format!("objects/{key}"))?);
        } else {
            return malformed(format!("objects: unexpected key {key:?}"));
        }
    }
    if count != Some(objects.len()) {
        return malformed(format!(
            "objects/count is {count:?} but {} objects follow",
            objects.len()
        ));
    }
    Ok(objects)
}

fn read_object(value: &Value, registry: &Registry, path: &str) -> Result<Object> {
    let fields = object(value, path)?;
    let id = uuid_of(fields.get("id"), &format!("{path}/id"))?;
    let path = format!("object {id}");
    let class_id = uuid_of(fields.get("classId"), &format!("{path}/classId"))?;
    let mut object = Object {
        id,
        class_id,
        owner_id: None,
        name: None,
        state: None,
        references: None,
    };
    for (key, value) in fields {
        match chunk_key(key, &path)? {
            ("id" | "classId", None) => {}
            ("name", None) => {
                object.name = match value {
                    Value::Null => None,
                    _ => Some(str_of(value, &format!("{path}/name"))?.to_owned()),
                }
            }
            ("ownerId", None) => {
                object.owner_id = Some(uuid_of(Some(value), &format!("{path}/ownerId"))?)
            }
            ("state", Some(k)) if k == kind::STATE => {
                let class_type = registry.get(&class_id).map(|c| c.type_name.as_str());
                object.state = Some(read_state(value, class_type, &path)?);
            }
            ("references", Some(k)) if k == kind::REFERENCES => {
                object.references = Some(read_references(value, &path)?)
            }
            _ => return malformed(format!("{path}: unexpected key {key:?}")),
        }
    }
    Ok(object)
}

fn read_state(value: &Value, class_type: Option<&str>, path: &str) -> Result<State> {
    let path = format!("{path}/state");
    let shape = class_type.and_then(encoding_for);
    let mut state = State::default();
    for (key, value) in object(value, &path)? {
        match chunk_key(key, &path)? {
            ("isArray", None) => {
                state.is_array = Some(
                    value
                        .as_bool()
                        .ok_or_else(|| expected(&path, key, "a boolean"))?,
                )
            }
            ("count", None) => {
                state.count = Some(
                    value
                        .as_i64()
                        .ok_or_else(|| expected(&path, key, "an integer"))?,
                )
            }
            ("subtypeId", None) => {
                state.subtype_id = Some(uuid_of(Some(value), &format!("{path}/subtypeId"))?)
            }
            ("values", None) => {
                let items = value
                    .as_array()
                    .ok_or_else(|| expected(&path, key, "an array"))?;
                state.values = Some(read_values(items, shape.map(|(e, _)| e), &path)?);
            }
            ("dynamicDescription", Some(k)) if k == kind::DYNAMIC_DESCRIPTION => {
                let mut desc = Vec::new();
                for (name, v) in object(value, &format!("{path}/dynamicDescription"))? {
                    let text = str_of(v, &format!("{path}/dynamicDescription/{name}"))?;
                    desc.push((kebab_case(name), text.to_owned()));
                }
                state.dynamic_description = Some(desc);
            }
            _ => return malformed(format!("{path}: unexpected key {key:?}")),
        }
    }
    values::check_count(
        &path,
        state.count,
        state.values.as_ref(),
        shape.map(|(_, w)| w),
    )?;
    Ok(state)
}

fn read_values(items: &[Value], encoding: Option<Encoding>, path: &str) -> Result<Values> {
    let path = format!("{path}/values");
    fn map<T>(items: &[Value], path: &str, f: impl Fn(&Value) -> Option<T>) -> Result<Vec<T>> {
        items
            .iter()
            .enumerate()
            .map(|(i, v)| {
                f(v).ok_or_else(|| Error::Malformed(format!("{path}[{i}]: unexpected value {v}")))
            })
            .collect()
    }
    let Some(encoding) = encoding else {
        return Ok(Values::Json(items.to_vec()));
    };
    Ok(match encoding {
        Encoding::F32 => Values::F32(map(items, &path, |v| v.as_f64().map(|x| x as f32))?),
        Encoding::F64 => Values::F64(map(items, &path, Value::as_f64)?),
        Encoding::I32 => Values::I32(map(items, &path, |v| {
            v.as_i64().and_then(|x| x.try_into().ok())
        })?),
        Encoding::I64 => Values::I64(map(items, &path, Value::as_i64)?),
        Encoding::U8 => Values::U8(map(items, &path, |v| {
            v.as_u64().and_then(|x| x.try_into().ok())
        })?),
        Encoding::Bool => Values::Bool(map(items, &path, |v| v.as_bool().map(u8::from))?),
        Encoding::Ticks => Values::Ticks(map(items, &path, ticks)?),
        Encoding::FileTime => {
            Values::FileTime(map(items, &path, |v| iso_to_filetime(v.as_str()?))?)
        }
        Encoding::Strings => Values::Strings(map(items, &path, |v| v.as_str().map(str::to_owned))?),
        Encoding::Uuids => Values::Uuids(map(items, &path, |v| Uuid::parse_str(v.as_str()?).ok())?),
    })
}

/// Timeline values are written as `"<ticks>flk"`.
fn ticks(value: &Value) -> Option<i64> {
    match value {
        Value::String(s) => s.strip_suffix("flk")?.parse().ok(),
        _ => value.as_i64(),
    }
}

fn read_references(value: &Value, path: &str) -> Result<Vec<Reference>> {
    let path = format!("{path}/references");
    let mut count = None;
    let mut refs = Vec::new();
    for (key, value) in object(value, &path)? {
        if key == "count" {
            count = Some(u32_of(value, &format!("{path}/count"))? as usize);
            continue;
        }
        let item = format!("{path}/{key}");
        let fields = object(value, &item)?;
        if fields.len() != 2 {
            return malformed(format!("{item}: expected localId and id"));
        }
        let local_id = u32_of(
            fields.get("localId").unwrap_or(&Value::Null),
            &format!("{item}/localId"),
        )?;
        let id = uuid_of(fields.get("id"), &format!("{item}/id"))?;
        refs.push(Reference { local_id, id });
    }
    if count != Some(refs.len()) {
        return malformed(format!(
            "{path}/count is {count:?} but {} references follow",
            refs.len()
        ));
    }
    Ok(refs)
}

/// Remove the commas Creator leaves before `}` and `]`, leaving strings alone.
fn strip_trailing_commas(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let (mut in_string, mut escaped) = (false, false);
    for (i, c) in text.char_indices() {
        if in_string {
            match c {
                _ if escaped => escaped = false,
                '\\' => escaped = true,
                '"' => in_string = false,
                _ => {}
            }
        } else if c == '"' {
            in_string = true;
        } else if c == ',' {
            let next = text[i + 1..].trim_start().chars().next();
            if matches!(next, Some('}' | ']')) {
                continue;
            }
        }
        out.push(c);
    }
    out
}

/// Split `"name|kind-uuid"` into its parts.
fn chunk_key<'a>(key: &'a str, path: &str) -> Result<(&'a str, Option<Uuid>)> {
    match key.split_once('|') {
        None => Ok((key, None)),
        Some((name, kind)) => {
            let kind = Uuid::parse_str(kind).map_err(|_| {
                Error::Malformed(format!("{path}: chunk key {key:?} has no kind UUID"))
            })?;
            Ok((name, Some(kind)))
        }
    }
}

fn kebab_case(camel: &str) -> String {
    let mut out = String::with_capacity(camel.len() + 4);
    for c in camel.chars() {
        if c.is_ascii_uppercase() {
            out.push('-');
            out.push(c.to_ascii_lowercase());
        } else {
            out.push(c);
        }
    }
    out
}

fn object<'a>(value: &'a Value, path: &str) -> Result<&'a Map<String, Value>> {
    value
        .as_object()
        .ok_or_else(|| Error::Malformed(format!("{path}: expected an object")))
}

fn str_of<'a>(value: &'a Value, path: &str) -> Result<&'a str> {
    value
        .as_str()
        .ok_or_else(|| Error::Malformed(format!("{path}: expected a string")))
}

fn u32_of(value: &Value, path: &str) -> Result<u32> {
    value
        .as_u64()
        .and_then(|v| v.try_into().ok())
        .ok_or_else(|| Error::Malformed(format!("{path}: expected an unsigned integer")))
}

fn uuid_of(value: Option<&Value>, path: &str) -> Result<Uuid> {
    value
        .and_then(Value::as_str)
        .and_then(|s| Uuid::parse_str(s).ok())
        .ok_or_else(|| Error::Malformed(format!("{path}: expected a UUID string")))
}

fn expected(path: &str, key: &str, what: &str) -> Error {
    Error::Malformed(format!("{path}/{key}: expected {what}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_trailing_commas_outside_strings() {
        assert_eq!(
            strip_trailing_commas(r#"{"a":[1,2,],"b":"x,}",}"#),
            r#"{"a":[1,2],"b":"x,}"}"#
        );
        assert_eq!(strip_trailing_commas(r#"{"a":"\",]",}"#), r#"{"a":"\",]"}"#);
        assert_eq!(strip_trailing_commas("[1 ,\n ]"), "[1 \n ]");
    }

    #[test]
    fn kebab_cases_dynamic_description_keys() {
        assert_eq!(kebab_case("defaultValue"), "default-value");
        assert_eq!(kebab_case("name"), "name");
    }

    #[test]
    fn parses_tick_strings() {
        assert_eq!(ticks(&Value::from("7056000000flk")), Some(7_056_000_000));
        assert_eq!(ticks(&Value::from("-5flk")), Some(-5));
        assert_eq!(ticks(&Value::from("5")), None);
    }
}
