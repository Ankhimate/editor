//! The `.ankh` zip container (PLAN §6.1, ADR 0004).
//!
//! An `.ankh` file is a zip archive, not a bare JSON file. The layout is:
//!
//! ```text
//! project.json      the document (schema::Project, pretty JSON)
//! images/…          referenced textures, path-preserved
//! ```
//!
//! Bundling images into the project keeps a character a single portable file
//! rather than a `.json` plus a loose folder that goes missing on copy. Images
//! are stored uncompressed-metadata but deflate-compressed like the rest; PNGs
//! are already compressed, so the ratio is ~1 and the cost is the CRC pass.

use std::io::{Read, Write};
use std::path::Path;

/// The document entry name inside the archive.
pub const PROJECT_ENTRY: &str = "project.json";
/// Directory prefix for bundled textures.
pub const IMAGES_PREFIX: &str = "images/";

#[derive(Debug, thiserror::Error)]
pub enum ContainerError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("zip error: {0}")]
    Zip(#[from] zip::result::ZipError),
    #[error("archive has no `{PROJECT_ENTRY}` entry")]
    MissingProject,
}

/// A named binary blob (a texture) to bundle alongside the document.
pub struct ImageBlob {
    /// Path relative to `images/`, e.g. `arm.png` or `parts/head.png`.
    pub rel_path: String,
    pub bytes: Vec<u8>,
}

/// The raw contents read out of a container: the document JSON plus any images.
pub struct Contents {
    pub project_json: String,
    pub images: Vec<ImageBlob>,
}

fn options() -> zip::write::SimpleFileOptions {
    // Deflate matches the crate's single enabled feature. No compression level
    // knob is exposed: default is the right trade for text + already-compressed
    // PNGs.
    zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated)
}

/// Write `project.json` + images to a zip container at `path`.
pub fn write(path: &Path, project_json: &str, images: &[ImageBlob]) -> Result<(), ContainerError> {
    let file = std::fs::File::create(path)?;
    let mut zip = zip::ZipWriter::new(file);

    zip.start_file(PROJECT_ENTRY, options())?;
    zip.write_all(project_json.as_bytes())?;

    for image in images {
        let name = format!("{IMAGES_PREFIX}{}", image.rel_path);
        zip.start_file(name, options())?;
        zip.write_all(&image.bytes)?;
    }

    zip.finish()?;
    Ok(())
}

/// Read `project.json` + images back out of a container.
pub fn read(path: &Path) -> Result<Contents, ContainerError> {
    let file = std::fs::File::open(path)?;
    let mut zip = zip::ZipArchive::new(file)?;

    let mut project_json: Option<String> = None;
    let mut images = Vec::new();

    for i in 0..zip.len() {
        let mut entry = zip.by_index(i)?;
        // Directory entries carry no bytes; skip them.
        if entry.is_dir() {
            continue;
        }
        let name = entry.name().to_string();
        if name == PROJECT_ENTRY {
            let mut s = String::new();
            entry.read_to_string(&mut s)?;
            project_json = Some(s);
        } else if let Some(rel) = name.strip_prefix(IMAGES_PREFIX) {
            let mut bytes = Vec::new();
            entry.read_to_end(&mut bytes)?;
            images.push(ImageBlob {
                rel_path: rel.to_string(),
                bytes,
            });
        }
        // Anything else is ignored so a newer writer's extra entries survive an
        // older reader without erroring.
    }

    Ok(Contents {
        project_json: project_json.ok_or(ContainerError::MissingProject)?,
        images,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_document_and_images() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("c.ankh");
        let imgs = vec![
            ImageBlob {
                rel_path: "arm.png".into(),
                bytes: vec![1, 2, 3, 4],
            },
            ImageBlob {
                rel_path: "parts/head.png".into(),
                bytes: vec![9, 8, 7],
            },
        ];
        write(&path, "{\"hi\":1}", &imgs).unwrap();

        let got = read(&path).unwrap();
        assert_eq!(got.project_json, "{\"hi\":1}");
        assert_eq!(got.images.len(), 2);
        // Order is archive order, which matches write order here.
        assert_eq!(got.images[0].rel_path, "arm.png");
        assert_eq!(got.images[0].bytes, vec![1, 2, 3, 4]);
        assert_eq!(got.images[1].rel_path, "parts/head.png");
    }

    #[test]
    fn missing_project_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("empty.ankh");
        write(&path, "{}", &[]).unwrap();
        // Overwrite with an archive that has only an image.
        let file = std::fs::File::create(&path).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        zip.start_file("images/x.png", options()).unwrap();
        zip.write_all(&[0u8]).unwrap();
        zip.finish().unwrap();

        assert!(matches!(read(&path), Err(ContainerError::MissingProject)));
    }
}
