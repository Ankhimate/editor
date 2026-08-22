//! The tools a model can call, and what they answer.
//!
//! **Deliberately coarse.** `docs/plugin-plan.md`: a faithful mirror of every
//! command is a worse tool surface than a small task-shaped one. Forty-nine
//! verbs as forty-nine tools would make a model spend its first three calls
//! discovering a vocabulary, and "move the bone a bit left" is not expressible
//! as a tool call anyway.
//!
//! So: open, describe, save, export — the shape of what someone actually asks
//! for — plus [`run_script`], which takes JavaScript over the whole verb
//! surface. Twenty lines of script beats twenty round trips, and the plugin host
//! already sandboxes it.
//!
//! # This is a tool list, not a transport
//!
//! Nothing here speaks JSON-RPC or knows what stdio is. A tool is a name, a
//! schema and a function from JSON to JSON, so the protocol layer above can be
//! replaced — or tested around — without touching any of this.

use crate::session::{Error, Session};
use serde_json::{Value, json};
use std::path::Path;

/// One callable tool.
pub struct Tool {
    pub name: &'static str,
    /// What it is for, in the terms a model will be reasoning in.
    pub description: &'static str,
    /// JSON Schema for the arguments.
    pub schema: Value,
}

/// Transport-free result. Protocol adapters decide how media is encoded.
#[derive(Debug)]
pub enum Output {
    Structured(Value),
    StructuredImage {
        structured: Value,
        png: Vec<u8>,
        width: u32,
        height: u32,
    },
    Image {
        png: Vec<u8>,
        width: u32,
        height: u32,
    },
}

impl Output {
    #[cfg(test)]
    fn structured(self) -> Value {
        match self {
            Self::Structured(value) => value,
            Self::StructuredImage { structured, .. } => structured,
            Self::Image { .. } => panic!("expected structured output"),
        }
    }
}

/// Every tool, in the order a session would use them.
pub fn all() -> Vec<Tool> {
    vec![
        Tool {
            name: "open_rig",
            description: "Open a rig through the shared importer registry. .ankh and PSD are \
                          built in; installed JavaScript plugins can add formats such as Spine \
                          and DragonBones. Returns a setup-pose preview and a compact inventory \
                          of image assets and attachment choices. Replaces whatever was open.",
            schema: json!({
                "type": "object",
                "required": ["path"],
                "properties": {
                    "path": { "type": "string", "description": "Absolute path to the file" }
                }
            }),
        },
        Tool {
            name: "new_rig",
            description: "Start an empty rig. Use this when building from a \
                          description rather than editing an existing file.",
            schema: json!({
                "type": "object",
                "required": ["name"],
                "properties": { "name": { "type": "string" } }
            }),
        },
        Tool {
            name: "describe_rig",
            description: "The open rig's full structure as JSON: bones, slots, \
                          skins, animations and their keys. This is the same \
                          context an export template sees, documented in \
                          docs/export-context.md.",
            schema: json!({ "type": "object", "properties": {} }),
        },
        Tool {
            name: "list_verbs",
            description: "Every verb `run_script` can invoke, with the arguments \
                          each takes. Call this before writing a script that \
                          uses anything beyond creating bones.",
            schema: json!({
                "type": "object",
                "properties": {
                    "prefix": {
                        "type": "string",
                        "description": "Only verbs starting with this — `bone.`, `anim.`"
                    }
                }
            }),
        },
        Tool {
            name: "run_script",
            description: "Run JavaScript against the open rig. `ops.invoke(verb, \
                          args)` performs an edit, `rig()` reads the structure, \
                          `names()` lists what exists, `console.log` reports \
                          back. Edits persist for the rest of the session. \
                          Prefer one script over many calls.",
            schema: json!({
                "type": "object",
                "required": ["script"],
                "properties": {
                    "script": { "type": "string", "description": "JavaScript source" }
                }
            }),
        },
        Tool {
            name: "save_rig",
            description: "Write the open rig to a path. Refuses to write over \
                          the file it was opened from: there is no undo here, so \
                          a mistake would leave nothing to go back to.",
            schema: json!({
                "type": "object",
                "required": ["path"],
                "properties": {
                    "path": { "type": "string", "description": "Where to write it" }
                }
            }),
        },
        Tool {
            name: "export_rig",
            description: "Export through a saved/built-in template preset or an installed \
                          JavaScript exporter. \
                          Paths rendered by the preset are confined to the output directory; \
                          the complete plan is rendered before writing and nothing is deleted.",
            schema: json!({
                "type": "object",
                "required": ["output_dir"],
                "properties": {
                    "output_dir": { "type": "string", "description": "Directory to write into" },
                    "preset": { "type": "string", "description": "Preset name, or plugin exporter id/label; defaults to Ankhimate runtime" }
                }
            }),
        },
        Tool {
            name: "render_frame",
            description: "Render the open rig at one animation time and return a PNG image. The optional focus filters only visuals; pose evaluation and the document are unchanged.",
            schema: render_schema(false),
        },
        Tool {
            name: "render_contact_sheet",
            description: "Render explicit or evenly spaced animation times into one labeled PNG. Every cell uses the same camera so motion and scale can be compared honestly.",
            schema: render_schema(true),
        },
    ]
}

