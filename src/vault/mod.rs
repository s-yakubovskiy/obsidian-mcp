//! Vault layer — pure filesystem operations on an Obsidian vault.
//!
//! Knows nothing about MCP; provides the data model and I/O primitives
//! that tool handlers delegate to.

pub mod frontmatter;
pub mod fs;
pub mod index;
pub mod parser;
pub mod patch;
pub mod periodic;
pub mod watcher;
pub mod wikilink;

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock};

use chrono::{Local, NaiveDate};
use notify_debouncer_mini::Debouncer;

use crate::config::Config;
use crate::error::{VaultError, VaultResult};
use crate::models::{
    DocumentMap, NoteMetadata, NotePeriod, PatchRequest, SearchResult, VaultStats, WikiLink,
};

use self::index::VaultIndex;

/// Internal shared state wrapped in `Arc` for cheap cloning.
struct VaultInner {
    root: PathBuf,
    index: Arc<RwLock<VaultIndex>>,
    /// Kept alive to sustain filesystem watching; never accessed after construction.
    /// Wrapped in `Mutex` to guarantee `Sync` (`Debouncer` contains a `mpsc::Sender`
    /// which is `Send` but not `Sync`).
    _watcher: Mutex<Option<Debouncer<notify::RecommendedWatcher>>>,
}

/// High-level facade over the vault filesystem, index, and watcher.
///
/// `Vault` is `Clone + Send + Sync` — cloning increments an internal `Arc`.
/// All read operations acquire a shared lock on the index; write operations
/// acquire an exclusive lock briefly after the filesystem mutation.
#[derive(Clone)]
pub struct Vault {
    inner: Arc<VaultInner>,
}

impl Vault {
    /// Open a vault: validate the path, build the index, and optionally start the watcher.
    pub async fn open(config: &Config) -> VaultResult<Self> {
        let root = config.vault_path.canonicalize().map_err(|_| {
            VaultError::InvalidPath(format!(
                "vault path does not exist: {}",
                config.vault_path.display()
            ))
        })?;

        if !root.join(".obsidian").is_dir() {
            tracing::warn!(
                path = %root.display(),
                "vault has no .obsidian/ directory — this may be a fresh vault"
            );
        }

        let vi = VaultIndex::build(&root).await?;
        let index = Arc::new(RwLock::new(vi));

        let watcher_handle = if config.watch {
            Some(watcher::start_watcher(root.clone(), Arc::clone(&index))?)
        } else {
            None
        };

        Ok(Self {
            inner: Arc::new(VaultInner {
                root,
                index,
                _watcher: Mutex::new(watcher_handle),
            }),
        })
    }

    /// Vault root path (canonicalized).
    pub fn root(&self) -> &Path {
        &self.inner.root
    }

    // ── fs delegation ──────────────────────────────────────────────────

    pub fn list_files(
        &self,
        dir: &Path,
        recursive: bool,
        glob: Option<&str>,
    ) -> VaultResult<Vec<PathBuf>> {
        fs::list_files(&self.inner.root, dir, recursive, glob)
    }

    pub fn read_note(&self, path: &Path) -> VaultResult<String> {
        fs::read_file(&self.inner.root, path)
    }

    pub fn write_note(&self, path: &Path, content: &str) -> VaultResult<()> {
        fs::write_file(&self.inner.root, path, content)?;
        self.reindex(path)?;
        Ok(())
    }

    pub fn append_note(&self, path: &Path, content: &str) -> VaultResult<()> {
        fs::append_file(&self.inner.root, path, content)?;
        self.reindex(path)?;
        Ok(())
    }

    pub fn delete_note(&self, path: &Path) -> VaultResult<()> {
        fs::delete_file(&self.inner.root, path)?;
        self.write_index().remove_file(path);
        Ok(())
    }

    pub fn move_note(&self, from: &Path, to: &Path) -> VaultResult<PathBuf> {
        let new_path = fs::move_file(&self.inner.root, from, to)?;
        self.write_index()
            .rename_file(&self.inner.root, from, &new_path)?;
        Ok(new_path)
    }

    // ── patch delegation ───────────────────────────────────────────────

    pub fn patch_note(&self, path: &Path, request: &PatchRequest) -> VaultResult<()> {
        let content = fs::read_file(&self.inner.root, path)?;
        let patched = patch::apply_patch(&content, request, path)?;
        fs::write_file(&self.inner.root, path, &patched)?;
        self.reindex(path)?;
        Ok(())
    }

    // ── frontmatter delegation ─────────────────────────────────────────

    pub fn get_frontmatter(&self, path: &Path) -> VaultResult<Option<serde_json::Value>> {
        let content = fs::read_file(&self.inner.root, path)?;
        frontmatter::parse_frontmatter(&content)
    }

