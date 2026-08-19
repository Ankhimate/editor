//! Plugins in JavaScript, over the document API.
//!
//! A plugin is a `.js` file that calls the same verbs a menu does. It adds no
//! vocabulary: `ops.invoke("bone.create", {...})` reaches the operator the
//! editor's File menu reaches, so `docs/plugin-api.md` documents both.
//!
//! # Why QuickJS
//!
//! Small, embeds without a build system, and sandboxed by construction — a
//! `Context` has no filesystem, no network and no clock unless something is
//! bound into it, and nothing here binds any of the three.
//!
//! No JIT, which does not matter: a plugin runs on a gesture or an import, never
//! inside `evaluate()`. That boundary is the one thing this crate must not
//! cross, because crossing it would put arbitrary script in the hot loop and
//! take determinism (PLAN §2.6) with it.
//!
//! Rejected: `deno_core`/V8 — a large dependency that complicates the `wasm32`
//! target for a JIT nothing here can use. Boa — slower and spec-incomplete in
//! corners a plugin author would find.
//!
//! # What a plugin can and cannot do
//!
//! It can list verbs, read the rig, and invoke verbs. It cannot mutate the
//! document directly: there is no binding that hands out a document, so every
//! change goes through a command and stays undoable. That is `CLAUDE.md`'s rule
//! for panels, extended to script.

pub mod importer;

use ankhimate_document::{Args, DocOps, Edit};
use rquickjs::{Context, Function, Object, Runtime};

/// What went wrong running a plugin.
#[derive(Debug)]
pub enum PluginError {
    /// The engine could not be created.
    Engine(String),
    /// The script failed to parse or threw.
    ///
    /// Carries the message QuickJS produced, which names the line — a plugin
    /// author with "an error occurred" has nothing to work with.
    Script(String),
}

impl std::fmt::Display for PluginError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PluginError::Engine(why) => write!(f, "the script engine failed: {why}"),
            PluginError::Script(why) => write!(f, "{why}"),
        }
    }
}

impl std::error::Error for PluginError {}

/// One run of one script against one document.
///
/// Deliberately short-lived. A host that kept a `Runtime` alive across edits
/// would have to answer what a plugin's captured references mean after an undo,
/// and the answer is nothing good — so a run borrows the document, does its
/// work, and hands it back.
pub struct Host {
    ops: DocOps,
    /// Files a script may open: those beside the one being imported, or none.
    ///
    /// Absent for an ordinary plugin run. An importer is the only thing that
    /// needs to read at all, and it needs its own directory rather than a
    /// filesystem — see [`importer::Sidecars`].
    sidecars: Option<importer::Sidecars>,
}

impl Default for Host {
    fn default() -> Self {
        Self::new()
    }
}

impl Host {
    pub fn new() -> Self {
        Self {
            ops: DocOps::builtin(),
            sidecars: None,
        }
    }

    /// Let this run open the files beside an imported one.
    pub fn with_sidecars(mut self, sidecars: importer::Sidecars) -> Self {
        self.sidecars = Some(sidecars);
        self
    }

    /// Read the importers a script registers, without running an import.
    ///
    /// What a host does at load time: run the file, collect what it declared,
    /// and put those in the File▸Import menu.
    pub fn importers(&self, script: &str) -> Result<Vec<importer::JsImporter>, PluginError> {
        let mut edit = Edit::default();
        let declared = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        self.run_inner(script, &mut edit, Some(declared.clone()))?;
        let found = declared.borrow().clone();
        Ok(found
            .into_iter()
            .map(|(id, label, extensions)| importer::JsImporter {
                id,
                label,
                extensions,
                source: script.to_string(),
            })
            .collect())
    }

    /// Run `script` against `edit`.
    ///
    /// Anything the script printed comes back as lines, since a plugin author
    /// debugging without `console.log` is a plugin author writing blind.
    pub fn run(&self, script: &str, edit: &mut Edit) -> Result<Vec<String>, PluginError> {
        self.run_inner(script, edit, None)
    }

