//! The template engine (T-603b).
//!
//! Handlebars, in **strict mode**. Strict mode is not a preference: the default
//! renders a missing field as an empty string, so a typo'd `{{nmae}}` silently
//! produces a bone with no name and an export that looks fine until an engine
//! rejects it. Strict mode turns that into an error carrying template name, line
//! and column.

use handlebars::{Handlebars, handlebars_helper};
use serde_json::Value;

/// A rendered file, held in memory until the whole set succeeds.
#[derive(Debug, Clone, PartialEq)]
pub struct RenderedFile {
    /// Path relative to the export's output directory, already confined.
    pub path: String,
    pub contents: String,
}

#[derive(Debug)]
pub enum TemplateError {
    /// The template did not parse.
    Parse { template: String, reason: String },
    /// The template parsed but failed to render — usually a missing field.
    Render { template: String, reason: String },
    /// A rendered output path escaped the output directory.
    EscapingPath { template: String, path: String },
    /// A rendered output path was empty or was only separators.
    EmptyPath { template: String },
}

impl std::fmt::Display for TemplateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TemplateError::Parse { template, reason } => {
                write!(f, "template '{template}' could not be parsed: {reason}")
            }
            TemplateError::Render { template, reason } => {
                write!(f, "template '{template}' failed: {reason}")
            }
            TemplateError::EscapingPath { template, path } => write!(
                f,
                "template '{template}' produced the output path '{path}', which leaves the \
                 output directory; export aborted"
            ),
            TemplateError::EmptyPath { template } => {
                write!(f, "template '{template}' produced an empty output path")
            }
        }
    }
}

impl std::error::Error for TemplateError {}

handlebars_helper!(deg: |r: f64| r.to_degrees());
handlebars_helper!(rad: |d: f64| d.to_radians());
handlebars_helper!(round: |v: f64, places: i64| {
    let m = 10f64.powi(places.clamp(0, 10) as i32);
    let r = (v * m).round() / m;
    // -0.0 prints as "-0", which is valid JSON but a gratuitous diff against
    // the 0 a different rig writes for the same rest pose.
    if r == 0.0 { 0.0 } else { r }
});
handlebars_helper!(pad: |n: i64, width: i64| format!("{:0>width$}", n, width = width.clamp(0, 32) as usize));
handlebars_helper!(eq_helper: |a: Json, b: Json| a == b);
handlebars_helper!(ne_helper: |a: Json, b: Json| a != b);
handlebars_helper!(add_helper: |a: f64, b: f64| a + b);
handlebars_helper!(sub_helper: |a: f64, b: f64| a - b);
handlebars_helper!(mul_helper: |a: f64, b: f64| a * b);
handlebars_helper!(div_helper: |a: f64, b: f64| if b == 0.0 { 0.0 } else { a / b });
handlebars_helper!(json_helper: |v: Json| serde_json::to_string(v).unwrap_or_default());
// `numbers`: a JSON array printed with every float rounded.
//
// `json` prints an f32 widened to f64 in full — `0.4` arrives as
// `0.4000000059604645`, which is correct and unreadable, and makes an export
// diff against a hand-authored file useless. `round` cannot help: it takes one
// number, and a template cannot map it over an array.
handlebars_helper!(numbers_helper: |v: Json, places: i64| {
    let m = 10f64.powi(places.clamp(0, 10) as i32);
    let snap = |f: f64| {
        let r = (f * m).round() / m;
        if r == 0.0 { 0.0 } else { r }
    };
    let parts: Vec<String> = v
        .as_array()
        .map(|a| {
            a.iter()
                .map(|x| match x.as_f64() {
                    Some(f) => serde_json::Number::from_f64(snap(f))
                        .map(|n| n.to_string())
                        .unwrap_or_else(|| "0".into()),
                    None => serde_json::to_string(x).unwrap_or_default(),
                })
                .collect()
        })
        .unwrap_or_default();
    format!("[{}]", parts.join(","))
});
handlebars_helper!(len_helper: |v: Json| match v {
    Value::Array(a) => a.len() as i64,
    Value::Object(o) => o.len() as i64,
    Value::String(s) => s.chars().count() as i64,
    _ => 0,
});
handlebars_helper!(hex_helper: |v: Json| {
    // RGBA floats to the 8-digit hex every runtime format writes colors in.
    let c = v.as_array().map(|a| {
        let get = |i: usize| a.get(i).and_then(|x| x.as_f64()).unwrap_or(1.0);
        [get(0), get(1), get(2), get(3)]
    }).unwrap_or([1.0; 4]);
    c.iter()
        .map(|f| format!("{:02x}", (f.clamp(0.0, 1.0) * 255.0).round() as u8))
        .collect::<String>()
});
// `or`: true when any argument is truthy. Handlebars ships no such helper, and
// a template separating optional channels with commas needs one.
handlebars_helper!(or_helper: |*args| args.iter().any(|v| match v {
    Value::Null => false,
    Value::Bool(b) => *b,
    Value::Number(n) => n.as_f64().is_some_and(|f| f != 0.0),
    Value::String(s) => !s.is_empty(),
    Value::Array(a) => !a.is_empty(),
    Value::Object(o) => !o.is_empty(),
}));

