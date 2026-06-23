//! Vault-relative path normalization and Unicode-aware filesystem resolution.

use std::ffi::{OsStr, OsString};
use std::path::{Component, Path, PathBuf};

use unicode_normalization::UnicodeNormalization;

use crate::error::{VaultError, VaultResult};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedPath {
    pub relative: PathBuf,
    pub absolute: PathBuf,
}

/// Normalize a vault-relative path, rejecting absolute paths and root escapes.
pub fn normalize_relative(path: &Path) -> VaultResult<PathBuf> {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(c) => normalized.push(c),
            Component::CurDir => {}
            Component::ParentDir => {
                if !normalized.pop() {
                    return Err(VaultError::OutsideVault(path.to_path_buf()));
                }
            }
            Component::RootDir | Component::Prefix(_) => {
                return Err(VaultError::InvalidPath(format!(
                    "absolute paths not allowed: {}",
                    path.display()
                )));
            }
        }
    }
    Ok(forward_slash_path(&normalized))
}

/// Convert an absolute path under the vault root into the internal relative form.
pub fn relative_from_absolute(vault_root: &Path, absolute: &Path) -> VaultResult<PathBuf> {
    let relative = absolute
        .strip_prefix(vault_root)
        .map_err(|_| VaultError::OutsideVault(absolute.to_path_buf()))?;
    normalize_relative(relative)
}

/// Unicode canonical key used for normalization-form-insensitive comparisons.
pub fn canonical_unicode_key(s: &str) -> String {
    s.nfc().collect()
}

/// Resolve an existing vault path to the actual disk-relative spelling.
pub fn resolve_existing(vault_root: &Path, relative: &Path) -> VaultResult<ResolvedPath> {
    let normalized = normalize_relative(relative)?;
    let canonical_root = canonical_root(vault_root)?;

    if normalized.as_os_str().is_empty() {
        return Ok(ResolvedPath {
            relative: PathBuf::new(),
            absolute: canonical_root,
        });
    }

    let mut absolute = canonical_root.clone();
    let mut actual_relative = PathBuf::new();

    for component in normal_components(&normalized) {
        let entry_name = match find_component(&absolute, &component, relative)? {
            Some(name) => name,
            None => return Err(VaultError::NoteNotFound(relative.to_path_buf())),
        };
        absolute.push(&entry_name);
        actual_relative.push(&entry_name);
        ensure_inside_root(&canonical_root, &absolute, relative)?;
    }

    Ok(ResolvedPath {
        relative: forward_slash_path(&actual_relative),
        absolute,
    })
}

/// Resolve a write target, preserving existing component spellings and the new
/// final spelling when the target does not already exist.
pub fn resolve_for_write(vault_root: &Path, relative: &Path) -> VaultResult<ResolvedPath> {
    match resolve_existing(vault_root, relative) {
        Ok(existing) => return Ok(existing),
        Err(VaultError::NoteNotFound(_)) => {}
        Err(err) => return Err(err),
    }

    let normalized = normalize_relative(relative)?;
    let canonical_root = canonical_root(vault_root)?;

    if normalized.as_os_str().is_empty() {
        return Ok(ResolvedPath {
            relative: PathBuf::new(),
            absolute: canonical_root,
        });
    }

    let components = normal_components(&normalized);
    let mut absolute = canonical_root.clone();
    let mut actual_relative = PathBuf::new();

    for (index, component) in components.iter().enumerate() {
        match find_component(&absolute, component, relative)? {
            Some(entry_name) => {
                absolute.push(&entry_name);
                actual_relative.push(&entry_name);
                ensure_inside_root(&canonical_root, &absolute, relative)?;
            }
            None => {
                absolute.push(component);
                actual_relative.push(component);
                for remaining in components.iter().skip(index + 1) {
                    absolute.push(remaining);
                    actual_relative.push(remaining);
                }
                ensure_write_parent_inside_root(&canonical_root, &absolute, relative)?;
                return Ok(ResolvedPath {
                    relative: forward_slash_path(&actual_relative),
                    absolute,
                });
            }
        }
    }

    Ok(ResolvedPath {
        relative: forward_slash_path(&actual_relative),
        absolute,
    })
}

pub fn resolve_parent_for_write(
    vault_root: &Path,
    relative: &Path,
) -> VaultResult<(PathBuf, PathBuf)> {
    let resolved = resolve_for_write(vault_root, relative)?;
    let parent_abs = resolved
        .absolute
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| resolved.absolute.clone());
    let parent_rel = resolved
        .relative
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_default();
    Ok((forward_slash_path(&parent_rel), parent_abs))
}

fn canonical_root(vault_root: &Path) -> VaultResult<PathBuf> {
    vault_root
        .canonicalize()
        .map_err(|_| VaultError::InvalidPath(vault_root.display().to_string()))
}

fn normal_components(path: &Path) -> Vec<OsString> {
    path.components()
        .filter_map(|component| match component {
            Component::Normal(c) => Some(c.to_os_string()),
            _ => None,
        })
        .collect()
}

