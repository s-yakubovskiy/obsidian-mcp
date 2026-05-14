//! MCP tool handlers — thin wrappers that translate MCP requests into vault operations.

pub mod graph;
pub mod metadata;
pub mod navigation;
pub mod notes;
pub mod periodic;
pub mod search;
pub mod utility;

use std::collections::HashSet;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, ErrorData, Implementation, ServerCapabilities, ServerInfo};
use rmcp::{ServerHandler, tool, tool_handler, tool_router};

use crate::client::semantic_daemon::SemanticDaemonClient;
use crate::config::SemanticMode;
use crate::vault::Vault;

#[derive(Clone)]
pub struct SemanticRuntime {
    pub mode: SemanticMode,
    pub daemon_client: Option<SemanticDaemonClient>,
    pub daemon_unavailable_reason: Option<String>,
    pub prefetch_count: usize,
    pub vault_ensured: Arc<AtomicBool>,
}

pub struct ObsidianMcp {
    vault: Vault,
    hybrid_alpha: f32,
    semantic_runtime: SemanticRuntime,
    #[allow(dead_code)]
    pub tool_router: ToolRouter<Self>,
}

#[tool_router]
impl ObsidianMcp {
    pub fn new(
        vault: Vault,
        hybrid_alpha: f32,
        semantic_runtime: SemanticRuntime,
        disabled_tools: HashSet<String>,
    ) -> Self {
        let mut tool_router = Self::tool_router();
        if !disabled_tools.is_empty() {
            tracing::info!(
                count = disabled_tools.len(),
                "disabling tools per filter config"
            );
            for name in disabled_tools {
                tool_router.disable_route(name);
            }
        }
        Self {
            tool_router,
            vault,
            hybrid_alpha,
            semantic_runtime,
        }
    }

    // ── Navigation ──────────────────────────────────────────────────

