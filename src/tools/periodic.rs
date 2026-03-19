//! Daily, weekly, and periodic note tools.

use chrono::NaiveDate;
use rmcp::model::ErrorCode;
use schemars::JsonSchema;
use serde::Deserialize;

use crate::models::NotePeriod;
use crate::vault::Vault;

fn parse_date(date_str: &str) -> Result<NaiveDate, rmcp::ErrorData> {
    NaiveDate::parse_from_str(date_str, "%Y-%m-%d").map_err(|_| {
        rmcp::ErrorData::new(
            ErrorCode::INVALID_PARAMS,
            format!("Invalid date '{date_str}'; expected YYYY-MM-DD"),
            None,
        )
    })
}

// ── periodic_get ────────────────────────────────────────────────────

#[derive(Deserialize, JsonSchema, Default)]
pub struct PeriodicGetParams {
    /// Period type: daily, weekly, monthly, quarterly, yearly.
    pub period: NotePeriod,
    /// ISO date (YYYY-MM-DD). Defaults to today if omitted.
    #[serde(default)]
    pub date: Option<String>,
}

/// Read the content of a periodic note for the given period and date.
pub async fn periodic_get(
    vault: &Vault,
    params: PeriodicGetParams,
) -> Result<String, rmcp::ErrorData> {
    let date = params.date.map(|s| parse_date(&s)).transpose()?;
    Ok(vault.get_periodic_note(&params.period, date)?)
}

// ── periodic_create ─────────────────────────────────────────────────

#[derive(Deserialize, JsonSchema, Default)]
pub struct PeriodicCreateParams {
    /// Period type: daily, weekly, monthly, quarterly, yearly.
    pub period: NotePeriod,
    /// ISO date (YYYY-MM-DD). Defaults to today if omitted.
    #[serde(default)]
    pub date: Option<String>,
    /// Custom content; overrides configured template expansion.
    #[serde(default)]
    pub content: Option<String>,
}

/// Create a periodic note, optionally with custom content instead of the template.
pub async fn periodic_create(
    vault: &Vault,
    params: PeriodicCreateParams,
) -> Result<String, rmcp::ErrorData> {
    let date = params.date.map(|s| parse_date(&s)).transpose()?;
    let path = vault.create_periodic_note(&params.period, date, params.content.as_deref())?;

    Ok(format!("Created: {}", path.display()))
}

// ── periodic_list_recent ────────────────────────────────────────────

#[derive(Deserialize, JsonSchema, Default)]
pub struct PeriodicListRecentParams {
    /// Period type: daily, weekly, monthly, quarterly, yearly.
    pub period: NotePeriod,
    /// Maximum number of notes to return (default: 10).
    #[serde(default)]
    pub limit: Option<usize>,
}

/// List recent periodic notes sorted newest-first.
pub async fn periodic_list_recent(
    vault: &Vault,
    params: PeriodicListRecentParams,
) -> Result<String, rmcp::ErrorData> {
    let limit = params.limit.unwrap_or(10);
    let paths = vault.list_recent_periodic_notes(&params.period, limit)?;

    let items: Vec<serde_json::Value> = paths
        .into_iter()
        .map(|p| {
            let date = p
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or_default()
                .to_string();
            serde_json::json!({ "path": p.to_string_lossy(), "date": date })
        })
        .collect();

    serde_json::to_string_pretty(&items)
        .map_err(|e| rmcp::ErrorData::new(ErrorCode::INTERNAL_ERROR, e.to_string(), None))
}
