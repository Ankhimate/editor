//! Tags in layer names, and what a PSD says about itself.
//!
//! A layered PSD is already a rig nobody has told the computer about. Two ways
//! to read it, and this module is the first:
//!
//! - **Tags** — `[bone]`, `[slot:name]` — what the artist *said*. Explicit,
//!   exact, and written in the one place every art tool round-trips: the layer
//!   name.
//! - **Inference** (`psd_infer`) — what the file *implies*. A group of layers
//!   named `fire_01…fire_05` is a flipbook; a face grouped for tidiness is one
//!   bone, not eleven.
//!
//! A tag always wins. Inference fills the silence, and every guess it makes is
//! reported so it can be corrected — the same principle `LoadReport` applies to
//! loss, applied to structure.
//!
//! # Why one grammar
//!
//! The three markers this replaces — `$pivot`, `$ik <name>`, `@skin:<name>` —
//! each had their own syntax and none composed: a group could not be both a
//! bone and a skin. `[tag]` and `[tag:value]` compose by construction, and a
//! reader that knows the shape can skip a tag it does not recognise instead of
//! choking on it, which is what lets a newer file open in an older build.
//!
//! # Inheritance, and blocking it
//!
//! A tag on a group applies to the group. A tag *pluralised* — `[bones]`,
//! `[slots]` — applies to each immediate child, which is how "every layer in
//! here is its own bone" is said once rather than eleven times. `[!bone]` on a
//! child refuses an inherited tag, because the exception is the thing worth
//! writing down.

use std::collections::BTreeMap;

/// What a layer's name asks for, once the tags are stripped out.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Tags {
    /// The name with every `[…]` removed and whitespace tidied.
    ///
    /// This is what the bone, slot or attachment is actually called — an artist
    /// writing `arm [bone][slot:upper]` means a thing called `arm`.
    pub name: String,
    /// Tag name to value, `None` for a bare `[tag]`.
    values: BTreeMap<String, Option<String>>,
    /// Tags this layer refuses to inherit, from `[!tag]`.
    blocked: Vec<String>,
}

impl Tags {
    /// Read the tags out of a layer name.
    pub fn parse(raw: &str) -> Self {
        let mut values = BTreeMap::new();
        let mut blocked = Vec::new();
        let mut name = String::with_capacity(raw.len());

        let mut rest = raw;
        while let Some(open) = rest.find('[') {
            name.push_str(&rest[..open]);
            let after = &rest[open + 1..];
            let Some(close) = after.find(']') else {
                // An unclosed bracket is part of the name, not a broken tag. An
                // artist writing `arm [wip` should get a layer called that
                // rather than an error about a file they cannot see the problem
                // in.
                name.push_str(&rest[open..]);
                rest = "";
                break;
            };
            let body = &after[..close];
            rest = &after[close + 1..];

            if let Some(refused) = body.strip_prefix('!') {
                blocked.push(refused.trim().to_ascii_lowercase());
                continue;
            }
            let (key, value) = match body.split_once(':') {
                Some((k, v)) => (k.trim().to_ascii_lowercase(), Some(v.trim().to_string())),
                None => (body.trim().to_ascii_lowercase(), None),
            };
            if !key.is_empty() {
                values.insert(key, value);
            }
        }
        name.push_str(rest);

        Self {
            name: name.split_whitespace().collect::<Vec<_>>().join(" "),
            values,
            blocked,
        }
    }

    /// Is this tag present?
    pub fn has(&self, tag: &str) -> bool {
        self.values.contains_key(tag)
    }

    /// The tag's value, if it was given one.
    pub fn value(&self, tag: &str) -> Option<&str> {
        self.values.get(tag).and_then(|v| v.as_deref())
    }

    /// A tag's value, or the layer's own name when the tag is bare.
    ///
    /// `[slot]` means "a slot named after this layer" and `[slot:torso]` names
    /// it; both are common and neither should need a second tag to disambiguate.
    pub fn value_or_name(&self, tag: &str) -> Option<&str> {
        match self.values.get(tag) {
            Some(Some(value)) => Some(value),
            Some(None) => Some(&self.name),
            None => None,
        }
    }

    /// A numeric value, when the tag carries one.
    pub fn number(&self, tag: &str) -> Option<f32> {
        self.value(tag)?.parse().ok()
    }

    /// Does this layer refuse to inherit `tag`?
    pub fn blocks(&self, tag: &str) -> bool {
        self.blocked.iter().any(|b| b == tag)
    }

    /// Every tag present, for reporting one the reader did not understand.
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.values.keys().map(String::as_str)
    }

    /// Take a tag from a parent, unless this layer already has it or refuses it.
    ///
    /// The child's own tag wins: a group saying `[slots]` and a child saying
    /// `[slot:hand]` means the child is a slot called `hand`, not a conflict.
    pub fn inherit(&mut self, tag: &str, value: Option<String>) {
        if self.has(tag) || self.blocks(tag) {
            return;
        }
        self.values.insert(tag.to_string(), value);
    }
}