    #[tool(
        name = "vault_list",
        description = "List files and directories in the vault. Supports recursive listing, glob filtering, and tree view (format: \"tree\"). Returns a JSON array of paths (list mode) or a tree-formatted string (tree mode)."
    )]
    async fn vault_list(
        &self,
        Parameters(params): Parameters<navigation::VaultListParams>,
    ) -> Result<CallToolResult, ErrorData> {
        navigation::vault_list(&self.vault, params)
    }

    // ── Note CRUD ───────────────────────────────────────────────────

    #[tool(
        name = "note_read",
        description = "Read the full content of a note. Returns the raw markdown including frontmatter."
    )]
    async fn note_read(
        &self,
        Parameters(params): Parameters<notes::NoteReadParams>,
    ) -> Result<String, ErrorData> {
        notes::note_read(&self.vault, params).await
    }

    #[tool(
        name = "note_create",
        description = "Create a new note with optional content and YAML frontmatter. Parent directories are created automatically. Fails if the note already exists."
    )]
    async fn note_create(
        &self,
        Parameters(params): Parameters<notes::NoteCreateParams>,
    ) -> Result<String, ErrorData> {
        notes::note_create(&self.vault, params).await
    }

    #[tool(
        name = "note_write",
        description = "Overwrite a note's entire content. The note must already exist."
    )]
    async fn note_write(
        &self,
        Parameters(params): Parameters<notes::NoteWriteParams>,
    ) -> Result<String, ErrorData> {
        notes::note_write(&self.vault, params).await
    }

    #[tool(
        name = "note_insert",
        description = "Insert content into an existing note. \
            Position: \"end\" (default) appends after existing content; \
            \"beginning\" inserts after frontmatter (or at the very start if none)."
    )]
    async fn note_insert(
        &self,
        Parameters(params): Parameters<notes::NoteInsertParams>,
    ) -> Result<String, ErrorData> {
        notes::note_insert(&self.vault, params).await
    }

    #[tool(
        name = "note_patch",
        description = "Patch a specific section of a note by targeting a heading, block reference, or frontmatter field. Supports append, prepend, and replace operations."
    )]
    async fn note_patch(
        &self,
        Parameters(params): Parameters<notes::NotePatchParams>,
    ) -> Result<String, ErrorData> {
        notes::note_patch(&self.vault, params).await
    }

    #[tool(
        name = "note_delete",
        description = "Delete a note from the vault. Requires `confirm: true` as a safety check to prevent accidental data loss."
    )]
    async fn note_delete(
        &self,
        Parameters(params): Parameters<notes::NoteDeleteParams>,
    ) -> Result<String, ErrorData> {
        notes::note_delete(&self.vault, params).await
    }

    #[tool(
        name = "note_move",
        description = "Move or rename a note. Parent directories at the destination are created automatically."
    )]
    async fn note_move(
        &self,
        Parameters(params): Parameters<notes::NoteMoveParams>,
    ) -> Result<String, ErrorData> {
        notes::note_move(&self.vault, params).await
    }

    // ── Search ──────────────────────────────────────────────────────

    #[tool(
        name = "search_text",
        description = "BM25-ranked full-text search across all notes. Returns matching files with relevance scores and context snippets. Supports stemming (e.g. 'program' matches 'programming'), optional fuzzy matching for typo tolerance, and field-level filtering."
    )]
    async fn search_text(
        &self,
        Parameters(params): Parameters<search::SearchTextParams>,
    ) -> Result<CallToolResult, ErrorData> {
        search::search_text(&self.vault, params).await
    }

    #[tool(
        name = "search_regex",
        description = "Search across all notes using a regular expression pattern. Returns matching files with context snippets."
    )]
    async fn search_regex(
        &self,
        Parameters(params): Parameters<search::SearchRegexParams>,
    ) -> Result<CallToolResult, ErrorData> {
        search::search_regex(&self.vault, params).await
    }

    #[tool(
        name = "search_metadata",
        description = "Search notes by metadata. Set type=\"tag\" to find notes with a specific tag (both inline #tags and frontmatter tags), or type=\"frontmatter\" to query by frontmatter field value. For tags: provide `tag` (required) and optional `include_nested`. For frontmatter: provide `field` (required), optional `operator` (eq/contains/exists), and `value` (required for eq/contains)."
    )]
    async fn search_metadata(
        &self,
        Parameters(params): Parameters<search::SearchMetadataParams>,
    ) -> Result<CallToolResult, ErrorData> {
        search::search_metadata(&self.vault, params).await
    }

    #[tool(
        name = "search_semantic",
        description = "Semantic search using daemon-backed runtime (preferred) with local compatibility fallback based on OBSIDIAN_SEMANTIC_MODE. Finds conceptually related notes without requiring exact keyword matches."
    )]
    async fn search_semantic(
        &self,
        Parameters(params): Parameters<search::SearchSemanticParams>,
    ) -> Result<CallToolResult, ErrorData> {
        search::search_semantic(
            &self.vault,
            params,
            self.hybrid_alpha,
            &self.semantic_runtime,
        )
        .await
    }

    // ── Metadata ────────────────────────────────────────────────────

    #[tool(
        name = "note_inspect",
        description = "Inspect a note. Views: \"metadata\" (default) returns tags, headings, outgoing links, block refs, backlinks count, frontmatter, and file stats. \"targets\" lists patchable headings, block refs, and frontmatter fields (use before note_patch)."
    )]
    async fn note_inspect(
        &self,
        Parameters(params): Parameters<metadata::NoteInspectParams>,
    ) -> Result<CallToolResult, ErrorData> {
        metadata::note_inspect(&self.vault, params).await
    }

    #[tool(
        name = "frontmatter",
        description = "Read, set, or remove frontmatter fields on a note. Actions: \"get\" returns all frontmatter as JSON (or null), \"set\" upserts a field (requires key + value), \"remove\" deletes a field (requires key)."
    )]
    async fn frontmatter(
        &self,
        Parameters(params): Parameters<metadata::FrontmatterParams>,
    ) -> Result<CallToolResult, ErrorData> {
        metadata::frontmatter(&self.vault, params).await
    }

    // ── Graph / Links ───────────────────────────────────────────────

    #[tool(
        name = "wikilinks",
        description = "Query the vault's wikilink graph. Queries: \"backlinks\" (requires path) finds notes linking TO a note, \"outgoing\" (requires path) finds links FROM a note with resolution status, \"broken\" (optional path) finds unresolved wikilinks, \"orphans\" finds disconnected notes."
    )]
    async fn wikilinks(
        &self,
        Parameters(params): Parameters<graph::WikilinksParams>,
    ) -> Result<CallToolResult, ErrorData> {
        graph::wikilinks(&self.vault, params).await
    }

    // ── Periodic Notes ──────────────────────────────────────────────

    #[tool(
        name = "periodic",
        description = "Manage periodic notes (daily, weekly, monthly, quarterly, yearly). \
            Actions: \"get\" — read note content (params: period, date?); \
            \"create\" — create from template or custom content (params: period, date?, content?); \
            \"list\" — list recent notes newest-first (params: period, limit?)."
    )]
    async fn periodic(
        &self,
        Parameters(params): Parameters<periodic::PeriodicParams>,
    ) -> Result<String, ErrorData> {
        periodic::periodic(&self.vault, params).await
    }

    // ── Utility ─────────────────────────────────────────────────────

    #[tool(
        name = "vault_info",
        description = "Return aggregate vault statistics: total notes, files, tags, links, and vault size in bytes."
    )]
    async fn vault_info(
        &self,
        Parameters(params): Parameters<utility::VaultInfoParams>,
    ) -> Result<CallToolResult, ErrorData> {
        utility::vault_info(&self.vault, params).await
    }

    #[tool(
        name = "open_in_obsidian",
        description = "Open a note in the Obsidian desktop app via the obsidian:// URI scheme. Requires Obsidian to be installed."
    )]
    async fn open_in_obsidian(
        &self,
        Parameters(params): Parameters<utility::OpenInObsidianParams>,
    ) -> Result<CallToolResult, ErrorData> {
        utility::open_in_obsidian(&self.vault, params).await
    }
}