    fn run_inner(
        &self,
        script: &str,
        edit: &mut Edit,
        declared: Option<std::rc::Rc<std::cell::RefCell<Vec<(String, String, Vec<String>)>>>>,
    ) -> Result<Vec<String>, PluginError> {
        let runtime = Runtime::new().map_err(|e| PluginError::Engine(e.to_string()))?;
        let context = Context::full(&runtime).map_err(|e| PluginError::Engine(e.to_string()))?;

        // The document is *moved* in for the duration of the call and moved
        // back out after. `rquickjs` requires its bindings to be `'static`, so a
        // borrow cannot cross into them — and an `Rc<RefCell<_>>` is the honest
        // way to say "one owner, one thread, shared between closures" rather
        // than a lock implying contention that cannot happen.
        let taken = std::mem::take(edit);
        let cell = std::rc::Rc::new(std::cell::RefCell::new(taken));
        let printed = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        // The thrown message, captured inside the `with` where the exception
        // still exists. Read after, it is already gone and every failure comes
        // back as the useless "Exception generated by QuickJS".
        let thrown = std::rc::Rc::new(std::cell::RefCell::new(None::<String>));
        let thrown_sink = thrown.clone();

        let outcome = context.with(|ctx| -> rquickjs::Result<()> {
            let thrown = thrown_sink;
            let global = ctx.globals();

            // ── console.log ──────────────────────────────────────────────
            let console = Object::new(ctx.clone())?;
            let sink = printed.clone();
            console.set(
                "log",
                Function::new(ctx.clone(), move |msg: rquickjs::Value| {
                    let text = match msg.as_string() {
                        Some(s) => s.to_string().unwrap_or_default(),
                        None => format!("{msg:?}"),
                    };
                    sink.borrow_mut().push(text);
                })?,
            )?;
            global.set("console", console)?;

            // ── ops ──────────────────────────────────────────────────────
            let ops_obj = Object::new(ctx.clone())?;

            // ops.list() — every verb, so a plugin can discover rather than
            // hardcode. Same list the editor's menus resolve against.
            let ids: Vec<String> = self.ops.ids().map(str::to_string).collect();
            ops_obj.set("list", Function::new(ctx.clone(), move || ids.clone())?)?;

            // ops.schema(id) — what a verb takes, as the JSON Schema the
            // operator declares. What an author reads instead of the source.
            let schemas: std::collections::BTreeMap<String, String> = self
                .ops
                .ids()
                .filter_map(|id| {
                    let op = self.ops.get(id)?;
                    Some((id.to_string(), op.schema().to_string()))
                })
                .collect();
            ops_obj.set(
                "schema",
                Function::new(ctx.clone(), move |id: String| {
                    schemas.get(&id).cloned().unwrap_or_default()
                })?,
            )?;

            // ops.invoke(id, args) — the whole write surface.
            //
            // Arguments cross as a JSON string rather than as a JS object:
            // `Args` is a `serde_json::Value` either way, and going through the
            // text avoids a second conversion path that could disagree with the
            // one every other caller uses.
            let cell_ref = cell.clone();
            // Cloned rather than borrowed for the same `'static` reason. `DocOps`
            // is a table of stateless descriptors, so this is cheap.
            let ops_ref = std::rc::Rc::new(DocOps::builtin());
            ops_obj.set(
                "invokeJson",
                Function::new(
                    ctx.clone(),
                    move |ctx: rquickjs::Ctx<'_>,
                          id: String,
                          args_json: String|
                          -> rquickjs::Result<()> {
                        let value: serde_json::Value =
                            serde_json::from_str(&args_json).unwrap_or(serde_json::Value::Null);
                        let mut borrowed = cell_ref.borrow_mut();
                        ops_ref
                            .invoke(&id, &mut borrowed, &Args::from_json(value))
                            .map_err(|e| {
                                // A refused edit or a bad argument becomes a JS
                                // exception, so a plugin can catch it and a
                                // failure is not silently a no-op.
                                //
                                // Thrown as an `Error` rather than a bare
                                // string: a thrown string has no `.message`, so
                                // the host reads nothing back out and every
                                // failure reaches the user as "Exception
                                // generated by QuickJS".
                                let exception =
                                    rquickjs::Exception::from_message(ctx.clone(), &e.to_string());
                                match exception {
                                    Ok(ex) => ctx.throw(ex.into_value()),
                                    Err(inner) => inner,
                                }
                            })
                    },
                )?,
            )?;

            // rig() — the read surface, as the template context.
            let cell_read = cell.clone();
            ops_obj.set(
                "describeJson",
                Function::new(ctx.clone(), move || {
                    let borrowed = cell_read.borrow();
                    ankhimate_document::describe(&borrowed.doc).to_string()
                })?,
            )?;

            // names() — the cheap question that precedes every edit.
            let cell_names = cell.clone();
            ops_obj.set(
                "namesJson",
                Function::new(ctx.clone(), move || {
                    let borrowed = cell_names.borrow();
                    serde_json::to_string(&ankhimate_document::names(&borrowed.doc))
                        .unwrap_or_default()
                })?,
            )?;

            global.set("__ops", ops_obj)?;

            // ── ankhimate.registerImporter / sidecars ────────────────────
            let ank = Object::new(ctx.clone())?;

            // What a script declares is collected rather than acted on: the
            // host decides whether this run is a load (collect) or an import
            // (run the body), so one plugin file serves both.
            let sink = declared.clone();
            ank.set(
                "declareImporter",
                Function::new(
                    ctx.clone(),
                    move |id: String, label: String, extensions: Vec<String>| {
                        if let Some(sink) = &sink {
                            sink.borrow_mut().push((id, label, extensions));
                        }
                    },
                )?,
            )?;

            // Files beside the imported one, and nothing else. Absent for an
            // ordinary run, so a plugin that is not an importer cannot read at
            // all.
            let sidecar_read = self.sidecars.as_ref().map(|s| s.clone_dir());
            ank.set(
                "sidecar",
                Function::new(ctx.clone(), move |name: String| {
                    sidecar_read
                        .as_ref()
                        .and_then(|dir| importer::Sidecars::new(dir.clone()).read(&name))
                })?,
            )?;

            // An atlas page is a PNG; a plugin needs its bytes to hand to
            // `asset.add_image`, and text-only sidecars are what blocked an
            // importer from bringing artwork across at all.
            let sidecar_bytes = self.sidecars.as_ref().map(|s| s.clone_dir());
            ank.set(
                "sidecarBytes",
                Function::new(ctx.clone(), move |name: String| {
                    sidecar_bytes
                        .as_ref()
                        .and_then(|dir| importer::Sidecars::new(dir.clone()).read_bytes(&name))
                })?,
            )?;

            let sidecar_list = self.sidecars.as_ref().map(|s| s.clone_dir());
            ank.set(
                "sidecars",
                Function::new(ctx.clone(), move || {
                    sidecar_list
                        .as_ref()
                        .map(|dir| importer::Sidecars::new(dir.clone()).list())
                        .unwrap_or_default()
                })?,
            )?;

            global.set("__ankhimate", ank)?;

            // A thin JS shim so a plugin writes objects rather than JSON text.
            // In script rather than Rust because that is where it belongs: it
            // is sugar over the binding, not part of it.
            ctx.eval::<(), _>(
                r#"
                globalThis.ops = {
                  list: () => __ops.list(),
                  schema: (id) => JSON.parse(__ops.schema(id) || "null"),
                  invoke: (id, args) => __ops.invokeJson(id, JSON.stringify(args ?? {})),
                };
                globalThis.rig = () => JSON.parse(__ops.describeJson());
                globalThis.names = () => JSON.parse(__ops.namesJson());

                // An importer registers itself; the host decides whether this
                // run collects declarations or performs a read. Keeping the
                // body here means one plugin file rather than a registration
                // and a separate reader to keep in step.
                let __importers = {};
                globalThis.ankhimate = {
                  registerImporter(spec) {
                    __importers[spec.id] = spec;
                    __ankhimate.declareImporter(
                      spec.id, spec.label ?? spec.id, spec.extensions ?? []);
                  },
                  sidecar: (name) => __ankhimate.sidecar(name),
                  sidecarBytes: (name) => __ankhimate.sidecarBytes(name),
                  sidecars: () => __ankhimate.sidecars(),
                };
                globalThis.__ankhimate_run_import = (text, fileName) => {
                  const ids = Object.keys(__importers);
                  if (!ids.length) throw new Error("this plugin registered no importer");
                  __importers[ids[0]].read(text, fileName);
                };
                "#,
            )?;

            // The message has to be fetched *here*, inside the same `with`:
            // the exception lives on the context and is gone by the time the
            // outer code sees an error value, which is how every failure came
            // back as the useless "Exception generated by QuickJS".
            let outcome = ctx.eval::<(), _>(script);
            if outcome.is_err()
                && let Some(ex) = ctx.catch().as_exception()
            {
                *thrown.borrow_mut() = Some(ex.message().unwrap_or_else(|| ex.to_string()));
            }
            outcome
        });

        // Whatever the script did or failed to do, the document comes back.
        //
        // Taken out of the cell rather than unwrapped from the `Rc`: the
        // context still holds clones at this point, so `try_unwrap` fails and
        // `unwrap_or_default` would quietly hand back an empty rig — which is
        // exactly what it did, and every write looked like a no-op.
        *edit = std::mem::take(&mut *cell.borrow_mut());
        let printed = printed.borrow().clone();

        match outcome {
            Ok(()) => Ok(printed),
            Err(e) => Err(PluginError::Script(
                thrown.borrow_mut().take().unwrap_or_else(|| e.to_string()),
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_plugin_builds_a_rig_through_the_same_verbs_a_menu_uses() {
        let host = Host::new();
        let mut edit = Edit::default();

        let printed = host
            .run(
                r#"
                ops.invoke("bone.create", { name: "root" });
                ops.invoke("bone.create", { name: "spine", parent: "root", y: 40 });
                console.log("built " + names().bones.length + " bones");
                "#,
                &mut edit,
            )
            .expect("the script runs");

        assert_eq!(edit.doc.skeleton.bones.len(), 2);
        assert!(
            edit.doc.skeleton.bones.values().any(|b| b.name == "spine"),
            "named by the script"
        );
        assert_eq!(printed, ["built 2 bones"]);
    }

    #[test]
    fn a_plugins_edits_are_undoable() {
        // The property that makes a plugin safe to run: it went through
        // commands, so it comes back out the same way.
        let host = Host::new();
        let mut edit = Edit::default();
        host.run(r#"ops.invoke("bone.create", { name: "root" });"#, &mut edit)
            .expect("runs");

        assert_eq!(edit.doc.skeleton.bones.len(), 1);
        assert!(edit.undo());
        assert_eq!(edit.doc.skeleton.bones.len(), 0);
    }

    #[test]
    fn a_bad_argument_throws_rather_than_doing_nothing() {
        // Silence would be the worst answer: a script naming a bone the rig
        // does not have has a bug, and it should stop rather than continue over
        // an edit that did not happen.
        let host = Host::new();
        let mut edit = Edit::default();

        let err = host
            .run(
                r#"ops.invoke("bone.create", { name: "hand", parent: "nope" });"#,
                &mut edit,
            )
            .expect_err("an unresolvable parent throws");

        assert!(format!("{err}").contains("nope"), "{err}");
        assert_eq!(edit.doc.skeleton.bones.len(), 0, "nothing was created");
    }

    #[test]
    fn a_plugin_can_catch_what_it_throws() {
        // The other half: a plugin that expects a verb to fail should be able to
        // handle it rather than being killed by it.
        let host = Host::new();
        let mut edit = Edit::default();

        let printed = host
            .run(
                r#"
                try {
                  ops.invoke("bone.create", { name: "hand", parent: "nope" });
                  console.log("no throw");
                } catch (e) {
                  console.log("caught");
                }
                "#,
                &mut edit,
            )
            .expect("the script survives");

        assert_eq!(printed, ["caught"]);
    }

    #[test]
    fn a_plugin_discovers_verbs_rather_than_hardcoding_them() {
        // What makes a plugin survive a new build: it asks what exists.
        let host = Host::new();
        let mut edit = Edit::default();

        let printed = host
            .run(
                r#"
                const ids = ops.list();
                console.log(ids.includes("bone.create") ? "found" : "missing");
                console.log(ops.schema("bone.create").required[0]);
                "#,
                &mut edit,
            )
            .expect("runs");

        assert_eq!(printed, ["found", "name"]);
    }

    #[test]
    fn a_plugin_reads_the_rig_in_the_documented_shape() {
        // `rig()` is the template context, so an exporter author already knows
        // these field names — one contract, not two.
        let host = Host::new();
        let mut edit = Edit::default();

        let printed = host
            .run(
                r#"
                ops.invoke("bone.create", { name: "arm", rotation: 90 });
                const r = rig();
                console.log("v" + r.context_version);
                console.log(String(r.skeleton.bones[0].rotation));
                "#,
                &mut edit,
            )
            .expect("runs");

        assert_eq!(printed[0], "v1");
        let rotation: f64 = printed[1].parse().expect("a number");
        assert!(
            (rotation - 90.0).abs() < 1e-3,
            "degrees, as the contract says: {rotation}"
        );
    }

    #[test]
    fn a_syntax_error_names_itself() {
        let host = Host::new();
        let mut edit = Edit::default();
        let err = host
            .run("this is not javascript", &mut edit)
            .expect_err("does not run");
        assert!(!format!("{err}").is_empty());
    }

    #[test]
    fn a_plugin_has_no_filesystem() {
        // Nothing binds one, and the test says so rather than trusting that
        // nobody adds one later. A plugin that could read the disk is a
        // different security question than the one this crate answers.
        let host = Host::new();
        let mut edit = Edit::default();

        let printed = host
            .run(
                r#"
                console.log(typeof require === "undefined" ? "no require" : "require!");
                console.log(typeof process === "undefined" ? "no process" : "process!");
                "#,
                &mut edit,
            )
            .expect("runs");

        assert_eq!(printed, ["no require", "no process"]);
    }
}
