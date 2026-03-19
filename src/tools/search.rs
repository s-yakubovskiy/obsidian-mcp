//! Text, regex, tag, and frontmatter search tools across vault notes.

use schemars::JsonSchema;
use serde::Deserialize;

use crate::error::{VaultError, VaultResult};
use crate::vault::Vault;

// ── search_text ─────────────────────────────────────────────────────

#[derive(Deserialize, JsonSchema, Default)]
pub struct SearchTextParams {
    /// Full-text search query (case-insensitive).
    pub query: String,
    /// Characters of context around each match (default: 100).
    #[serde(default)]
    pub context_length: Option<usize>,
    /// Maximum number of file results to return (default: 20).
    #[serde(default)]
    pub max_results: Option<usize>,
}

pub fn handle_search_text(vault: &Vault, params: SearchTextParams) -> VaultResult<String> {
    let context_length = params.context_length.unwrap_or(100);
    let max_results = params.max_results.unwrap_or(20);

    let results = vault.search_text(&params.query, context_length)?;
    let limited: Vec<_> = results.into_iter().take(max_results).collect();

    serde_json::to_string_pretty(&limited)
        .map_err(|e| VaultError::Other(format!("JSON serialization failed: {e}")))
}

// ── search_regex ────────────────────────────────────────────────────

#[derive(Deserialize, JsonSchema, Default)]
pub struct SearchRegexParams {
    /// Regular expression pattern to search for.
    pub pattern: String,
    /// Characters of context around each match (default: 100).
    #[serde(default)]
    pub context_length: Option<usize>,
    /// Maximum number of file results to return (default: 20).
    #[serde(default)]
    pub max_results: Option<usize>,
}

pub fn handle_search_regex(vault: &Vault, params: SearchRegexParams) -> VaultResult<String> {
    let context_length = params.context_length.unwrap_or(100);
    let max_results = params.max_results.unwrap_or(20);

    let results = vault.search_regex(&params.pattern, context_length)?;
    let limited: Vec<_> = results.into_iter().take(max_results).collect();

    serde_json::to_string_pretty(&limited)
        .map_err(|e| VaultError::Other(format!("JSON serialization failed: {e}")))
}

// ── search_tag ──────────────────────────────────────────────────────

#[derive(Deserialize, JsonSchema, Default)]
pub struct SearchTagParams {
    /// Tag to search for (without the `#` prefix).
    pub tag: String,
    /// If true, also match nested tags (e.g. `inbox` matches `inbox/read`). Default: true.
    #[serde(default)]
    pub include_nested: Option<bool>,
}

pub fn handle_search_tag(vault: &Vault, params: SearchTagParams) -> VaultResult<String> {
    let tag = params.tag.strip_prefix('#').unwrap_or(&params.tag);
    let include_nested = params.include_nested.unwrap_or(true);

    let results = if include_nested {
        vault.search_by_tag_prefix(tag)?
    } else {
        vault.search_by_tag(tag)?
    };

    serde_json::to_string_pretty(&results)
        .map_err(|e| VaultError::Other(format!("JSON serialization failed: {e}")))
}

// ── search_frontmatter ─────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "snake_case")]
pub enum FrontmatterOperator {
    /// Exact equality (or array-contains for list fields).
    #[default]
    Eq,
    /// Substring match for strings; element membership for arrays.
    Contains,
    /// Field exists regardless of value.
    Exists,
}

#[derive(Deserialize, JsonSchema, Default)]
pub struct SearchFrontmatterParams {
    /// Frontmatter field name to query.
    pub field: String,
    /// Value to compare against. Required for `eq` and `contains`; ignored for `exists`.
    #[serde(default)]
    pub value: Option<serde_json::Value>,
    /// Comparison operator (default: `eq`).
    #[serde(default)]
    pub operator: FrontmatterOperator,
}