fn render_schema(contact_sheet: bool) -> Value {
    let mut properties = serde_json::Map::from_iter([
        (
            "animation".into(),
            json!({ "type": "string", "description": "Animation name; omit for setup pose" }),
        ),
        (
            "width".into(),
            json!({ "type": "integer", "minimum": 1, "maximum": 8192, "default": if contact_sheet { 1024 } else { 512 } }),
        ),
        (
            "height".into(),
            json!({ "type": "integer", "minimum": 1, "maximum": 8192, "default": if contact_sheet { 768 } else { 512 } }),
        ),
        (
            "background".into(),
            json!({ "type": "array", "items": { "type": "integer", "minimum": 0, "maximum": 255 }, "minItems": 4, "maxItems": 4, "description": "RGBA bytes" }),
        ),
        (
            "camera".into(),
            json!({
                "type": "object",
                "properties": {
                    "center": { "type": "array", "items": { "type": "number" }, "minItems": 2, "maxItems": 2 },
                    "zoom": { "type": "number", "exclusiveMinimum": 0 },
                    "padding": { "type": "number", "minimum": 0, "maximum": 0.45, "default": 0.08 }
                },
                "description": "Omit center and zoom to fit automatically; provide both for fixed framing"
            }),
        ),
        (
            "focus".into(),
            json!({
                "type": "object",
                "properties": {
                    "bones": { "type": "array", "items": { "type": "string" } },
                    "include_descendants": { "type": "boolean", "default": false },
                    "mode": { "type": "string", "enum": ["dim", "isolate", "skeleton_only", "art_only"], "default": "dim" },
                    "other_opacity": { "type": "number", "minimum": 0, "maximum": 1, "default": 0.12 },
                    "show_bone_names": { "type": "boolean", "default": false },
                    "show_joint_points": { "type": "boolean", "default": false },
                    "show_constraint_targets": { "type": "boolean", "default": false },
                    "motion_trails": { "type": "array", "items": { "type": "string" } }
                }
            }),
        ),
    ]);
    if contact_sheet {
        properties.insert("times".into(), json!({ "type": "array", "items": { "type": "number" }, "description": "Explicit animation times in seconds" }));
        properties.insert("frame_count".into(), json!({ "type": "integer", "minimum": 1, "maximum": 100, "default": 6, "description": "Used only when times is empty" }));
        properties.insert("columns".into(), json!({ "type": "integer", "minimum": 1 }));
    } else {
        properties.insert("time".into(), json!({ "type": "number", "default": 0 }));
    }
    json!({ "type": "object", "properties": properties })
}