/// A configured engine.
pub struct Engine {
    registry: Handlebars<'static>,
}

impl Default for Engine {
    fn default() -> Self {
        Self::new()
    }
}

impl Engine {
    pub fn new() -> Self {
        let mut registry = Handlebars::new();
        registry.set_strict_mode(true);
        // Templates emit JSON, Lua, XML — never HTML. HTML-escaping would turn
        // every `&` and `"` in a name into an entity and corrupt the output.
        registry.register_escape_fn(handlebars::no_escape);
        registry.register_helper("deg", Box::new(deg));
        registry.register_helper("rad", Box::new(rad));
        registry.register_helper("round", Box::new(round));
        registry.register_helper("pad", Box::new(pad));
        registry.register_helper("eq", Box::new(eq_helper));
        registry.register_helper("ne", Box::new(ne_helper));
        registry.register_helper("add", Box::new(add_helper));
        registry.register_helper("sub", Box::new(sub_helper));
        registry.register_helper("mul", Box::new(mul_helper));
        registry.register_helper("div", Box::new(div_helper));
        registry.register_helper("or", Box::new(or_helper));
        registry.register_helper("json", Box::new(json_helper));
        registry.register_helper("numbers", Box::new(numbers_helper));
        registry.register_helper("len", Box::new(len_helper));
        registry.register_helper("hex", Box::new(hex_helper));
        Self { registry }
    }

    /// Render one template body against one context.
    pub fn render(&self, name: &str, body: &str, context: &Value) -> Result<String, TemplateError> {
        self.registry
            .render_template(body, context)
            .map_err(|e| classify(name, e))
    }
}

fn classify(name: &str, error: handlebars::RenderError) -> TemplateError {
    let reason = error.to_string();
    // A parse failure surfaces as a render error from `render_template`; the
    // distinction still matters to a user, who fixes them differently.
    if reason.contains("Template error") || reason.contains("invalid syntax") {
        TemplateError::Parse {
            template: name.to_string(),
            reason,
        }
    } else {
        TemplateError::Render {
            template: name.to_string(),
            reason,
        }
    }
}

/// Normalise a rendered output path and confine it to the output directory.
///
/// `output_path` is itself a template, so it can render to anything — including
/// `../../.bashrc` by way of a bone named `..`. Presets are meant to be shared
/// between studios and rigs arrive from other people, so both halves of that can
/// come from outside. Anything that climbs above the root is rejected rather
/// than clamped: a silently rewritten path writes a file the user did not ask
/// for, somewhere they did not look.
pub fn confine(template_name: &str, rendered: &str) -> Result<String, TemplateError> {
    let unified = rendered.replace('\\', "/");
    let mut parts: Vec<&str> = Vec::new();

    for segment in unified.split('/') {
        // A segment of only whitespace is not a usable filename, and treating it
        // as one produces a file nobody can find. `"  /  "` must be rejected as
        // empty, not accepted verbatim.
        match segment.trim() {
            "" | "." => continue,
            ".." => {
                if parts.pop().is_none() {
                    return Err(TemplateError::EscapingPath {
                        template: template_name.to_string(),
                        path: rendered.to_string(),
                    });
                }
            }
            other => {
                // An absolute path, a drive letter, or a UNC share all escape
                // the output directory just as surely as `..` does.
                if other.len() >= 2 && other.as_bytes()[1] == b':' && other.is_char_boundary(1) {
                    return Err(TemplateError::EscapingPath {
                        template: template_name.to_string(),
                        path: rendered.to_string(),
                    });
                }
                parts.push(other);
            }
        }
    }

    if unified.starts_with('/') {
        return Err(TemplateError::EscapingPath {
            template: template_name.to_string(),
            path: rendered.to_string(),
        });
    }
    if parts.is_empty() {
        return Err(TemplateError::EmptyPath {
            template: template_name.to_string(),
        });
    }
    Ok(parts.join("/"))
}
