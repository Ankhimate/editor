//! Running an export (T-603b).
//!
//! Rendering is pure; writing is not. This module is where a bug destroys work
//! that was never ours to lose, so it holds three rules:
//!
//! 1. **Confine every path** to the output directory. See [`crate::template::confine`].
//! 2. **All or nothing.** Render every file to memory first, write only once all
//!    of them succeed. A template that fails on the ninth animation must not
//!    leave eight files from this run beside four from the last — a state no
//!    user can reason about and no runtime can load.
//! 3. **Never delete.** A stale-file sweep is the obvious convenience and it is
//!    refused: `output_dir` is user-chosen and can be pointed at a source tree
//!    by accident. Orphans are reported, and the user decides.

use crate::atlas::{self, Atlas};
use crate::context::{Context, ExportInfo};
use crate::preset::{Cadence, Preset};
use crate::template::{Engine, RenderedFile, TemplateError, confine};
use ankhimate_core::assets::AssetDb;
use ankhimate_formats::schema::Project;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

#[derive(Debug)]
pub enum ExportError {
    Atlas(crate::atlas::AtlasError),
    Template(TemplateError),
    /// Two templates rendered to the same path — the later would silently
    /// overwrite the earlier, so the export stops instead.
    DuplicatePath {
        path: String,
        first: String,
        second: String,
    },
    Io {
        path: String,
        reason: String,
    },
}

impl std::fmt::Display for ExportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ExportError::Atlas(e) => write!(f, "{e}"),
            ExportError::Template(e) => write!(f, "{e}"),
            ExportError::DuplicatePath {
                path,
                first,
                second,
            } => write!(
                f,
                "templates '{first}' and '{second}' both write '{path}'; one would overwrite the \
                 other, so nothing was written"
            ),
            ExportError::Io { path, reason } => write!(f, "could not write '{path}': {reason}"),
        }
    }
}

impl std::error::Error for ExportError {}

impl From<TemplateError> for ExportError {
    fn from(e: TemplateError) -> Self {
        ExportError::Template(e)
    }
}

/// Everything an export produced, before any of it touches the disk.
///
/// Holding the whole set in memory is what makes rule 2 possible, and it is
/// affordable: these are text files and a handful of atlas pages.
#[derive(Debug, Default)]
pub struct Plan {
    pub files: Vec<RenderedFile>,
    /// `(relative path, PNG bytes)`.
    pub binaries: Vec<(String, Vec<u8>)>,
}

impl Plan {
    /// Every path this plan writes, in order.
    pub fn paths(&self) -> Vec<&str> {
        self.files
            .iter()
            .map(|f| f.path.as_str())
            .chain(self.binaries.iter().map(|(p, _)| p.as_str()))
            .collect()
    }

    /// Split the plan's paths into those that already exist and those that do
    /// not, so a user can be told what an export will replace before it does.
    pub fn survey(&self, output_dir: &Path) -> Survey {
        let mut created = Vec::new();
        let mut replaced = Vec::new();
        for path in self.paths() {
            if output_dir.join(path).exists() {
                replaced.push(path.to_string());
            } else {
                created.push(path.to_string());
            }
        }
        Survey {
            created,
            replaced,
            orphans: orphans(output_dir, &self.paths()),
        }
    }
}

/// What an export is about to do to a directory.
#[derive(Debug, Default, PartialEq)]
pub struct Survey {
    pub created: Vec<String>,
    pub replaced: Vec<String>,
    /// Files already in the output directory that this export does not write.
    ///
    /// Reported, never deleted — see the module note. A renamed animation leaves
    /// its old file behind, and only the user knows whether that matters.
    pub orphans: Vec<String>,
}

fn orphans(output_dir: &Path, writing: &[&str]) -> Vec<String> {
    let expected: BTreeSet<String> = writing.iter().map(|p| p.to_string()).collect();
    let mut found = Vec::new();
    walk(output_dir, output_dir, &mut found);
    found
        .into_iter()
        .filter(|p| !expected.contains(p))
        .collect()
}

fn walk(root: &Path, dir: &Path, out: &mut Vec<String>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut paths: Vec<PathBuf> = entries.filter_map(|e| e.ok()).map(|e| e.path()).collect();
    paths.sort();
    for path in paths {
        if path.is_dir() {
            walk(root, &path, out);
        } else if let Ok(rel) = path.strip_prefix(root) {
            out.push(rel.to_string_lossy().replace('\\', "/"));
        }
    }
}

/// Render a preset into a [`Plan`] without writing anything.
///
/// This is also what the editor's live preview calls, so a preview is the real
/// export rather than an approximation of it — a preview that can disagree with
/// the export is worse than none.
pub fn plan(project: &Project, assets: &AssetDb, preset: &Preset) -> Result<Plan, ExportError> {
    plan_with(project, assets, preset, Images::Encoded)
}

