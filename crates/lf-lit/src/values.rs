//! The `values` array of a value controller's state.
//!
//! In the binary container the array starts with a `u32` count of
//! *components*, not elements: a vector2 array of 4 points has 8 components.
//! The controller's class decides how components are encoded.

use uuid::Uuid;

use crate::error::{Error, Result};

/// How a controller class encodes its components.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Encoding {
    F32,
    F64,
    I32,
    I64,
    U8,
    Bool,
    /// Timeline time or duration, in ticks (see [`crate::time::TICKS_PER_SECOND`]).
    Ticks,
    /// Windows FILETIME.
    FileTime,
    /// `u32` byte length + UTF-8, per element. Enums store the value's name.
    Strings,
    Uuids,
}

/// The encoding and element width for a value-controller type name such as
/// `lightform.object.value-controller.plain-vector2`. `None` for classes that
/// don't hold a `values` array or aren't known.
pub fn encoding_for(type_name: &str) -> Option<(Encoding, usize)> {
    use Encoding::*;
    let kind = type_name.rsplit('.').next()?.strip_prefix("plain-")?;
    Some(match kind {
        "float" => (F32, 1),
        "vector2" => (F32, 2),
        "vector3" => (F32, 3),
        "vector4" | "color4f" | "quaternion" => (F32, 4),
        "matrix3" => (F32, 9),
        "matrix4" => (F32, 16),
        "double" | "angle" | "distance" => (F64, 1),
        "int32" => (I32, 1),
        "vector2i" => (I32, 2),
        "vector3i" => (I32, 3),
        "vector4i" => (I32, 4),
        "int64" => (I64, 1),
        "color4u8" => (U8, 4),
        "boolean" => (Bool, 1),
        "timeline-time" | "timeline-duration" => (Ticks, 1),
        "date-time" => (FileTime, 1),
        "string" | "url" | "enum" | "file-system-path" => (Strings, 1),
        "uuid" => (Uuids, 1),
        _ => return None,
    })
}

/// A decoded `values` array. Fixed-width variants hold components in order.
#[derive(Debug, Clone, PartialEq)]
pub enum Values {
    F32(Vec<f32>),
    F64(Vec<f64>),
    I32(Vec<i32>),
    I64(Vec<i64>),
    U8(Vec<u8>),
    /// Stored bytes, kept as read so writing reproduces them; nonzero is true.
    Bool(Vec<u8>),
    Ticks(Vec<i64>),
    FileTime(Vec<i64>),
    Strings(Vec<String>),
    Uuids(Vec<Uuid>),
    /// A binary array whose controller class isn't known.
    Raw {
        count: u32,
        bytes: Vec<u8>,
    },
    /// A JSON-container array whose controller class isn't known.
    Json(Vec<serde_json::Value>),
}

impl Values {
    /// Decode a binary array value (count + components).
    pub fn decode(encoding: Option<Encoding>, value: &[u8]) -> Result<Values> {
        let Some((count, body)) = value.split_first_chunk::<4>() else {
            return Err(Error::Malformed(
                "values array is shorter than its count".into(),
            ));
        };
        let count = u32::from_le_bytes(*count);
        let n = count as usize;
        let Some(encoding) = encoding else {
            return Ok(Values::Raw {
                count,
                bytes: body.to_vec(),
            });
        };
        Ok(match encoding {
            Encoding::F32 => Values::F32(fixed(body, n, f32::from_le_bytes)?),
            Encoding::F64 => Values::F64(fixed(body, n, f64::from_le_bytes)?),
            Encoding::I32 => Values::I32(fixed(body, n, i32::from_le_bytes)?),
            Encoding::I64 => Values::I64(fixed(body, n, i64::from_le_bytes)?),
            Encoding::Ticks => Values::Ticks(fixed(body, n, i64::from_le_bytes)?),
            Encoding::FileTime => Values::FileTime(fixed(body, n, i64::from_le_bytes)?),
            Encoding::U8 => Values::U8(fixed(body, n, u8::from_le_bytes)?),
            Encoding::Bool => Values::Bool(fixed(body, n, u8::from_le_bytes)?),
            Encoding::Uuids => Values::Uuids(fixed(body, n, Uuid::from_bytes)?),
            Encoding::Strings => Values::Strings(strings(body, n)?),
        })
    }

