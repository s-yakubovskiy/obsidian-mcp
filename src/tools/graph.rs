//! Backlink, outgoing-link, and graph traversal tools.

use std::path::{Path, PathBuf};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::error::VaultResult;
use crate::vault::Vault;

// ── param structs ───────────────────────────────────────────────────

#[derive(Debug, Deserialize, JsonSchema, Default)]
pub struct LinksBacklinksParams {
    /// Path to the note (relative to vault root), e.g. `notes/example.md`.
    pub path: String,
}

#[derive(Debug, Deserialize, JsonSchema, Default)]
pub struct LinksOutgoingParams {
    /// Path to the note (relative to vault root), e.g. `notes/example.md`.
    pub path: String,
}

#[derive(Debug, Deserialize, JsonSchema, Default)]
pub struct LinksBrokenParams {
    /// Optional note path to check. If omitted, checks the entire vault.
    pub path: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema, Default)]
pub struct LinksOrphansParams {}

// ── response types ──────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct BacklinkSource {
    /// Path of the note that contains links to the target.
    pub source_path: PathBuf,
    /// The specific wikilinks in this note that point to the target.
    pub links: Vec<BacklinkRef>,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct BacklinkRef {
    /// Raw wikilink text, e.g. `[[note#heading|alias]]`.
    pub raw: String,
    /// 0-based line number where the link appears.
    pub line: usize,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct OutgoingLink {
    /// Raw wikilink text.
    pub raw: String,
    /// Link target (note name or path).
    pub target: String,
    /// Resolved vault-relative path, or `null` if the link is broken.
    pub resolved_path: Option<PathBuf>,
    /// Heading fragment, if present.
    pub heading: Option<String>,
    /// Block reference, if present.
    pub block_ref: Option<String>,
    /// Display alias, if present.
    pub alias: Option<String>,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct BrokenLink {
    /// Path of the note containing the broken link.
    pub source_path: PathBuf,
    /// Raw wikilink text.
    pub link_raw: String,
    /// Unresolved target.
    pub target: String,
}

// ── handler functions ───────────────────────────────────────────────

/// Find all notes linking TO a given note, with the specific wikilinks used.
pub fn get_backlinks(vault: &Vault, path: &Path) -> VaultResult<Vec<BacklinkSource>> {
    vault.get_note_metadata(path)?;

    let backlink_notes = vault.backlinks(path)?;

    let result = backlink_notes
        .iter()
        .filter_map(|source| {
            let matching: Vec<BacklinkRef> = source
                .links
                .iter()
                .filter(|link| vault.resolve_link(&link.target).as_deref() == Some(path))
                .map(|link| BacklinkRef {
                    raw: link.raw.clone(),
                    line: link.line,
                })
                .collect();

            if matching.is_empty() {
                None
            } else {
                Some(BacklinkSource {
                    source_path: source.path.clone(),
                    links: matching,
                })
            }
        })
        .collect();

    Ok(result)
}

/// Find all outgoing links FROM a given note, with resolution status.
pub fn get_outgoing_links(vault: &Vault, path: &Path) -> VaultResult<Vec<OutgoingLink>> {
    vault.get_note_metadata(path)?;

    let links = vault.outgoing_links(path)?;

    let result = links
        .into_iter()
        .map(|link| {
            let resolved_path = vault.resolve_link(&link.target);
            OutgoingLink {
                raw: link.raw,
                target: link.target,
                resolved_path,
                heading: link.heading,
                block_ref: link.block_ref,
                alias: link.alias,
            }
        })
        .collect();

    Ok(result)
}

/// Find all broken (unresolved) wikilinks, optionally filtered to a single note.
pub fn get_broken_links(vault: &Vault, path: Option<&Path>) -> VaultResult<Vec<BrokenLink>> {
    if let Some(p) = path {
        vault.get_note_metadata(p)?;
        let links = vault.outgoing_links(p)?;

        Ok(links
            .into_iter()
            .filter(|link| !link.target.is_empty() && vault.resolve_link(&link.target).is_none())
            .map(|link| BrokenLink {
                source_path: p.to_path_buf(),
                link_raw: link.raw,
                target: link.target,
            })
            .collect())
    } else {
        let all = vault.broken_links()?;
        Ok(all
            .into_iter()
            .map(|(source_path, link)| BrokenLink {
                source_path,
                link_raw: link.raw,
                target: link.target,
            })
            .collect())
    }
}

/// Find notes with no inbound and no outbound links.
pub fn get_orphan_notes(vault: &Vault) -> VaultResult<Vec<PathBuf>> {
    let orphans = vault.orphan_notes()?;
    Ok(orphans.into_iter().map(|n| n.path).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::config::Config;

    fn test_config(vault_root: &Path) -> Config {
        Config {
            vault_path: vault_root.to_path_buf(),
            watch: false,
            log_level: "error".into(),
        }
    }

    fn create_test_vault(dir: &Path) {
        std::fs::create_dir_all(dir.join(".obsidian")).unwrap();

        std::fs::write(dir.join("a.md"), "# A\n\nLinks to [[b]] and [[c]].\n").unwrap();
        std::fs::write(dir.join("b.md"), "# B\n\nLinks back to [[a]].\n").unwrap();
        std::fs::write(dir.join("c.md"), "# C\n\nLinks to [[a#heading|alias]].\n").unwrap();
        std::fs::write(
            dir.join("d.md"),
            "# D\n\nLinks to [[nonexistent]] and [[a]].\n",
        )
        .unwrap();
        std::fs::write(dir.join("orphan.md"), "# Orphan\n\nNo links here.\n").unwrap();
    }

    #[tokio::test]
    async fn backlinks_returns_correct_sources_and_refs() {
        let dir = tempfile::tempdir().unwrap();
        create_test_vault(dir.path());
        let vault = Vault::open(&test_config(dir.path())).await.unwrap();

        let backlinks = get_backlinks(&vault, Path::new("a.md")).unwrap();

        let source_paths: Vec<&PathBuf> = backlinks.iter().map(|bl| &bl.source_path).collect();
        assert!(source_paths.contains(&&PathBuf::from("b.md")));
        assert!(source_paths.contains(&&PathBuf::from("c.md")));
        assert!(source_paths.contains(&&PathBuf::from("d.md")));

        let b_entry = backlinks
            .iter()
            .find(|bl| bl.source_path == PathBuf::from("b.md"))
            .unwrap();
        assert_eq!(b_entry.links.len(), 1);
        assert!(b_entry.links[0].raw.contains("[[a]]"));
    }

    #[tokio::test]
    async fn backlinks_nonexistent_note_errors() {
        let dir = tempfile::tempdir().unwrap();
        create_test_vault(dir.path());
        let vault = Vault::open(&test_config(dir.path())).await.unwrap();

        assert!(get_backlinks(&vault, Path::new("nonexistent.md")).is_err());
    }

    #[tokio::test]
    async fn backlinks_note_with_none_returns_empty() {
        let dir = tempfile::tempdir().unwrap();
        create_test_vault(dir.path());
        let vault = Vault::open(&test_config(dir.path())).await.unwrap();

        let backlinks = get_backlinks(&vault, Path::new("orphan.md")).unwrap();
        assert!(backlinks.is_empty());
    }

    #[tokio::test]
    async fn outgoing_links_with_resolved_paths() {
        let dir = tempfile::tempdir().unwrap();
        create_test_vault(dir.path());
        let vault = Vault::open(&test_config(dir.path())).await.unwrap();

        let links = get_outgoing_links(&vault, Path::new("a.md")).unwrap();
        assert_eq!(links.len(), 2);

        let b_link = links.iter().find(|l| l.target == "b").unwrap();
        assert_eq!(b_link.resolved_path, Some(PathBuf::from("b.md")));

        let c_link = links.iter().find(|l| l.target == "c").unwrap();
        assert_eq!(c_link.resolved_path, Some(PathBuf::from("c.md")));
    }

    #[tokio::test]
    async fn outgoing_links_broken_shown_as_none() {
        let dir = tempfile::tempdir().unwrap();
        create_test_vault(dir.path());
        let vault = Vault::open(&test_config(dir.path())).await.unwrap();

        let links = get_outgoing_links(&vault, Path::new("d.md")).unwrap();

        let broken = links.iter().find(|l| l.target == "nonexistent").unwrap();
        assert!(broken.resolved_path.is_none());

        let resolved = links.iter().find(|l| l.target == "a").unwrap();
        assert_eq!(resolved.resolved_path, Some(PathBuf::from("a.md")));
    }

    #[tokio::test]
    async fn outgoing_links_include_heading_and_alias() {
        let dir = tempfile::tempdir().unwrap();
        create_test_vault(dir.path());
        let vault = Vault::open(&test_config(dir.path())).await.unwrap();

        let links = get_outgoing_links(&vault, Path::new("c.md")).unwrap();
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].target, "a");
        assert_eq!(links[0].heading.as_deref(), Some("heading"));
        assert_eq!(links[0].alias.as_deref(), Some("alias"));
        assert_eq!(links[0].resolved_path, Some(PathBuf::from("a.md")));
    }

    #[tokio::test]
    async fn outgoing_links_nonexistent_note_errors() {
        let dir = tempfile::tempdir().unwrap();
        create_test_vault(dir.path());
        let vault = Vault::open(&test_config(dir.path())).await.unwrap();

        assert!(get_outgoing_links(&vault, Path::new("nonexistent.md")).is_err());
    }

    #[tokio::test]
    async fn broken_links_vault_wide() {
        let dir = tempfile::tempdir().unwrap();
        create_test_vault(dir.path());
        let vault = Vault::open(&test_config(dir.path())).await.unwrap();

        let broken = get_broken_links(&vault, None).unwrap();
        assert!(!broken.is_empty());
        assert!(broken.iter().any(|bl| bl.target == "nonexistent"));
        assert!(
            broken
                .iter()
                .any(|bl| bl.source_path == PathBuf::from("d.md"))
        );
    }

    #[tokio::test]
    async fn broken_links_single_note_with_broken() {
        let dir = tempfile::tempdir().unwrap();
        create_test_vault(dir.path());
        let vault = Vault::open(&test_config(dir.path())).await.unwrap();

        let broken = get_broken_links(&vault, Some(Path::new("d.md"))).unwrap();
        assert_eq!(broken.len(), 1);
        assert_eq!(broken[0].target, "nonexistent");
    }

    #[tokio::test]
    async fn broken_links_single_note_without_broken() {
        let dir = tempfile::tempdir().unwrap();
        create_test_vault(dir.path());
        let vault = Vault::open(&test_config(dir.path())).await.unwrap();

        let broken = get_broken_links(&vault, Some(Path::new("a.md"))).unwrap();
        assert!(broken.is_empty());
    }

    #[tokio::test]
    async fn broken_links_nonexistent_note_errors() {
        let dir = tempfile::tempdir().unwrap();
        create_test_vault(dir.path());
        let vault = Vault::open(&test_config(dir.path())).await.unwrap();

        assert!(get_broken_links(&vault, Some(Path::new("nonexistent.md"))).is_err());
    }

    #[tokio::test]
    async fn orphan_notes_detected() {
        let dir = tempfile::tempdir().unwrap();
        create_test_vault(dir.path());
        let vault = Vault::open(&test_config(dir.path())).await.unwrap();

        let orphans = get_orphan_notes(&vault).unwrap();
        assert!(orphans.contains(&PathBuf::from("orphan.md")));

        assert!(!orphans.contains(&PathBuf::from("a.md")));
        assert!(!orphans.contains(&PathBuf::from("b.md")));
        assert!(!orphans.contains(&PathBuf::from("c.md")));
    }

    #[tokio::test]
    async fn orphan_notes_exclude_notes_with_outgoing_links() {
        let dir = tempfile::tempdir().unwrap();
        create_test_vault(dir.path());
        let vault = Vault::open(&test_config(dir.path())).await.unwrap();

        let orphans = get_orphan_notes(&vault).unwrap();
        // d.md has outgoing links even though nothing links to it
        assert!(!orphans.contains(&PathBuf::from("d.md")));
    }
}