#[tool_handler]
impl ServerHandler for ObsidianMcp {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new(
                env!("CARGO_PKG_NAME"),
                env!("CARGO_PKG_VERSION"),
            ))
            .with_instructions(
                "Obsidian vault MCP server. Provides tools to read, write, search, \
                 and navigate your Obsidian notes via direct filesystem access.",
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ALL_TOOL_NAMES;
    use crate::test_helpers::{create_test_vault, test_config};
    use crate::vault::Vault;

    fn test_runtime() -> SemanticRuntime {
        SemanticRuntime {
            mode: SemanticMode::Local,
            daemon_client: None,
            daemon_unavailable_reason: None,
            prefetch_count: 50,
            vault_ensured: Arc::new(AtomicBool::new(false)),
        }
    }

    #[tokio::test]
    async fn no_disabled_tools_exposes_all() {
        let tmp = tempfile::tempdir().unwrap();
        create_test_vault(tmp.path());
        let vault = Vault::open(&test_config(tmp.path())).await.unwrap();
        let server = ObsidianMcp::new(vault, 0.25, test_runtime(), HashSet::new());

        for name in ALL_TOOL_NAMES {
            assert!(
                server.tool_router.has_route(name),
                "expected tool '{name}' to be enabled"
            );
        }
    }

    #[tokio::test]
    async fn disabled_tools_are_hidden() {
        let tmp = tempfile::tempdir().unwrap();
        create_test_vault(tmp.path());
        let vault = Vault::open(&test_config(tmp.path())).await.unwrap();

        let disabled: HashSet<String> = ["open_in_obsidian", "wikilinks", "periodic"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let server = ObsidianMcp::new(vault, 0.25, test_runtime(), disabled);

        assert!(!server.tool_router.has_route("open_in_obsidian"));
        assert!(!server.tool_router.has_route("wikilinks"));
        assert!(!server.tool_router.has_route("periodic"));

        assert!(server.tool_router.has_route("note_read"));
        assert!(server.tool_router.has_route("vault_list"));
        assert!(server.tool_router.has_route("search_text"));
    }

    #[tokio::test]
    async fn disable_all_tools_hides_everything() {
        let tmp = tempfile::tempdir().unwrap();
        create_test_vault(tmp.path());
        let vault = Vault::open(&test_config(tmp.path())).await.unwrap();

        let disabled: HashSet<String> = ALL_TOOL_NAMES.iter().map(|s| s.to_string()).collect();
        let server = ObsidianMcp::new(vault, 0.25, test_runtime(), disabled);

        for name in ALL_TOOL_NAMES {
            assert!(
                !server.tool_router.has_route(name),
                "expected tool '{name}' to be disabled"
            );
        }
    }
}