/// Whether a plan carries its atlas pages as encoded PNG bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Images {
    /// Encode every page. Required before [`write`] — a plan without bytes
    /// writes no images.
    Encoded,
    /// Name the pages but leave their bytes empty.
    ///
    /// For a caller that only displays the plan. PNG encoding a 2048² page costs
    /// tens of milliseconds, and the editor's preview does it only to show a
    /// count and then drop the bytes. The atlas is still **baked** — regions and
    /// UVs reach the context, so the rendered text is byte-identical to a real
    /// export. Only the compression is skipped.
    Named,
}

/// [`plan`], with control over whether atlas pages are encoded.
pub fn plan_with(
    project: &Project,
    assets: &AssetDb,
    preset: &Preset,
    images: Images,
) -> Result<Plan, ExportError> {
    let baked: Option<Atlas> = if preset.atlas.enabled {
        Some(atlas::bake(assets, &preset.atlas.settings).map_err(ExportError::Atlas)?)
    } else {
        None
    };

    let engine = Engine::new();
    let mut plan = Plan::default();
    // Path -> the template that claimed it, so a collision names both.
    let mut claimed: Vec<(String, String)> = Vec::new();

    for template in &preset.templates {
        let info = ExportInfo {
            output_dir: preset.output_dir.clone(),
            preset_name: preset.name.clone(),
            template_name: template.name.clone(),
            atlas_stem: preset.atlas.stem.clone(),
        };
        let context = Context::build(project, baked.as_ref(), info);

        let renders: Vec<serde_json::Value> = match template.per {
            Cadence::Once => vec![context.to_value()],
            Cadence::Animation => (0..context.animations.len())
                .map(|i| context.with_animation(i))
                .collect(),
        };

        for value in renders {
            let path_name = format!("{} (output path)", template.name);
            let rendered_path = engine.render(&path_name, &template.output_path, &value)?;
            let path = confine(&template.name, &rendered_path)?;

            if let Some((_, first)) = claimed.iter().find(|(p, _)| p == &path) {
                return Err(ExportError::DuplicatePath {
                    path,
                    first: first.clone(),
                    second: template.name.clone(),
                });
            }
            claimed.push((path.clone(), template.name.clone()));

            let contents = engine.render(&template.name, &template.body, &value)?;
            plan.files.push(RenderedFile { path, contents });
        }
    }

    if let Some(atlas) = &baked {
        for page in &atlas.pages {
            let name = atlas::page_filename(&preset.atlas.stem, page.index);
            let bytes = match images {
                Images::Encoded => atlas::encode_png(page).map_err(|e| ExportError::Io {
                    path: name.clone(),
                    reason: e.to_string(),
                })?,
                Images::Named => Vec::new(),
            };
            plan.binaries.push((name, bytes));
        }
    }

    if preset.copy_images {
        let mut names: Vec<&str> = assets.images.iter().map(|(_, a)| a.name.as_str()).collect();
        names.sort_unstable();
        for name in names {
            if let Some(id) = assets.by_name(name) {
                let asset = &assets.images[id];
                // Already-encoded source bytes, so `Named` saves only the clone.
                let bytes = match images {
                    Images::Encoded => asset.bytes.clone(),
                    Images::Named => Vec::new(),
                };
                plan.binaries
                    .push((format!("images/{}.png", asset.name), bytes));
            }
        }
    }

    Ok(plan)
}

/// Write a plan to disk.
///
/// Every file is rendered before this is called, so a failure here is an I/O
/// failure rather than a template one. Directories are created as needed;
/// nothing is removed.
pub fn write(plan: &Plan, output_dir: &Path) -> Result<(), ExportError> {
    // A plan built with `Images::Named` carries page names and no bytes. Writing
    // it would produce zero-length PNGs beside perfectly good JSON — an export
    // that looks successful and is not, which is the failure this module exists
    // to prevent. Refuse before creating anything.
    if let Some((path, _)) = plan.binaries.iter().find(|(_, bytes)| bytes.is_empty()) {
        return Err(ExportError::Io {
            path: path.clone(),
            reason: "this plan was built for preview and carries no image data; \
                     re-plan with Images::Encoded before writing"
                .into(),
        });
    }

    std::fs::create_dir_all(output_dir).map_err(|e| ExportError::Io {
        path: output_dir.display().to_string(),
        reason: e.to_string(),
    })?;

    for file in &plan.files {
        write_one(output_dir, &file.path, file.contents.as_bytes())?;
    }
    for (path, bytes) in &plan.binaries {
        write_one(output_dir, path, bytes)?;
    }
    Ok(())
}

fn write_one(output_dir: &Path, rel: &str, bytes: &[u8]) -> Result<(), ExportError> {
    let full = output_dir.join(rel);
    if let Some(parent) = full.parent() {
        std::fs::create_dir_all(parent).map_err(|e| ExportError::Io {
            path: parent.display().to_string(),
            reason: e.to_string(),
        })?;
    }
    std::fs::write(&full, bytes).map_err(|e| ExportError::Io {
        path: full.display().to_string(),
        reason: e.to_string(),
    })
}

/// Render and write in one step.
pub fn export(
    project: &Project,
    assets: &AssetDb,
    preset: &Preset,
    output_dir: &Path,
) -> Result<Plan, ExportError> {
    let plan = plan(project, assets, preset)?;
    write(&plan, output_dir)?;
    Ok(plan)
}
