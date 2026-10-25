//! A readable JSON rendering of a document, in the same shape as
//! `tools/re/litdump.py --registry` so the two can be compared:
//! `{"container", "header", "objects": {"<uuid>": {"name", "classId", "ownerId",
//! "state", "references", "class"}}}`.

use serde_json::{Map, Value, json};

use crate::document::{Container, Document, Object, State};
use crate::registry::Registry;
use crate::time::filetime_to_iso;
use crate::values::Values;

pub fn to_json(doc: &Document, registry: &Registry) -> Value {
    let h = &doc.header;
    let mut header = Map::new();
    header.insert("formatVersion".into(), json!(h.format_version));
    if let Some(v) = h.max_bytes_per_line {
        header.insert("maxBytesPerLine".into(), json!(v));
    }
    if let Some(v) = &h.blob_text_format {
        header.insert("blobTextFormat".into(), json!(v));
    }
    header.insert("dataVersionNumber".into(), json!(h.data_version));
    header.insert(
        "originalDataVersionNumber".into(),
        json!(h.original_data_version),
    );

    let objects: Map<String, Value> = doc
        .objects
        .iter()
        .map(|o| (o.id.to_string(), object(o, registry)))
        .collect();
    json!({
        "container": match doc.container { Container::Binary => "binary", Container::Json => "json" },
        "header": header,
        "objects": objects,
    })
}

fn object(o: &Object, registry: &Registry) -> Value {
    let mut m = Map::new();
    m.insert("name".into(), json!(o.name));
    m.insert("classId".into(), json!(o.class_id.to_string()));
    m.insert("ownerId".into(), json!(o.owner_id.map(|u| u.to_string())));
    if let Some(state) = &o.state {
        m.insert("state".into(), self::state(state));
    }
    if let Some(refs) = &o.references {
        let refs: Map<String, Value> = refs
            .iter()
            .map(|r| (r.local_id.to_string(), json!(r.id.to_string())))
            .collect();
        m.insert("references".into(), Value::Object(refs));
    }
    if let Some(info) = registry.get(&o.class_id) {
        m.insert("class".into(), json!(info.type_name));
    }
    Value::Object(m)
}

fn state(s: &State) -> Value {
    let mut m = Map::new();
    if let Some(v) = s.subtype_id {
        m.insert("subtypeId".into(), json!(v.to_string()));
    }
    if let Some(v) = s.is_array {
        m.insert("isArray".into(), json!(v));
    }
    if let Some(v) = s.count {
        m.insert("count".into(), json!(v));
    }
    if let Some(v) = &s.values {
        m.insert("values".into(), values(v));
    }
    if let Some(desc) = &s.dynamic_description {
        let desc: Map<String, Value> = desc
            .iter()
            .map(|(k, v)| (camel_case(k), json!(v)))
            .collect();
        m.insert("dynamicDescription".into(), Value::Object(desc));
    }
    Value::Object(m)
}

fn values(v: &Values) -> Value {
    match v {
        Values::F32(v) => v.iter().map(|&x| float(f64::from(x))).collect(),
        Values::F64(v) => v.iter().map(|&x| float(x)).collect(),
        Values::I32(v) => json!(v),
        Values::I64(v) | Values::Ticks(v) => json!(v),
        Values::U8(v) => json!(v),
        Values::Bool(v) => v.iter().map(|&b| json!(b != 0)).collect(),
        Values::FileTime(v) => v.iter().map(|&t| json!(filetime_to_iso(t))).collect(),
        Values::Strings(v) => json!(v),
        Values::Uuids(v) => v.iter().map(|u| json!(u.to_string())).collect(),
        Values::Raw { count, bytes } => {
            let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
            json!({ "count": count, "raw": hex })
        }
        Values::Json(v) => json!(v),
    }
}

/// JSON has no NaN or infinity, so those become strings.
fn float(x: f64) -> Value {
    if x.is_finite() {
        json!(x)
    } else if x.is_nan() {
        json!("NaN")
    } else if x > 0.0 {
        json!("Infinity")
    } else {
        json!("-Infinity")
    }
}

fn camel_case(kebab: &str) -> String {
    let mut parts = kebab.split('-');
    let mut out = parts.next().unwrap_or_default().to_owned();
    for part in parts {
        let mut chars = part.chars();
        if let Some(first) = chars.next() {
            out.extend(first.to_uppercase());
            out.push_str(chars.as_str());
        }
    }
    out
}
