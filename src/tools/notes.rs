//! Note CRUD tools: read, create, edit, delete, rename, move.

use std::path::Path;

use rmcp::ErrorData;
use rmcp::model::ErrorCode;
use schemars::JsonSchema;
use serde::Deserialize;

use crate::models::{PatchOperation, PatchRequest, PatchTargetType};
use crate::vault::Vault;

// ── Parameter structs ───────────────────────────────────────────────

#[derive(Deserialize, JsonSchema, Default)]
pub struct NoteReadParams {
    /// Path to the note, relative to vault root (e.g. "folder/note.md").
    pub path: String,
}

#[derive(Deserialize, JsonSchema, Default)]
pub struct NoteCreateParams {
    /// Path for the new note, relative to vault root. Parent dirs are created automatically.
    pub path: String,
    /// Initial body content. Defaults to empty.
    #[serde(default)]
    pub content: Option<String>,
    /// Optional YAML frontmatter as a JSON object (e.g. `{"tags": ["rust"], "draft": true}`).
    #[serde(default)]
    pub frontmatter: Option<serde_json::Value>,
}

#[derive(Deserialize, JsonSchema, Default)]
pub struct NoteWriteParams {
    /// Path to the note, relative to vault root.
    pub path: String,
    /// New content that replaces the entire note.
    pub content: String,
}

#[derive(Deserialize, JsonSchema, Default)]
pub struct NoteAppendParams {
    /// Path to the note, relative to vault root.
    pub path: String,
    /// Content to append at the end of the note.
    pub content: String,
}

#[derive(Deserialize, JsonSchema, Default)]
pub struct NotePrependParams {
    /// Path to the note, relative to vault root.
    pub path: String,
    /// Content to insert after frontmatter (or at the very start if no frontmatter).
    pub content: String,
}

#[derive(Deserialize, JsonSchema)]
pub struct NotePatchParams {
    /// Path to the note, relative to vault root.
    pub path: String,
    /// Patch operation: `append`, `prepend`, or `replace`.
    pub operation: PatchOperation,
    /// Target type: `heading`, `block`, or `frontmatter`.
    pub target_type: PatchTargetType,
    /// Target identifier — heading text, block ID, or frontmatter field name.
    pub target: String,
    /// Content to insert or replace with.
    pub content: String,
}

impl Default for NotePatchParams {
    fn default() -> Self {
        Self {
            path: String::new(),
            operation: PatchOperation::Append,
            target_type: PatchTargetType::Heading,
            target: String::new(),
            content: String::new(),
        }
    }
}

#[derive(Deserialize, JsonSchema, Default)]
pub struct NoteDeleteParams {
    /// Path to the note, relative to vault root.
    pub path: String,
    /// Must be `true` to confirm deletion — a safety check to prevent accidental data loss.
    pub confirm: bool,
}

#[derive(Deserialize, JsonSchema, Default)]
pub struct NoteMoveParams {
    /// Current path of the note, relative to vault root.
    pub from: String,
    /// Destination path, relative to vault root.
    pub to: String,
}

// ── Handler functions ───────────────────────────────────────────────

pub async fn note_read(vault: &Vault, params: NoteReadParams) -> Result<String, ErrorData> {
    Ok(vault.read_note(Path::new(&params.path))?)
}

pub async fn note_create(vault: &Vault, params: NoteCreateParams) -> Result<String, ErrorData> {
    vault.create_note(
        Path::new(&params.path),
        params.content.as_deref().unwrap_or(""),
        params.frontmatter.as_ref(),
    )?;
    Ok(format!("Created note: {}", params.path))
}

pub async fn note_write(vault: &Vault, params: NoteWriteParams) -> Result<String, ErrorData> {
    vault.write_note(Path::new(&params.path), &params.content)?;
    Ok(format!("Written to: {}", params.path))
}

pub async fn note_append(vault: &Vault, params: NoteAppendParams) -> Result<String, ErrorData> {
    vault.append_note(Path::new(&params.path), &params.content)?;
    Ok(format!("Appended to: {}", params.path))
}

pub async fn note_prepend(vault: &Vault, params: NotePrependParams) -> Result<String, ErrorData> {
    vault.prepend_note(Path::new(&params.path), &params.content)?;
    Ok(format!("Prepended to: {}", params.path))
}

pub async fn note_patch(vault: &Vault, params: NotePatchParams) -> Result<String, ErrorData> {
    let request = PatchRequest {
        operation: params.operation,
        target_type: params.target_type,
        target: params.target,
        content: params.content,
    };
    vault.patch_note(Path::new(&params.path), &request)?;
    Ok(format!("Patched: {}", params.path))
}

pub async fn note_delete(vault: &Vault, params: NoteDeleteParams) -> Result<String, ErrorData> {
    if !params.confirm {
        return Err(ErrorData::new(
            ErrorCode::INVALID_PARAMS,
            "Deletion requires `confirm: true` as a safety check",
            None::<serde_json::Value>,
        ));
    }
    vault.delete_note(Path::new(&params.path))?;
    Ok(format!("Deleted: {}", params.path))
}

pub async fn note_move(vault: &Vault, params: NoteMoveParams) -> Result<String, ErrorData> {
    let new_path = vault.move_note(Path::new(&params.from), Path::new(&params.to))?;
    Ok(format!("Moved to: {}", new_path.display()))
}
