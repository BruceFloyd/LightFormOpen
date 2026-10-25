//! Property keys. The binary container stores each property name as a 32-bit
//! hash of its kebab-case spelling: `h = h * 33 + byte`, unseeded.

/// Hash a property name the way the binary container stores it.
pub const fn key_hash(name: &str) -> u32 {
    let bytes = name.as_bytes();
    let mut h: u32 = 0;
    let mut i = 0;
    while i < bytes.len() {
        h = h.wrapping_mul(33).wrapping_add(bytes[i] as u32);
        i += 1;
    }
    h
}

/// Every key name found in the engine's chunk-property-name table, plus the
/// top-level header keys.
pub const KEY_NAMES: &[&str] = &[
    "magic",
    "format",
    "format-version",
    "max-bytes-per-line",
    "id",
    "name",
    "class-id",
    "class-name",
    "control-type",
    "count",
    "data-version-number",
    "default-value",
    "enum-description-id",
    "flags",
    "group-name",
    "interface-description-id",
    "is-array",
    "local-id",
    "maximum-value",
    "minimum-value",
    "object-description-id",
    "original-data-version-number",
    "owner-id",
    "precision",
    "purpose-enum-description-id",
    "purpose-enum-value",
    "range",
    "subtype-id",
    "value",
    "value-controller-object-description-id",
    "value-count",
    "value-names",
    "value-type",
    "values",
];

/// The name for a stored key hash, if it is a known key.
pub fn key_name(hash: u32) -> Option<&'static str> {
    KEY_NAMES.iter().copied().find(|n| key_hash(n) == hash)
}

pub const MAGIC: u32 = key_hash("magic");
pub const FORMAT: u32 = key_hash("format");
pub const FORMAT_VERSION: u32 = key_hash("format-version");
pub const MAX_BYTES_PER_LINE: u32 = key_hash("max-bytes-per-line");
pub const DATA_VERSION: u32 = key_hash("data-version-number");
pub const ORIGINAL_DATA_VERSION: u32 = key_hash("original-data-version-number");
pub const COUNT: u32 = key_hash("count");
pub const VALUES: u32 = key_hash("values");
pub const ID: u32 = key_hash("id");
pub const CLASS_ID: u32 = key_hash("class-id");
pub const OWNER_ID: u32 = key_hash("owner-id");
pub const NAME: u32 = key_hash("name");
pub const IS_ARRAY: u32 = key_hash("is-array");
pub const SUBTYPE_ID: u32 = key_hash("subtype-id");
pub const LOCAL_ID: u32 = key_hash("local-id");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_matches_observed_value() {
        assert_eq!(key_hash("id"), 0x0000_0ded);
    }

    #[test]
    fn names_have_distinct_hashes() {
        for (i, a) in KEY_NAMES.iter().enumerate() {
            for b in &KEY_NAMES[i + 1..] {
                assert_ne!(key_hash(a), key_hash(b), "{a} and {b} collide");
            }
        }
    }
}
