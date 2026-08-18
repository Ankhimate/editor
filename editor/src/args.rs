//! Operator arguments, and the name-to-id resolution they need.
//!
//! A keybinding invokes an operator with no arguments — it acts on the
//! selection, which is already live state. Every other caller cannot: a plugin
//! or an MCP client asking to "create a bone named `spine` under `root`" has to
//! say so, and has no way to hold a [`BoneId`].
//!
//! # Why names, not ids
//!
//! Slotmap keys are not stable across sessions (ADR 0004). A plugin that stored
//! one would break on the next load, and a script written against a rig would
//! break the moment a bone was deleted and undone — `IdRemap` exists precisely
//! because undo hands a restored bone a *new* key.
//!
//! So arguments carry names and resolve at invoke time, which is the same
//! contract `formats/src/schema.rs` uses on disk: names in the file, ids in
//! memory. Resolution failure is an error the caller sees, not a silent no-op.

use ankhimate_core::ids::{AnimationId, BoneId, SlotId};
use serde_json::Value;

/// What an operator was asked to do, before names become ids.
///
/// Thin on purpose. This is a JSON object with typed accessors rather than a
/// generated struct per operator: the set of operators is open — a plugin adds
/// its own — so the argument type cannot be an enum over known shapes without
/// closing the thing the registry exists to keep open.
#[derive(Debug, Clone, Default)]
pub struct Args(Value);

/// What went wrong reading an argument.
///
/// Carries the argument's name in every case. A caller that mistyped `bone` as
/// `bones` and a caller that named a bone the rig does not have need to be told
/// different things, and "invalid arguments" tells them neither.
#[derive(Debug, Clone, PartialEq)]
pub enum ArgError {
    /// The key is absent and the operator needs it.
    Missing(String),
    /// Present, but the wrong JSON type.
    WrongType {
        key: String,
        wanted: &'static str,
        got: &'static str,
    },
    /// A well-formed name that resolves to nothing in this document.
    Unresolved {
        key: String,
        kind: &'static str,
        name: String,
    },
}

impl std::fmt::Display for ArgError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ArgError::Missing(key) => write!(f, "missing argument `{key}`"),
            ArgError::WrongType { key, wanted, got } => {
                write!(f, "`{key}` should be {wanted}, got {got}")
            }
            ArgError::Unresolved { key, kind, name } => {
                write!(f, "`{key}` names a {kind} this rig does not have: `{name}`")
            }
        }
    }
}

impl std::error::Error for ArgError {}

/// A JSON value's type, for error messages.
fn type_of(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "a boolean",
        Value::Number(_) => "a number",
        Value::String(_) => "a string",
        Value::Array(_) => "an array",
        Value::Object(_) => "an object",
    }
}

impl Args {
    /// No arguments — what a keybinding passes.
    pub fn none() -> Self {
        Self(Value::Null)
    }

    pub fn from_json(value: Value) -> Self {
        Self(value)
    }

    pub fn as_json(&self) -> &Value {
        &self.0
    }

    /// Is anything here at all?
    pub fn is_empty(&self) -> bool {
        match &self.0 {
            Value::Null => true,
            Value::Object(map) => map.is_empty(),
            _ => false,
        }
    }

    fn get(&self, key: &str) -> Option<&Value> {
        self.0.get(key).filter(|v| !v.is_null())
    }

    pub fn str(&self, key: &str) -> Result<&str, ArgError> {
        let v = self.get(key).ok_or_else(|| ArgError::Missing(key.into()))?;
        v.as_str().ok_or_else(|| ArgError::WrongType {
            key: key.into(),
            wanted: "a string",
            got: type_of(v),
        })
    }

    pub fn f32(&self, key: &str) -> Result<f32, ArgError> {
        let v = self.get(key).ok_or_else(|| ArgError::Missing(key.into()))?;
        v.as_f64()
            .map(|n| n as f32)
            .ok_or_else(|| ArgError::WrongType {
                key: key.into(),
                wanted: "a number",
                got: type_of(v),
            })
    }

    /// A number, or `default` when absent.
    ///
    /// Absent is not the same as present-and-wrong: a misspelled key silently
    /// taking a default is how a plugin author spends an afternoon, so a value
    /// of the wrong *type* still errors.
    pub fn f32_or(&self, key: &str, default: f32) -> Result<f32, ArgError> {
        match self.get(key) {
            None => Ok(default),
            Some(_) => self.f32(key),
        }
    }

    pub fn bool_or(&self, key: &str, default: bool) -> Result<bool, ArgError> {
        match self.get(key) {
            None => Ok(default),
            Some(v) => v.as_bool().ok_or_else(|| ArgError::WrongType {
                key: key.into(),
                wanted: "a boolean",
                got: type_of(v),
            }),
        }
    }

    /// An optional string — absent and `null` both mean "not given".
    pub fn opt_str(&self, key: &str) -> Result<Option<&str>, ArgError> {
        match self.get(key) {
            None => Ok(None),
            Some(v) => v.as_str().map(Some).ok_or_else(|| ArgError::WrongType {
                key: key.into(),
                wanted: "a string",
                got: type_of(v),
            }),
        }
    }
}

/// Resolves the names in [`Args`] against a document.
///
/// Separate from `Args` so the argument type stays free of the document, and so
/// a caller can validate a script's shape before a rig is even open.
pub struct Resolver<'a> {
    doc: &'a ankhimate_document::Document,
}

