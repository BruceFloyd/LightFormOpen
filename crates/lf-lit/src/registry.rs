//! Class UUID → type name, recovered from the (display name, type, UUID)
//! triples in Creator's binaries. Value decoding needs it: a value controller's
//! class determines how its `values` array is encoded.

use std::collections::HashMap;
use std::sync::OnceLock;

use uuid::Uuid;

use crate::error::{Error, Result};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClassInfo {
    /// Dotted type name, e.g. `lightform.object.value-controller.plain-float`.
    pub type_name: String,
    pub display: String,
}

#[derive(Debug, Default)]
pub struct Registry {
    classes: HashMap<Uuid, ClassInfo>,
}

impl Registry {
    /// The registry recovered from Creator 2.2.12.
    pub fn builtin() -> &'static Registry {
        static BUILTIN: OnceLock<Registry> = OnceLock::new();
        BUILTIN.get_or_init(|| {
            Registry::from_json(include_str!("../data/class_registry.json"))
                .expect("built-in class registry is valid")
        })
    }

    /// Parse `{"<uuid>": {"type": "...", "display": "..."}, ...}`.
    pub fn from_json(text: &str) -> Result<Registry> {
        let root: serde_json::Value = serde_json::from_str(text)?;
        let entries = root
            .as_object()
            .ok_or_else(|| Error::Malformed("class registry is not a JSON object".into()))?;
        let mut classes = HashMap::with_capacity(entries.len());
        for (id, info) in entries {
            let id = Uuid::parse_str(id).map_err(|_| {
                Error::Malformed(format!("class registry key {id:?} is not a UUID"))
            })?;
            let field = |name: &str| info.get(name).and_then(|v| v.as_str()).unwrap_or("");
            classes.insert(
                id,
                ClassInfo {
                    type_name: field("type").to_owned(),
                    display: field("display").to_owned(),
                },
            );
        }
        Ok(Registry { classes })
    }

    pub fn get(&self, class_id: &Uuid) -> Option<&ClassInfo> {
        self.classes.get(class_id)
    }

    pub fn len(&self) -> usize {
        self.classes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.classes.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_registry_loads() {
        let registry = Registry::builtin();
        assert_eq!(registry.len(), 424);
        let effect = Uuid::parse_str("00076275-62ae-4dc8-9c01-d26ce74ee230").unwrap();
        assert_eq!(
            registry.get(&effect).unwrap().type_name,
            "lightform.object.effect"
        );
    }

    /// `tools/re/litdump.py` reads its own copy; the two must not drift.
    #[test]
    fn matches_the_tools_copy() {
        let tools = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tools/re/lit_class_registry.json"
        );
        if let Ok(text) = std::fs::read_to_string(tools) {
            assert!(
                text == include_str!("../data/class_registry.json"),
                "crates/lf-lit/data/class_registry.json differs from {tools}"
            );
        }
    }
}
