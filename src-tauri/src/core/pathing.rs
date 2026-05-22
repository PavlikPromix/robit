use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};

use super::models::{ItemKind, MovePreview, MoveRequest};

pub fn build_preview(request: &MoveRequest) -> Result<MovePreview> {
    let source = normalize_existing_path(&request.source_path)?;
    let destination_parent = normalize_existing_directory(&request.destination_parent)?;
    let item_kind = classify_source(&source)?;
    let file_name = source
        .file_name()
        .context("source path must include a file or directory name")?;
    let destination = destination_parent.join(file_name);

    if source == destination {
        bail!("source and destination resolve to the same path");
    }

    if destination.exists() {
        bail!(
            "destination already exists: {}",
            destination.to_string_lossy()
        );
    }

    Ok(MovePreview {
        source_path: source.to_string_lossy().to_string(),
        destination_path: destination.to_string_lossy().to_string(),
        item_kind,
        locks: Vec::new(),
    })
}

pub fn classify_source(path: &Path) -> Result<ItemKind> {
    let metadata = path
        .metadata()
        .with_context(|| format!("cannot read source metadata: {}", path.display()))?;
    if metadata.is_dir() {
        Ok(ItemKind::Directory)
    } else if metadata.is_file() {
        Ok(ItemKind::File)
    } else {
        bail!("source must be a regular file or directory");
    }
}

fn normalize_existing_path(path: &str) -> Result<PathBuf> {
    let path = PathBuf::from(path);
    if !path.exists() {
        bail!("source does not exist: {}", path.display());
    }
    path.canonicalize()
        .with_context(|| format!("cannot canonicalize source: {}", path.display()))
}

fn normalize_existing_directory(path: &str) -> Result<PathBuf> {
    let path = PathBuf::from(path);
    if !path.exists() {
        bail!("destination parent does not exist: {}", path.display());
    }
    let metadata = path
        .metadata()
        .with_context(|| format!("cannot read destination metadata: {}", path.display()))?;
    if !metadata.is_dir() {
        bail!("destination parent must be a directory");
    }
    path.canonicalize()
        .with_context(|| format!("cannot canonicalize destination parent: {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_regular_paths_by_metadata() {
        let current = std::env::current_dir().unwrap();
        assert_eq!(classify_source(&current).unwrap(), ItemKind::Directory);
    }
}