impl<'a> Resolver<'a> {
    pub fn new(doc: &'a ankhimate_document::Document) -> Self {
        Self { doc }
    }

    pub fn bone(&self, args: &Args, key: &str) -> Result<BoneId, ArgError> {
        let name = args.str(key)?;
        self.doc
            .skeleton
            .bones
            .iter()
            .find(|(_, b)| b.name == name)
            .map(|(id, _)| id)
            .ok_or_else(|| ArgError::Unresolved {
                key: key.into(),
                kind: "bone",
                name: name.into(),
            })
    }

    /// A bone argument that may be absent — for `parent`, where "no parent"
    /// is a real answer rather than a missing one.
    pub fn opt_bone(&self, args: &Args, key: &str) -> Result<Option<BoneId>, ArgError> {
        match args.opt_str(key)? {
            None => Ok(None),
            Some(_) => self.bone(args, key).map(Some),
        }
    }

    pub fn slot(&self, args: &Args, key: &str) -> Result<SlotId, ArgError> {
        let name = args.str(key)?;
        self.doc
            .skeleton
            .slots
            .iter()
            .find(|(_, s)| s.name == name)
            .map(|(id, _)| id)
            .ok_or_else(|| ArgError::Unresolved {
                key: key.into(),
                kind: "slot",
                name: name.into(),
            })
    }

    pub fn animation(&self, args: &Args, key: &str) -> Result<AnimationId, ArgError> {
        let name = args.str(key)?;
        self.doc
            .animations
            .iter()
            .find(|(_, a)| a.name == name)
            .map(|(id, _)| id)
            .ok_or_else(|| ArgError::Unresolved {
                key: key.into(),
                kind: "animation",
                name: name.into(),
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ankhimate_core::math::Transform;
    use ankhimate_core::skeleton::Bone;
    use ankhimate_document::Document;
    use serde_json::json;

    fn rig() -> Document {
        let mut doc = Document::new();
        for name in ["root", "spine"] {
            doc.skeleton.add_bone(Bone {
                name: name.into(),
                parent: None,
                length: 10.0,
                local_transform: Transform::default(),
                inherit: Default::default(),
                color: Bone::default_color(),
            });
        }
        doc
    }

    #[test]
    fn a_missing_argument_names_itself() {
        let args = Args::from_json(json!({ "parent": "root" }));
        assert_eq!(args.str("name"), Err(ArgError::Missing("name".into())));
    }

    #[test]
    fn a_wrong_type_says_what_it_wanted_and_what_it_got() {
        // "invalid arguments" tells a plugin author nothing. This tells them
        // which key, what it should be, and what they sent.
        let args = Args::from_json(json!({ "name": 42 }));
        assert_eq!(
            args.str("name"),
            Err(ArgError::WrongType {
                key: "name".into(),
                wanted: "a string",
                got: "a number",
            })
        );
    }

    #[test]
    fn a_default_covers_absence_but_not_a_wrong_type() {
        // The distinction that matters: a key nobody sent takes the default,
        // and a key sent wrongly is a bug the caller should hear about rather
        // than a value quietly replaced.
        let args = Args::from_json(json!({ "y": "up" }));
        assert_eq!(args.f32_or("x", 7.0), Ok(7.0));
        assert!(
            args.f32_or("y", 7.0).is_err(),
            "wrong type is not a default"
        );
    }

    #[test]
    fn null_reads_as_absent() {
        // A JS caller writing `{parent: null}` means "no parent", not "a bone
        // named null" — and every serializer emits it for an absent optional.
        let args = Args::from_json(json!({ "parent": null }));
        assert_eq!(args.opt_str("parent"), Ok(None));
        assert_eq!(args.f32_or("parent", 3.0), Ok(3.0));
    }

    #[test]
    fn a_name_resolves_to_the_bone_it_names() {
        let doc = rig();
        let resolver = Resolver::new(&doc);
        let args = Args::from_json(json!({ "bone": "spine" }));

        let id = resolver.bone(&args, "bone").expect("resolves");
        assert_eq!(doc.skeleton.bones[id].name, "spine");
    }

    #[test]
    fn a_name_the_rig_lacks_is_an_error_not_a_silent_none() {
        // The whole reason arguments carry names: a script naming a bone that
        // is not there has a bug in it, and doing nothing quietly is how that
        // bug survives to the next session.
        let doc = rig();
        let resolver = Resolver::new(&doc);
        let args = Args::from_json(json!({ "bone": "tail" }));

        assert_eq!(
            resolver.bone(&args, "bone"),
            Err(ArgError::Unresolved {
                key: "bone".into(),
                kind: "bone",
                name: "tail".into(),
            })
        );
    }

    #[test]
    fn an_absent_optional_bone_is_none_but_a_bad_one_still_errors() {
        // `parent` is the case: omitting it means a root bone, while naming a
        // bone that does not exist is a mistake.
        let doc = rig();
        let resolver = Resolver::new(&doc);

        let absent = Args::from_json(json!({}));
        assert_eq!(resolver.opt_bone(&absent, "parent"), Ok(None));

        let wrong = Args::from_json(json!({ "parent": "nope" }));
        assert!(resolver.opt_bone(&wrong, "parent").is_err());
    }
}