/// Call a tool by name.
pub fn call(session: &mut Session, name: &str, args: &Value) -> Result<Output, Error> {
    match name {
        "open_rig" => {
            let path = string_arg(args, "path")?;
            session.open_rig(Path::new(&path))?;
            let names = ankhimate_document::read::names(session.doc()?);
            let rendered = ankhimate_render::render_frame(
                session.doc()?,
                &ankhimate_render::FrameRequest::default(),
            )
            .map_err(|error| Error::Render(error.to_string()))?;
            Ok(Output::StructuredImage {
                structured: json!({
                    "opened": session.summary()?,
                    "preview": {
                        "pose": "setup",
                        "width": rendered.width,
                        "height": rendered.height,
                        "mime_type": "image/png",
                    },
                    "assets": {
                        "images": names.images,
                        "attachments": names.attachments,
                    },
                }),
                png: rendered.bytes,
                width: rendered.width,
                height: rendered.height,
            })
        }
        "new_rig" => {
            let name = string_arg(args, "name")?;
            session.new_rig(&name);
            Ok(Output::Structured(json!({ "created": session.summary()? })))
        }
        "describe_rig" => {
            let doc = session.doc()?;
            Ok(Output::Structured(ankhimate_document::read::describe(doc)))
        }
        "list_verbs" => {
            let prefix = args
                .get("prefix")
                .and_then(Value::as_str)
                .unwrap_or_default();
            Ok(Output::Structured(verb_list(prefix)))
        }
        "run_script" => {
            let script = string_arg(args, "script")?;
            let log = session.run_script(&script)?;
            // The summary comes back with the log because a model that just
            // edited a rig wants to know what it now has — and asking would be
            // a second round trip for something we already know.
            Ok(Output::Structured(
                json!({ "log": log, "rig": session.summary()? }),
            ))
        }
        "save_rig" => {
            let path = string_arg(args, "path")?;
            session.save_rig(Path::new(&path))?;
            Ok(Output::Structured(json!({ "written": path })))
        }
        "export_rig" => {
            let output_dir = string_arg(args, "output_dir")?;
            let preset = args.get("preset").and_then(Value::as_str);
            let survey = session.export_rig(Path::new(&output_dir), preset)?;
            Ok(Output::Structured(json!({
                "output_dir": output_dir,
                "created": survey.created,
                "replaced": survey.replaced,
                "orphans": survey.orphans,
            })))
        }
        "render_frame" => {
            let request: ankhimate_render::FrameRequest = serde_json::from_value(args.clone())
                .map_err(|error| {
                    Error::Refused(format!("invalid render_frame arguments: {error}"))
                })?;
            let rendered = ankhimate_render::render_frame(session.doc()?, &request)
                .map_err(|error| Error::Render(error.to_string()))?;
            Ok(Output::Image {
                png: rendered.bytes,
                width: rendered.width,
                height: rendered.height,
            })
        }
        "render_contact_sheet" => {
            let request: ankhimate_render::ContactSheetRequest =
                serde_json::from_value(args.clone()).map_err(|error| {
                    Error::Refused(format!("invalid render_contact_sheet arguments: {error}"))
                })?;
            let rendered = ankhimate_render::render_contact_sheet(session.doc()?, &request)
                .map_err(|error| Error::Render(error.to_string()))?;
            Ok(Output::Image {
                png: rendered.bytes,
                width: rendered.width,
                height: rendered.height,
            })
        }
        _ => Err(Error::Refused(format!(
            "no tool named `{name}`. Available: {}",
            all().iter().map(|t| t.name).collect::<Vec<_>>().join(", ")
        ))),
    }
}

/// Every verb and its schema, for `list_verbs`.
///
/// Built from the same registry the editor's menus and a plugin resolve
/// against, so a verb added anywhere appears here without being listed twice.
fn verb_list(prefix: &str) -> Value {
    let ops = ankhimate_document::DocOps::builtin();
    let verbs: Vec<Value> = ops
        .ids()
        .filter(|id| id.starts_with(prefix))
        .filter_map(|id| {
            let op = ops.get(id)?;
            Some(json!({
                "verb": id,
                "label": op.label(),
                "arguments": op.schema(),
            }))
        })
        .collect();
    json!({ "verbs": verbs })
}