    pub fn set_frontmatter_field(
        &self,
        path: &Path,
        key: &str,
        value: serde_json::Value,
    ) -> VaultResult<()> {
        let content = fs::read_file(&self.inner.root, path)?;
        let updated = frontmatter::set_frontmatter_field(&content, key, value)?;
        fs::write_file(&self.inner.root, path, &updated)?;
        self.reindex(path)?;
        Ok(())
    }

    pub fn remove_frontmatter_field(&self, path: &Path, key: &str) -> VaultResult<()> {
        let content = fs::read_file(&self.inner.root, path)?;
        let updated = frontmatter::remove_frontmatter_field(&content, key)?;
        fs::write_file(&self.inner.root, path, &updated)?;
        self.reindex(path)?;
        Ok(())
    }

    // ── index delegation (read-lock) ───────────────────────────────────

    pub fn get_note_metadata(&self, path: &Path) -> VaultResult<NoteMetadata> {
        self.read_index()
            .get_note(path)
            .cloned()
            .ok_or_else(|| VaultError::NoteNotFound(path.to_path_buf()))
    }

    pub fn get_document_map(&self, path: &Path) -> VaultResult<DocumentMap> {
        let content = fs::read_file(&self.inner.root, path)?;
        Ok(parser::build_document_map(&content))
    }

    pub fn search_text(&self, query: &str, context_len: usize) -> VaultResult<Vec<SearchResult>> {
        self.read_index()
            .search_text(&self.inner.root, query, context_len)
    }

    pub fn search_regex(
        &self,
        pattern: &str,
        context_len: usize,
    ) -> VaultResult<Vec<SearchResult>> {
        self.read_index()
            .search_regex(&self.inner.root, pattern, context_len)
    }

    pub fn search_by_tag(&self, tag: &str) -> VaultResult<Vec<NoteMetadata>> {
        Ok(self
            .read_index()
            .notes_with_tag(tag)
            .into_iter()
            .cloned()
            .collect())
    }

    pub fn search_frontmatter(
        &self,
        field: &str,
        value: &serde_json::Value,
    ) -> VaultResult<Vec<NoteMetadata>> {
        Ok(self
            .read_index()
            .search_frontmatter(field, value)
            .into_iter()
            .cloned()
            .collect())
    }

    pub fn backlinks(&self, path: &Path) -> VaultResult<Vec<NoteMetadata>> {
        Ok(self
            .read_index()
            .backlinks_to(path)
            .into_iter()
            .cloned()
            .collect())
    }

    pub fn outgoing_links(&self, path: &Path) -> VaultResult<Vec<WikiLink>> {
        Ok(self
            .read_index()
            .outgoing_links(path)
            .into_iter()
            .cloned()
            .collect())
    }

    pub fn broken_links(&self) -> VaultResult<Vec<(PathBuf, WikiLink)>> {
        Ok(self.read_index().broken_links())
    }

    pub fn orphan_notes(&self) -> VaultResult<Vec<NoteMetadata>> {
        Ok(self
            .read_index()
            .orphan_notes()
            .into_iter()
            .cloned()
            .collect())
    }

    pub fn vault_stats(&self) -> VaultResult<VaultStats> {
        Ok(self.read_index().stats().clone())
    }

    // ── periodic delegation ────────────────────────────────────────────

    pub fn get_periodic_note(
        &self,
        period: &NotePeriod,
        date: Option<NaiveDate>,
    ) -> VaultResult<String> {
        let config = periodic::read_periodic_config(&self.inner.root, period)?;
        let date = date.unwrap_or_else(|| Local::now().date_naive());
        let path = periodic::periodic_note_path(&config, &date);
        fs::read_file(&self.inner.root, &path)
    }

    pub fn create_periodic_note(
        &self,
        period: &NotePeriod,
        date: Option<NaiveDate>,
    ) -> VaultResult<PathBuf> {
        let config = periodic::read_periodic_config(&self.inner.root, period)?;
        let date = date.unwrap_or_else(|| Local::now().date_naive());
        let path = periodic::periodic_note_path(&config, &date);

        if fs::file_exists(&self.inner.root, &path) {
            return Err(VaultError::AlreadyExists(path));
        }

        let content = match &config.template {
            Some(tmpl) if !tmpl.is_empty() => {
                let title = path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or_default();
                periodic::expand_template(&self.inner.root, Path::new(tmpl), &date, title)?
            }
            _ => String::new(),
        };

        fs::write_file(&self.inner.root, &path, &content)?;
        self.reindex(&path)?;
        Ok(path)
    }

    pub fn list_recent_periodic_notes(
        &self,
        period: &NotePeriod,
        limit: usize,
    ) -> VaultResult<Vec<PathBuf>> {
        let config = periodic::read_periodic_config(&self.inner.root, period)?;
        periodic::list_recent_periodic_notes(&self.inner.root, &config, limit)
    }

