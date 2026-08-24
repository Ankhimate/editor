//! Stable short-key projection shared by minified JSON and binary Ankh v1.

use serde_json::Value;

// Append-only. Position is the wire tag; published entries are never reordered.
const FIELDS: &[&str] = &[
    "version",
    "name",
    "fps",
    "assets",
    "bones",
    "slots",
    "draw_order",
    "skins",
    "default_skin",
    "constraints",
    "constraint_order",
    "animations",
    "groups",
    "psd_layer_paths",
    "export_presets",
    "attachment",
    "audio",
    "balance",
    "bend_direction",
    "blend_mode",
    "bone",
    "bone_offsets",
    "channels",
    "closed",
    "color",
    "constant_speed",
    "dark_color",
    "duration",
    "edges",
    "end_slot",
    "entries",
    "events",
    "file",
    "float_value",
    "forces",
    "frames",
    "handles",
    "height",
    "inherit_deform",
    "inherit_reflect",
    "inherit_rotation",
    "inherit_scale",
    "int_value",
    "interp",
    "kind",
    "keys",
    "length",
    "linked",
    "local",
    "looping",
    "markers",
    "members",
    "mix",
    "mode",
    "offset",
    "offset_x",
    "offset_y",
    "offsets",
    "parent",
    "path",
    "physics",
    "pivot_x",
    "pivot_y",
    "relative",
    "rotate",
    "rotation",
    "scale_x",
    "scale_y",
    "sequence",
    "setup_index",
    "shear_x",
    "shear_y",
    "skin",
    "slot",
    "softness",
    "source_path",
    "stiffness",
    "stretch",
    "stretch_limit",
    "string_value",
    "sx",
    "sy",
    "target",
    "texture",
    "time",
    "timelines",
    "transform_mix",
    "translate_x",
    "translate_y",
    "triangles",
    "tx",
    "ty",
    "type",
    "uv",
    "uvs",
    "value",
    "vertices",
    "volume",
    "weights",
    "width",
    "x",
    "y",
    "axis",
    "constraint",
    "curve",
];

const ALPHABET: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";

fn tag(index: usize) -> String {
    if index < ALPHABET.len() {
        return (ALPHABET[index] as char).to_string();
    }
    let n = index - ALPHABET.len();
    format!(
        "{}{}",
        ALPHABET[n / ALPHABET.len()] as char,
        ALPHABET[n % ALPHABET.len()] as char
    )
}

fn field_for_tag(value: &str) -> Option<&'static str> {
    FIELDS
        .iter()
        .enumerate()
        .find_map(|(index, field)| (tag(index) == value).then_some(*field))
}

pub fn contract(value: Value) -> Value {
    transform(value, true)
}

pub fn expand(value: Value) -> Value {
    transform(value, false)
}

fn transform(value: Value, contracting: bool) -> Value {
    match value {
        Value::Array(values) => Value::Array(
            values
                .into_iter()
                .map(|v| transform(v, contracting))
                .collect(),
        ),
        Value::Object(values) => {
            let mut out = serde_json::Map::new();
            for (key, value) in values {
                let key = if contracting {
                    FIELDS
                        .iter()
                        .position(|field| *field == key)
                        .map(tag)
                        .unwrap_or_else(|| format!("~{key}"))
                } else if let Some(original) = key.strip_prefix('~') {
                    original.to_string()
                } else {
                    field_for_tag(&key).unwrap_or(&key).to_string()
                };
                out.insert(key, transform(value, contracting));
            }
            Value::Object(out)
        }
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn known_keys_shrink_and_unknown_keys_survive() {
        let source = serde_json::json!({"version":1,"animations":[],"future_field":42});
        let compact = contract(source.clone());
        assert!(compact.get("a").is_some());
        assert_eq!(expand(compact), source);
    }
}