pub fn handle_search_frontmatter(
    vault: &Vault,
    params: SearchFrontmatterParams,
) -> VaultResult<String> {
    let results = match params.operator {
        FrontmatterOperator::Exists => vault.search_frontmatter_exists(&params.field)?,
        FrontmatterOperator::Eq => {
            let value = params.value.ok_or_else(|| {
                VaultError::InvalidPath("'value' is required for 'eq' operator".into())
            })?;
            vault.search_frontmatter(&params.field, &value)?
        }
        FrontmatterOperator::Contains => {
            let value = params.value.ok_or_else(|| {
                VaultError::InvalidPath("'value' is required for 'contains' operator".into())
            })?;
            vault.search_frontmatter_contains(&params.field, &value)?
        }
    };

    serde_json::to_string_pretty(&results)
        .map_err(|e| VaultError::Other(format!("JSON serialization failed: {e}")))
}

// ── tests ───────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use std::path::Path;

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
    }

    async fn setup_search_vault() -> (tempfile::TempDir, Vault) {
        let dir = tempfile::tempdir().unwrap();
        create_test_vault(dir.path());

        let vault = Vault::open(&test_config(dir.path())).await.unwrap();

        vault
            .write_note(
                Path::new("rust.md"),
                "---\ntags: [lang, systems]\nstatus: stable\n---\n# Rust\nRust is a systems language.\n",
            )
            .unwrap();
        vault
            .write_note(
                Path::new("python.md"),
                "---\ntags: [lang, scripting]\nstatus: in progress\n---\n# Python\nPython is dynamic.\n",
            )
            .unwrap();
        vault
            .write_note(
                Path::new("notes.md"),
                "# Notes\nSome random notes about #inbox stuff.\n\n#inbox/read #inbox/todo\n",
            )
            .unwrap();
        vault
            .write_note(
                Path::new("empty.md"),
                "# Empty\nNothing interesting here.\n",
            )
            .unwrap();

        (dir, vault)
    }

    // ── search_text ─────────────────────────────────────────────────

    #[tokio::test]
    async fn search_text_finds_match() {
        let (_dir, vault) = setup_search_vault().await;
        let result = handle_search_text(
            &vault,
            SearchTextParams {
                query: "systems".into(),
                ..Default::default()
            },
        )
        .unwrap();

        assert!(result.contains("rust.md"));
        assert!(!result.contains("python.md"));
    }

    #[tokio::test]
    async fn search_text_limits_results() {
        let (_dir, vault) = setup_search_vault().await;
        let result = handle_search_text(
            &vault,
            SearchTextParams {
                query: "is".into(),
                max_results: Some(1),
                ..Default::default()
            },
        )
        .unwrap();

        let parsed: Vec<serde_json::Value> = serde_json::from_str(&result).unwrap();
        assert_eq!(parsed.len(), 1);
    }

    #[tokio::test]
    async fn search_text_empty_query_returns_empty() {
        let (_dir, vault) = setup_search_vault().await;
        let result = handle_search_text(
            &vault,
            SearchTextParams {
                query: String::new(),
                ..Default::default()
            },
        )
        .unwrap();

        let parsed: Vec<serde_json::Value> = serde_json::from_str(&result).unwrap();
        assert!(parsed.is_empty());
    }

    // ── search_regex ────────────────────────────────────────────────

    #[tokio::test]
    async fn search_regex_valid_pattern() {
        let (_dir, vault) = setup_search_vault().await;
        let result = handle_search_regex(
            &vault,
            SearchRegexParams {
                pattern: r"(?i)python".into(),
                ..Default::default()
            },
        )
        .unwrap();

        assert!(result.contains("python.md"));
    }

    #[tokio::test]
    async fn search_regex_invalid_pattern_returns_error() {
        let (_dir, vault) = setup_search_vault().await;
        let result = handle_search_regex(
            &vault,
            SearchRegexParams {
                pattern: "[invalid".into(),
                ..Default::default()
            },
        );

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn search_regex_limits_results() {
        let (_dir, vault) = setup_search_vault().await;
        let result = handle_search_regex(
            &vault,
            SearchRegexParams {
                pattern: r"\w+".into(),
                max_results: Some(2),
                ..Default::default()
            },
        )
        .unwrap();

        let parsed: Vec<serde_json::Value> = serde_json::from_str(&result).unwrap();
        assert!(parsed.len() <= 2);
    }

    // ── search_tag ──────────────────────────────────────────────────

    #[tokio::test]
    async fn search_tag_exact() {
        let (_dir, vault) = setup_search_vault().await;
        let result = handle_search_tag(
            &vault,
            SearchTagParams {
                tag: "inbox".into(),
                include_nested: Some(false),
                ..Default::default()
            },
        )
        .unwrap();

        assert!(result.contains("notes.md"));
        let parsed: Vec<serde_json::Value> = serde_json::from_str(&result).unwrap();
        assert_eq!(parsed.len(), 1);
    }

    #[tokio::test]
    async fn search_tag_include_nested() {
        let (_dir, vault) = setup_search_vault().await;
        let result = handle_search_tag(
            &vault,
            SearchTagParams {
                tag: "inbox".into(),
                include_nested: Some(true),
                ..Default::default()
            },
        )
        .unwrap();

        assert!(result.contains("notes.md"));
    }

    #[tokio::test]
    async fn search_tag_strips_hash_prefix() {
        let (_dir, vault) = setup_search_vault().await;
        let result = handle_search_tag(
            &vault,
            SearchTagParams {
                tag: "#lang".into(),
                include_nested: Some(false),
                ..Default::default()
            },
        )
        .unwrap();

        let parsed: Vec<serde_json::Value> = serde_json::from_str(&result).unwrap();
        assert_eq!(parsed.len(), 2);
    }

    // ── search_frontmatter ──────────────────────────────────────────

    #[tokio::test]
    async fn search_frontmatter_eq() {
        let (_dir, vault) = setup_search_vault().await;
        let result = handle_search_frontmatter(
            &vault,
            SearchFrontmatterParams {
                field: "status".into(),
                value: Some(serde_json::json!("stable")),
                operator: FrontmatterOperator::Eq,
            },
        )
        .unwrap();

        assert!(result.contains("rust.md"));
        let parsed: Vec<serde_json::Value> = serde_json::from_str(&result).unwrap();
        assert_eq!(parsed.len(), 1);
    }

    #[tokio::test]
    async fn search_frontmatter_eq_array_contains() {
        let (_dir, vault) = setup_search_vault().await;
        let result = handle_search_frontmatter(
            &vault,
            SearchFrontmatterParams {
                field: "tags".into(),
                value: Some(serde_json::json!("systems")),
                operator: FrontmatterOperator::Eq,
            },
        )
        .unwrap();

        assert!(result.contains("rust.md"));
    }

    #[tokio::test]
    async fn search_frontmatter_contains_substring() {
        let (_dir, vault) = setup_search_vault().await;
        let result = handle_search_frontmatter(
            &vault,
            SearchFrontmatterParams {
                field: "status".into(),
                value: Some(serde_json::json!("progress")),
                operator: FrontmatterOperator::Contains,
            },
        )
        .unwrap();

        assert!(result.contains("python.md"));
        let parsed: Vec<serde_json::Value> = serde_json::from_str(&result).unwrap();
        assert_eq!(parsed.len(), 1);
    }

    #[tokio::test]
    async fn search_frontmatter_exists() {
        let (_dir, vault) = setup_search_vault().await;
        let result = handle_search_frontmatter(
            &vault,
            SearchFrontmatterParams {
                field: "status".into(),
                value: None,
                operator: FrontmatterOperator::Exists,
            },
        )
        .unwrap();

        let parsed: Vec<serde_json::Value> = serde_json::from_str(&result).unwrap();
        assert_eq!(parsed.len(), 2); // rust.md + python.md
    }

    #[tokio::test]
    async fn search_frontmatter_exists_missing_field() {
        let (_dir, vault) = setup_search_vault().await;
        let result = handle_search_frontmatter(
            &vault,
            SearchFrontmatterParams {
                field: "nonexistent".into(),
                value: None,
                operator: FrontmatterOperator::Exists,
            },
        )
        .unwrap();

        let parsed: Vec<serde_json::Value> = serde_json::from_str(&result).unwrap();
        assert!(parsed.is_empty());
    }

    #[tokio::test]
    async fn search_frontmatter_eq_without_value_errors() {
        let (_dir, vault) = setup_search_vault().await;
        let result = handle_search_frontmatter(
            &vault,
            SearchFrontmatterParams {
                field: "status".into(),
                value: None,
                operator: FrontmatterOperator::Eq,
            },
        );

        assert!(result.is_err());
    }
}