    // ── private helpers ────────────────────────────────────────────────

    fn read_index(&self) -> std::sync::RwLockReadGuard<'_, VaultIndex> {
        self.inner.index.read().expect("index lock poisoned")
    }

    fn write_index(&self) -> std::sync::RwLockWriteGuard<'_, VaultIndex> {
        self.inner.index.write().expect("index lock poisoned")
    }

    fn reindex(&self, path: &Path) -> VaultResult<()> {
        self.write_index().reindex_file(&self.inner.root, path)
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;
    use crate::config::Config;
    use crate::models::{PatchOperation, PatchTargetType};

    fn test_config(vault_root: &Path) -> Config {
        Config {
            vault_path: vault_root.to_path_buf(),
            watch: false,
            log_level: "error".into(),
        }
    }

    fn create_test_vault(dir: &Path) {
        std::fs::create_dir_all(dir.join(".obsidian")).unwrap();
    }

    #[tokio::test]
    async fn vault_open_succeeds() {
        let dir = tempfile::tempdir().unwrap();
        create_test_vault(dir.path());
        let config = test_config(dir.path());

        let vault = Vault::open(&config).await;
        assert!(vault.is_ok());
    }

    #[tokio::test]
    async fn vault_open_without_obsidian_dir() {
        let dir = tempfile::tempdir().unwrap();
        let config = test_config(dir.path());

        let vault = Vault::open(&config).await;
        assert!(vault.is_ok(), "should succeed even without .obsidian/");
    }

    #[tokio::test]
    async fn vault_open_nonexistent_path() {
        let config = Config {
            vault_path: PathBuf::from("/nonexistent/path/that/does/not/exist"),
            watch: false,
            log_level: "error".into(),
        };

        let result = Vault::open(&config).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn vault_read_write_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        create_test_vault(dir.path());

        let vault = Vault::open(&test_config(dir.path())).await.unwrap();

        vault
            .write_note(Path::new("hello.md"), "# Hello\nWorld")
            .unwrap();
        let content = vault.read_note(Path::new("hello.md")).unwrap();
        assert_eq!(content, "# Hello\nWorld");
    }

    #[tokio::test]
    async fn vault_append_note() {
        let dir = tempfile::tempdir().unwrap();
        create_test_vault(dir.path());
        let vault = Vault::open(&test_config(dir.path())).await.unwrap();

        vault.write_note(Path::new("log.md"), "line one\n").unwrap();
        vault
            .append_note(Path::new("log.md"), "line two\n")
            .unwrap();

        let content = vault.read_note(Path::new("log.md")).unwrap();
        assert_eq!(content, "line one\nline two\n");
    }

    #[tokio::test]
    async fn vault_delete_removes_from_index() {
        let dir = tempfile::tempdir().unwrap();
        create_test_vault(dir.path());
        let vault = Vault::open(&test_config(dir.path())).await.unwrap();

        vault
            .write_note(Path::new("ephemeral.md"), "# Gone soon")
            .unwrap();
        assert!(vault.get_note_metadata(Path::new("ephemeral.md")).is_ok());

        vault.delete_note(Path::new("ephemeral.md")).unwrap();
        assert!(vault.get_note_metadata(Path::new("ephemeral.md")).is_err());
    }

    #[tokio::test]
    async fn vault_move_updates_index() {
        let dir = tempfile::tempdir().unwrap();
        create_test_vault(dir.path());
        let vault = Vault::open(&test_config(dir.path())).await.unwrap();

        vault.write_note(Path::new("old.md"), "# Moved").unwrap();
        assert!(vault.get_note_metadata(Path::new("old.md")).is_ok());

        let new_path = vault
            .move_note(Path::new("old.md"), Path::new("subdir/new.md"))
            .unwrap();
        assert_eq!(new_path, PathBuf::from("subdir/new.md"));

        assert!(vault.get_note_metadata(Path::new("old.md")).is_err());
        assert!(vault.get_note_metadata(Path::new("subdir/new.md")).is_ok());

        let content = vault.read_note(Path::new("subdir/new.md")).unwrap();
        assert_eq!(content, "# Moved");
    }

    #[tokio::test]
    async fn vault_patch_note() {
        let dir = tempfile::tempdir().unwrap();
        create_test_vault(dir.path());
        let vault = Vault::open(&test_config(dir.path())).await.unwrap();

        vault
            .write_note(
                Path::new("patched.md"),
                "# Section\nOriginal content\n# Other\nKeep this\n",
            )
            .unwrap();

        let request = PatchRequest {
            operation: PatchOperation::Replace,
            target_type: PatchTargetType::Heading,
            target: "Section".into(),
            content: "Replaced content\n".into(),
        };
        vault.patch_note(Path::new("patched.md"), &request).unwrap();

        let content = vault.read_note(Path::new("patched.md")).unwrap();
        assert!(content.contains("Replaced content"));
        assert!(content.contains("# Other"));
        assert!(content.contains("Keep this"));
    }

    #[tokio::test]
    async fn vault_frontmatter_operations() {
        let dir = tempfile::tempdir().unwrap();
        create_test_vault(dir.path());
        let vault = Vault::open(&test_config(dir.path())).await.unwrap();

        vault
            .write_note(Path::new("meta.md"), "# Note\nBody text\n")
            .unwrap();

        vault
            .set_frontmatter_field(
                Path::new("meta.md"),
                "status",
                serde_json::Value::String("draft".into()),
            )
            .unwrap();
        vault
            .set_frontmatter_field(
                Path::new("meta.md"),
                "priority",
                serde_json::Value::Number(1.into()),
            )
            .unwrap();

        let fm = vault.get_frontmatter(Path::new("meta.md")).unwrap();
        assert!(fm.is_some());
        let obj = fm.unwrap();
        assert_eq!(obj["status"], "draft");
        assert_eq!(obj["priority"], 1);

        vault
            .remove_frontmatter_field(Path::new("meta.md"), "status")
            .unwrap();

        let fm = vault.get_frontmatter(Path::new("meta.md")).unwrap();
        let obj = fm.unwrap();
        assert!(obj.get("status").is_none());
        assert_eq!(obj["priority"], 1);
    }

    #[tokio::test]
    async fn vault_search_text() {
        let dir = tempfile::tempdir().unwrap();
        create_test_vault(dir.path());
        let vault = Vault::open(&test_config(dir.path())).await.unwrap();

        vault
            .write_note(
                Path::new("searchable.md"),
                "# Rust\nRust is a systems language.\n",
            )
            .unwrap();
        vault
            .write_note(Path::new("other.md"), "# Python\nPython is dynamic.\n")
            .unwrap();

        let results = vault.search_text("Rust", 40).unwrap();
        assert!(!results.is_empty());
        assert!(
            results
                .iter()
                .any(|r| r.path == PathBuf::from("searchable.md"))
        );
        assert!(!results.iter().any(|r| r.path == PathBuf::from("other.md")));
    }

    #[tokio::test]
    async fn vault_document_map() {
        let dir = tempfile::tempdir().unwrap();
        create_test_vault(dir.path());
        let vault = Vault::open(&test_config(dir.path())).await.unwrap();

        vault
            .write_note(
                Path::new("mapped.md"),
                "---\ntags: [rust]\n---\n# Heading\n## Sub\nText ^block1\n",
            )
            .unwrap();

        let map = vault.get_document_map(Path::new("mapped.md")).unwrap();
        assert!(map.headings.iter().any(|h| h.contains("Heading")));
        assert!(map.headings.iter().any(|h| h.contains("Sub")));
        assert!(map.block_refs.contains(&"block1".to_string()));
        assert!(map.frontmatter_fields.contains(&"tags".to_string()));
    }

    #[tokio::test]
    async fn vault_tags_and_backlinks() {
        let dir = tempfile::tempdir().unwrap();
        create_test_vault(dir.path());
        let vault = Vault::open(&test_config(dir.path())).await.unwrap();

        vault
            .write_note(Path::new("a.md"), "---\ntags: [project]\n---\n# A\n[[b]]\n")
            .unwrap();
        vault
            .write_note(Path::new("b.md"), "---\ntags: [project]\n---\n# B\n")
            .unwrap();

        let tagged = vault.search_by_tag("project").unwrap();
        assert_eq!(tagged.len(), 2);

        let links = vault.outgoing_links(Path::new("a.md")).unwrap();
        assert!(links.iter().any(|l| l.target == "b"));

        let bl = vault.backlinks(Path::new("b.md")).unwrap();
        assert!(bl.iter().any(|m| m.path == PathBuf::from("a.md")));
    }

    #[tokio::test]
    async fn vault_stats() {
        let dir = tempfile::tempdir().unwrap();
        create_test_vault(dir.path());
        let vault = Vault::open(&test_config(dir.path())).await.unwrap();

        vault.write_note(Path::new("one.md"), "# One").unwrap();
        vault.write_note(Path::new("two.md"), "# Two").unwrap();

        let stats = vault.vault_stats().unwrap();
        assert_eq!(stats.total_notes, 2);
    }

    #[test]
    fn vault_is_send_sync_clone() {
        fn assert_send_sync_clone<T: Send + Sync + Clone>() {}
        assert_send_sync_clone::<Vault>();
    }
}