    /// Encode as a binary array value (count + components).
    pub fn encode(&self) -> Result<Vec<u8>> {
        let mut out = (self.len() as u32).to_le_bytes().to_vec();
        match self {
            Values::F32(v) => v
                .iter()
                .for_each(|x| out.extend_from_slice(&x.to_le_bytes())),
            Values::F64(v) => v
                .iter()
                .for_each(|x| out.extend_from_slice(&x.to_le_bytes())),
            Values::I32(v) => v
                .iter()
                .for_each(|x| out.extend_from_slice(&x.to_le_bytes())),
            Values::I64(v) | Values::Ticks(v) | Values::FileTime(v) => v
                .iter()
                .for_each(|x| out.extend_from_slice(&x.to_le_bytes())),
            Values::U8(v) | Values::Bool(v) => out.extend_from_slice(v),
            Values::Uuids(v) => v.iter().for_each(|x| out.extend_from_slice(x.as_bytes())),
            Values::Strings(v) => {
                for s in v {
                    out.extend_from_slice(&(s.len() as u32).to_le_bytes());
                    out.extend_from_slice(s.as_bytes());
                }
            }
            Values::Raw { bytes, .. } => out.extend_from_slice(bytes),
            Values::Json(_) => {
                return Err(Error::Malformed(
                    "can't write values of an unknown controller class to the binary container"
                        .into(),
                ));
            }
        }
        Ok(out)
    }