/// Tags this reader understands. Anything else is reported, not ignored.
///
/// Listed here rather than checked at each use so a file naming `[bonee]` is
/// told about — a silently dropped tag is an artist wondering why their rig
/// came in wrong.
pub const KNOWN: &[&str] = &[
    // Structure
    "bone", "bones", "slot", "slots", "skin", "folder", "ik", "pivot", // Geometry
    "mesh", "weights", "clip", "box", "point", // Behaviour
    "physics", "frames", "fps", // Processing
    "scale", "ignore", "merge", "blend", "alpha",
];

/// The plural form of a tag, when it has one.
///
/// `[bones]` on a group is `[bone]` on each child. Kept as data rather than as
/// a match so adding a tag does not mean remembering to add its plural.
pub fn singular_of(plural: &str) -> Option<&'static str> {
    match plural {
        "bones" => Some("bone"),
        "slots" => Some("slot"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_plain_name_carries_no_tags() {
        let tags = Tags::parse("torso");
        assert_eq!(tags.name, "torso");
        assert!(!tags.has("bone"));
    }

    #[test]
    fn tags_are_stripped_out_of_the_name() {
        // The layer is called `arm`. An artist should not end up with a bone
        // named `arm [bone][slot:upper]`.
        let tags = Tags::parse("arm [bone][slot:upper]");
        assert_eq!(tags.name, "arm");
        assert!(tags.has("bone"));
        assert_eq!(tags.value("slot"), Some("upper"));
    }

    #[test]
    fn a_bare_tag_falls_back_to_the_layer_name() {
        // `[slot]` means "a slot named after me", which is the common case and
        // should not need `[slot:arm]` on a layer already called `arm`.
        let tags = Tags::parse("arm [slot]");
        assert_eq!(tags.value_or_name("slot"), Some("arm"));
        assert_eq!(tags.value("slot"), None, "bare means no explicit value");
    }

    #[test]
    fn tags_are_case_insensitive() {
        // An artist typing `[Bone]` means `[bone]`, and a rig that silently
        // came in wrong because of a capital is a bad afternoon.
        let tags = Tags::parse("arm [BONE][Slot:Upper]");
        assert!(tags.has("bone"));
        assert_eq!(
            tags.value("slot"),
            Some("Upper"),
            "the key folds, the value does not — it is a name"
        );
    }

    #[test]
    fn an_unclosed_bracket_is_part_of_the_name() {
        // `arm [wip` is a layer someone is working on, not a syntax error in a
        // file they cannot see the problem in.
        let tags = Tags::parse("arm [wip");
        assert_eq!(tags.name, "arm [wip");
        assert!(tags.names().next().is_none());
    }

    #[test]
    fn a_child_refuses_an_inherited_tag() {
        // The exception is the thing worth writing down: a group of eleven face
        // layers where one really is a bone.
        let mut child = Tags::parse("eye [!bone]");
        child.inherit("bone", None);
        assert!(!child.has("bone"), "the block held");

        let mut sibling = Tags::parse("brow");
        sibling.inherit("bone", None);
        assert!(sibling.has("bone"), "and the others still inherit");
    }

    #[test]
    fn a_childs_own_tag_beats_an_inherited_one() {
        // `[slots]` on the group and `[slot:hand]` on the child is not a
        // conflict — it is the child being specific.
        let mut child = Tags::parse("hand [slot:hand]");
        child.inherit("slot", Some("wrist".into()));
        assert_eq!(child.value("slot"), Some("hand"));
    }

    #[test]
    fn a_plural_names_its_singular() {
        assert_eq!(singular_of("bones"), Some("bone"));
        assert_eq!(singular_of("slots"), Some("slot"));
        assert_eq!(singular_of("meshes"), None, "not every tag pluralises");
    }

    #[test]
    fn a_number_reads_as_one() {
        let tags = Tags::parse("cape [scale:0.5][frames:5]");
        assert_eq!(tags.number("scale"), Some(0.5));
        assert_eq!(tags.number("frames"), Some(5.0));
        assert_eq!(tags.number("bone"), None);
    }

    #[test]
    fn several_tags_on_one_layer_all_survive() {
        // The failure of the markers this replaces: a group could not be both a
        // bone and a skin, because each marker owned the whole name.
        let tags = Tags::parse("cape [bone][physics:cloth][skin:winter]");
        assert!(tags.has("bone"));
        assert_eq!(tags.value("physics"), Some("cloth"));
        assert_eq!(tags.value("skin"), Some("winter"));
        assert_eq!(tags.name, "cape");
    }

    #[test]
    fn an_unknown_tag_is_readable_so_it_can_be_reported() {
        // A misspelled tag must be findable. Dropping it silently is an artist
        // wondering why `[bonee]` did nothing.
        let tags = Tags::parse("arm [bonee]");
        let unknown: Vec<&str> = tags.names().filter(|t| !KNOWN.contains(t)).collect();
        assert_eq!(unknown, ["bonee"]);
    }
}