fn string_arg(args: &Value, key: &str) -> Result<String, Error> {
    args.get(key)
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| Error::Refused(format!("`{key}` is required and must be a string")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call_ok(session: &mut Session, name: &str, args: Value) -> Value {
        call(session, name, &args)
            .unwrap_or_else(|e| panic!("`{name}` failed: {e}"))
            .structured()
    }

    #[test]
    fn the_tool_list_is_small_enough_to_read() {
        // The whole design decision, pinned. Forty-nine verbs as forty-nine
        // tools would make a model spend its first calls discovering a
        // vocabulary rather than doing the work — so if this list starts
        // growing towards the verb count, something has gone wrong.
        let tools = all();
        assert!(
            tools.len() <= 10,
            "the tool surface is meant to be coarse, and this is {} tools",
            tools.len()
        );
        assert!(
            tools.iter().any(|t| t.name == "run_script"),
            "the escape hatch the coarse list depends on"
        );
    }

    #[test]
    fn every_tool_describes_itself_and_its_arguments() {
        // A tool a model cannot understand without reading our source is one it
        // will call wrongly and then guess about.
        for tool in all() {
            assert!(!tool.description.is_empty(), "`{}`", tool.name);
            assert_eq!(
                tool.schema.get("type").and_then(Value::as_str),
                Some("object"),
                "`{}` has no argument schema",
                tool.name
            );
        }
    }

    #[test]
    fn a_rig_can_be_built_described_and_saved() {
        // The whole loop, in the calls a model would actually make.
        let dir = tempfile::tempdir().unwrap();
        let mut session = Session::new();

        call_ok(&mut session, "new_rig", json!({ "name": "hero" }));
        let result = call_ok(
            &mut session,
            "run_script",
            json!({
                "script": r#"
                ops.invoke("bone.create", { name: "root" });
                ops.invoke("bone.create", { name: "spine", parent: "root", length: 40 });
                console.log("built " + names().bones.length + " bones");
                "#
            }),
        );
        assert_eq!(result["log"][0], "built 2 bones");
        assert!(
            result["rig"].as_str().unwrap().contains("2 bones"),
            "the summary comes back with the log, so a model does not have to ask"
        );

        let described = call_ok(&mut session, "describe_rig", json!({}));
        assert!(
            described["skeleton"]["bones"].as_array().unwrap().len() == 2,
            "describe reads the same context an export template does"
        );

        let path = dir.path().join("hero.ankh");
        let written = call_ok(
            &mut session,
            "save_rig",
            json!({ "path": path.to_str().unwrap() }),
        );
        assert!(written["written"].as_str().is_some());
        assert!(path.exists());
    }

    #[test]
    fn list_verbs_answers_from_the_real_registry() {
        // Not a hardcoded list: a verb added to `document` appears here without
        // being written down twice, which is the whole reason MCP is a consumer
        // of the plugin API rather than a second road.
        let mut session = Session::new();
        let all_verbs = call_ok(&mut session, "list_verbs", json!({}));
        let count = all_verbs["verbs"].as_array().unwrap().len();
        assert!(count >= 49, "the verb surface shrank: {count}");

        let bones = call_ok(&mut session, "list_verbs", json!({ "prefix": "bone." }));
        let listed = bones["verbs"].as_array().unwrap();
        assert!(!listed.is_empty());
        assert!(
            listed
                .iter()
                .all(|v| v["verb"].as_str().unwrap().starts_with("bone.")),
            "the prefix filtered"
        );
        assert!(
            listed[0]["arguments"].get("properties").is_some(),
            "and each one says what arguments it takes"
        );
    }

    #[test]
    fn an_unknown_tool_lists_the_ones_that_exist() {
        // A model that guessed a name should get the list rather than a refusal
        // it has to guess again from.
        let mut session = Session::new();
        let error = call(&mut session, "delete_everything", &json!({}))
            .unwrap_err()
            .to_string();
        assert!(error.contains("run_script"), "{error}");
    }

    #[test]
    fn a_missing_argument_says_which_one() {
        let mut session = Session::new();
        let error = call(&mut session, "open_rig", &json!({}))
            .unwrap_err()
            .to_string();
        assert!(error.contains("path"), "{error}");
    }

    #[test]
    fn saving_over_the_source_is_refused_through_the_tool_too() {
        // The rule is kept in `session`, and this checks it is not bypassed by
        // the layer above it — which is where a rule usually goes missing.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hero.ankh");
        let as_str = path.to_str().unwrap().to_string();

        let mut session = Session::new();
        call_ok(&mut session, "new_rig", json!({ "name": "hero" }));
        call_ok(&mut session, "save_rig", json!({ "path": &as_str }));

        let mut reopened = Session::new();
        call_ok(&mut reopened, "open_rig", json!({ "path": &as_str }));
        let error = call(&mut reopened, "save_rig", &json!({ "path": &as_str }))
            .unwrap_err()
            .to_string();
        assert!(error.contains("opened from"), "{error}");
    }

    #[test]
    fn a_rig_exports_through_the_reference_preset() {
        let dir = tempfile::tempdir().unwrap();
        let mut session = Session::new();
        call_ok(&mut session, "new_rig", json!({ "name": "hero" }));
        call_ok(
            &mut session,
            "run_script",
            json!({ "script": "ops.invoke('bone.create', { name: 'root' });" }),
        );

        let result = call_ok(
            &mut session,
            "export_rig",
            json!({ "output_dir": dir.path().to_str().unwrap() }),
        );
        assert!(!result["created"].as_array().unwrap().is_empty());
        assert!(dir.path().read_dir().unwrap().next().is_some());
    }

    #[test]
    fn render_tools_return_png_media_without_writing_a_file() {
        let mut session = Session::new();
        call_ok(&mut session, "new_rig", json!({ "name": "preview" }));
        let output = call(
            &mut session,
            "render_frame",
            &json!({ "width": 64, "height": 48 }),
        )
        .unwrap();
        match output {
            Output::Image { png, width, height } => {
                assert_eq!((width, height), (64, 48));
                assert!(png.starts_with(b"\x89PNG\r\n\x1a\n"));
                assert_eq!(image_dimensions(&png), (64, 48));
            }
            Output::Structured(_) | Output::StructuredImage { .. } => {
                panic!("render_frame returned JSON instead of an image")
            }
        }
    }

    #[test]
    fn opening_a_rig_returns_setup_preview_and_attachment_choices() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hands.ankh");
        let mut built = Session::new();
        call_ok(&mut built, "new_rig", json!({ "name": "hands" }));
        call_ok(
            &mut built,
            "run_script",
            json!({ "script": r#"
                ops.invoke("bone.create", { name: "root" });
                ops.invoke("slot.create", { name: "hand", bone: "root" });
                ops.invoke("attachment.create_point", { slot: "hand", name: "closed" });
                ops.invoke("attachment.create_point", { slot: "hand", name: "open" });
                ops.invoke("slot.set_attachment", { slot: "hand", attachment: "closed" });
            "# }),
        );
        call_ok(
            &mut built,
            "save_rig",
            json!({ "path": path.to_str().unwrap() }),
        );

        let mut opened = Session::new();
        let output = call(
            &mut opened,
            "open_rig",
            &json!({ "path": path.to_str().unwrap() }),
        )
        .unwrap();
        let Output::StructuredImage {
            structured,
            png,
            width,
            height,
        } = output
        else {
            panic!("open_rig did not return structured data with a preview");
        };
        assert_eq!((width, height), (512, 512));
        assert!(png.starts_with(b"\x89PNG\r\n\x1a\n"));
        assert_eq!(structured["preview"]["pose"], "setup");
        assert_eq!(structured["assets"]["images"], json!([]));
        assert_eq!(
            structured["assets"]["attachments"],
            json!([{
                "slot": "hand",
                "current": "closed",
                "available": ["closed", "open"],
            }])
        );
    }

    fn image_dimensions(png: &[u8]) -> (u32, u32) {
        (
            u32::from_be_bytes(png[16..20].try_into().unwrap()),
            u32::from_be_bytes(png[20..24].try_into().unwrap()),
        )
    }
}