    /// Number of components (strings and UUIDs count one each).
    pub fn len(&self) -> usize {
        match self {
            Values::F32(v) => v.len(),
            Values::F64(v) => v.len(),
            Values::I32(v) => v.len(),
            Values::I64(v) | Values::Ticks(v) | Values::FileTime(v) => v.len(),
            Values::U8(v) | Values::Bool(v) => v.len(),
            Values::Strings(v) => v.len(),
            Values::Uuids(v) => v.len(),
            Values::Raw { count, .. } => *count as usize,
            Values::Json(v) => v.len(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// True when the controller class wasn't known, so the components weren't decoded.
    pub fn is_undecoded(&self) -> bool {
        matches!(self, Values::Raw { .. } | Values::Json(_))
    }
}

/// Check that a state's element `count` agrees with its component array.
pub(crate) fn check_count(
    path: &str,
    count: Option<i64>,
    values: Option<&Values>,
    width: Option<usize>,
) -> Result<()> {
    if let (Some(count), Some(values), Some(width)) = (count, values, width)
        && usize::try_from(count)
            .ok()
            .and_then(|c| c.checked_mul(width))
            != Some(values.len())
    {
        return Err(Error::Malformed(format!(
            "{path}: count is {count} elements but values hold {} components of width {width}",
            values.len()
        )));
    }
    Ok(())
}

fn fixed<const W: usize, T>(body: &[u8], n: usize, from: impl Fn([u8; W]) -> T) -> Result<Vec<T>> {
    if Some(body.len()) != n.checked_mul(W) {
        return Err(Error::Malformed(format!(
            "values array holds {} bytes, expected {n} components of {W} bytes",
            body.len()
        )));
    }
    Ok(body.as_chunks::<W>().0.iter().map(|c| from(*c)).collect())
}

fn strings(body: &[u8], n: usize) -> Result<Vec<String>> {
    // Every string needs at least its 4 length bytes, so a larger count is
    // malformed; checking first keeps a crafted count from sizing the buffer.
    if n > body.len() / 4 {
        return Err(Error::Malformed(format!(
            "values array claims {n} strings but holds only {} bytes",
            body.len()
        )));
    }
    let mut out = Vec::with_capacity(n);
    let mut rest = body;
    for _ in 0..n {
        let (s, tail) = read_string(rest)?;
        out.push(s);
        rest = tail;
    }
    if !rest.is_empty() {
        return Err(Error::Malformed(format!(
            "{} bytes left after {n} strings",
            rest.len()
        )));
    }
    Ok(out)
}

/// Read a `u32` byte length + UTF-8 string, returning the remaining bytes.
pub(crate) fn read_string(bytes: &[u8]) -> Result<(String, &[u8])> {
    let Some((len, rest)) = bytes.split_first_chunk::<4>() else {
        return Err(Error::Malformed("string is shorter than its length".into()));
    };
    let len = u32::from_le_bytes(*len) as usize;
    if rest.len() < len {
        return Err(Error::Malformed(
            "string runs past the end of its value".into(),
        ));
    }
    let (text, rest) = rest.split_at(len);
    let text = std::str::from_utf8(text)
        .map_err(|e| Error::Malformed(format!("string is not valid UTF-8: {e}")))?;
    Ok((text.to_owned(), rest))
}

/// Encode a `u32` byte length + UTF-8 string.
pub(crate) fn write_string(s: &str) -> Vec<u8> {
    let mut out = (s.len() as u32).to_le_bytes().to_vec();
    out.extend_from_slice(s.as_bytes());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn round_trip(encoding: Encoding, values: Values) {
        let bytes = values.encode().unwrap();
        assert_eq!(Values::decode(Some(encoding), &bytes).unwrap(), values);
    }

    #[test]
    fn round_trips_each_encoding() {
        round_trip(Encoding::F32, Values::F32(vec![1.5, -2.0, 3.25, 0.0]));
        round_trip(Encoding::F64, Values::F64(vec![std::f64::consts::PI]));
        round_trip(Encoding::I32, Values::I32(vec![4, -4]));
        round_trip(Encoding::Ticks, Values::Ticks(vec![7_056_000_000]));
        round_trip(Encoding::Bool, Values::Bool(vec![1, 0]));
        round_trip(
            Encoding::Strings,
            Values::Strings(vec!["UNION".into(), "".into(), "é".into()]),
        );
        round_trip(Encoding::Uuids, Values::Uuids(vec![Uuid::from_u128(42)]));
    }

    #[test]
    fn counts_components_not_elements() {
        let points = Values::F32(vec![0.0; 8]);
        assert_eq!(&points.encode().unwrap()[..4], &8u32.to_le_bytes());
    }

    #[test]
    fn unknown_class_stays_raw() {
        let bytes = [2, 0, 0, 0, 0xaa, 0xbb];
        let values = Values::decode(None, &bytes).unwrap();
        assert!(values.is_undecoded());
        assert_eq!(values.encode().unwrap(), bytes);
    }

    #[test]
    fn rejects_size_mismatch_and_bad_utf8() {
        assert!(Values::decode(Some(Encoding::F32), &[2, 0, 0, 0, 0, 0, 0, 0]).is_err());
        assert!(Values::decode(Some(Encoding::Strings), &[1, 0, 0, 0, 1, 0, 0, 0, 0xff]).is_err());
    }

    #[test]
    fn rejects_string_count_larger_than_the_data() {
        let bytes = [0xff, 0xff, 0xff, 0x7f, 0, 0, 0, 0];
        assert!(Values::decode(Some(Encoding::Strings), &bytes).is_err());
    }

    #[test]
    fn known_controller_types() {
        let t = "lightform.object.value-controller.plain-vector2";
        assert_eq!(encoding_for(t), Some((Encoding::F32, 2)));
        assert_eq!(
            encoding_for("lightform.object.value-controller.plain-object-reference"),
            None
        );
        assert_eq!(encoding_for("lightform.object.project"), None);
    }
}