fn find_component(
    directory: &Path,
    requested: &OsStr,
    original_path: &Path,
) -> VaultResult<Option<OsString>> {
    let entries = match std::fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(VaultError::Io(err)),
    };

    let requested_key = canonical_unicode_key(&requested.to_string_lossy());
    let mut normalized_matches = Vec::new();

    for entry in entries {
        let entry = entry?;
        let file_name = entry.file_name();
        if file_name == requested {
            return Ok(Some(file_name));
        }

        let entry_key = canonical_unicode_key(&file_name.to_string_lossy());
        if entry_key == requested_key {
            normalized_matches.push(file_name);
        }
    }

    match normalized_matches.as_slice() {
        [] => Ok(None),
        [single] => Ok(Some(single.clone())),
        _ => Err(VaultError::InvalidPath(format!(
            "ambiguous Unicode-normalized path component '{}' in {}",
            requested.to_string_lossy(),
            original_path.display()
        ))),
    }
}

fn ensure_inside_root(
    canonical_root: &Path,
    absolute: &Path,
    original_path: &Path,
) -> VaultResult<()> {
    if absolute.exists() {
        let canonical = absolute.canonicalize()?;
        if !canonical.starts_with(canonical_root) {
            return Err(VaultError::OutsideVault(original_path.to_path_buf()));
        }
    }
    Ok(())
}

fn ensure_write_parent_inside_root(
    canonical_root: &Path,
    absolute: &Path,
    original_path: &Path,
) -> VaultResult<()> {
    if let Some(parent) = nearest_existing_parent(absolute) {
        let canonical_parent = parent.canonicalize()?;
        if !canonical_parent.starts_with(canonical_root) {
            return Err(VaultError::OutsideVault(original_path.to_path_buf()));
        }
    }
    Ok(())
}

fn nearest_existing_parent(path: &Path) -> Option<PathBuf> {
    let mut current = path.parent()?.to_path_buf();
    loop {
        if current.exists() {
            return Some(current);
        }
        if !current.pop() {
            return None;
        }
    }
}

fn forward_slash_path(path: &Path) -> PathBuf {
    PathBuf::from(path.to_string_lossy().replace('\\', "/"))
}

#[cfg(test)]
mod tests {
    use super::*;

    use tempfile::TempDir;
    use unicode_normalization::UnicodeNormalization;

    fn setup_vault() -> TempDir {
        let dir = TempDir::new().unwrap();
        std::fs::create_dir_all(dir.path().join(".obsidian")).unwrap();
        dir
    }

    #[test]
    fn normalize_rejects_parent_escape() {
        let err = normalize_relative(Path::new("../secret.md")).unwrap_err();
        assert!(matches!(err, VaultError::OutsideVault(_)));
    }

    #[test]
    fn normalize_rejects_absolute_path() {
        let err = normalize_relative(Path::new("/tmp/secret.md")).unwrap_err();
        assert!(matches!(err, VaultError::InvalidPath(_)));
    }

    #[test]
    fn resolve_existing_finds_decomposed_filename_from_composed_input() {
        let vault = setup_vault();
        let composed = "02_База-знаний/Сущности/lic1c.md";
        let decomposed: String = composed.nfd().collect();
        let disk_path = PathBuf::from(&decomposed);
        std::fs::create_dir_all(vault.path().join(disk_path.parent().unwrap())).unwrap();
        std::fs::write(vault.path().join(&disk_path), "# License\n").unwrap();

        let resolved = resolve_existing(vault.path(), Path::new(composed)).unwrap();
        assert_eq!(resolved.relative, disk_path);
        assert!(resolved.absolute.is_file());
    }

    #[test]
    fn resolve_for_write_uses_existing_decomposed_parent() {
        let vault = setup_vault();
        let composed_dir = "02_База-знаний";
        let decomposed_dir: String = composed_dir.nfd().collect();
        std::fs::create_dir_all(vault.path().join(&decomposed_dir)).unwrap();

        let resolved = resolve_for_write(vault.path(), Path::new("02_База-знаний/New.md")).unwrap();
        assert_eq!(
            resolved.relative,
            PathBuf::from(format!("{decomposed_dir}/New.md"))
        );
        assert!(
            resolved
                .absolute
                .ends_with(Path::new(&decomposed_dir).join("New.md"))
        );
    }

    #[test]
    fn resolve_for_write_preserves_new_final_component_spelling() {
        let vault = setup_vault();
        let path = Path::new("New Folder/café.md");

        let resolved = resolve_for_write(vault.path(), path).unwrap();
        assert_eq!(resolved.relative, path);
    }

    #[test]
    fn relative_from_absolute_returns_forward_slashes() {
        let vault = setup_vault();
        let absolute = vault.path().join("a").join("b.md");
        std::fs::create_dir_all(absolute.parent().unwrap()).unwrap();
        std::fs::write(&absolute, "# B\n").unwrap();

        let relative = relative_from_absolute(vault.path(), &absolute).unwrap();
        assert_eq!(relative, PathBuf::from("a/b.md"));
    }
}
