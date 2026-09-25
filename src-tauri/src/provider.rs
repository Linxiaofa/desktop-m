//! AI provider metadata and Windows Credential Manager backed API keys.
//!
//! This module intentionally has no filesystem or shell access. Provider metadata is
//! stored in SQLite; secret values are only stored in the operating system vault.

use chrono::Utc;
use keyring::{Entry, Error as KeyringError};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use url::Url;
use uuid::Uuid;

const CREDENTIAL_SERVICE: &str = "desktop-manager.ai-provider";

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum ProviderKind {
    #[serde(rename = "openai")]
    OpenAi,
    #[serde(rename = "deepseek")]
    DeepSeek,
    #[serde(rename = "xiaomi_mimo")]
    XiaomiMimo,
    #[serde(rename = "openai_compatible")]
    OpenAiCompatible,
    #[serde(rename = "custom")]
    Custom,
}

impl ProviderKind {
    fn as_db(self) -> &'static str {
        match self {
            Self::OpenAi => "openai",
            Self::DeepSeek => "deepseek",
            Self::XiaomiMimo => "xiaomi_mimo",
            Self::OpenAiCompatible => "openai_compatible",
            Self::Custom => "custom",
        }
    }

    fn from_db(value: &str) -> Result<Self, ProviderError> {
        match value {
            "openai" => Ok(Self::OpenAi),
            "deepseek" => Ok(Self::DeepSeek),
            "xiaomi_mimo" => Ok(Self::XiaomiMimo),
            "openai_compatible" => Ok(Self::OpenAiCompatible),
            "custom" => Ok(Self::Custom),
            _ => Err(ProviderError::InvalidStoredKind),
        }
    }
}

/// Input from a trusted application command. `api_key` is write-only: this type
/// deliberately implements Deserialize but not Serialize or Debug.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderInput {
    pub id: Option<String>,
    pub kind: ProviderKind,
    pub name: String,
    pub base_url: String,
    pub model: String,
    #[serde(default)]
    pub allow_local_http: bool,
    /// `None` keeps the existing key when updating a provider.
    pub api_key: Option<String>,
    /// Remove a previously saved key. Cannot be combined with `api_key`.
    #[serde(default)]
    pub clear_api_key: bool,
}

/// Safe to serialize to the UI. No secret, credential target, or vault data is
/// included in this structure.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderConfig {
    pub id: String,
    pub kind: ProviderKind,
    pub name: String,
    pub base_url: String,
    pub model: String,
    pub allow_local_http: bool,
    pub has_api_key: bool,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Error)]
pub enum ProviderError {
    #[error("invalid provider id")]
    InvalidId,
    #[error("invalid stored provider kind")]
    InvalidStoredKind,
    #[error("{0} is required")]
    Required(&'static str),
    #[error("invalid provider base URL; use HTTPS or HTTP on loopback")]
    InvalidBaseUrl,
    #[error("API key and clear_api_key cannot both be set")]
    ConflictingKeyChange,
    #[error("provider not found")]
    NotFound,
    #[error("AI provider credentials require Windows")]
    UnsupportedPlatform,
    #[error("credential manager operation failed")]
    Credential(#[source] KeyringError),
    #[error("database operation failed")]
    Database(#[from] rusqlite::Error),
    #[error("credential manager recovery failed after database failure")]
    RecoveryFailed,
}

pub type ProviderResult<T> = Result<T, ProviderError>;

/// Creates the non-secret metadata table. A provider's API key is never stored
/// here, including in a hash, reference URL, or serialized settings blob.
pub fn init_schema(conn: &Connection) -> ProviderResult<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS ai_providers (
            id TEXT PRIMARY KEY,
            kind TEXT NOT NULL CHECK (kind IN (
                'openai', 'deepseek', 'xiaomi_mimo', 'openai_compatible', 'custom'
            )),
            name TEXT NOT NULL,
            base_url TEXT NOT NULL,
            model TEXT NOT NULL,
            allow_local_http INTEGER NOT NULL DEFAULT 0,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        );",
    )?;
    let mut statement = conn.prepare("PRAGMA table_info(ai_providers)")?;
    let columns = statement.query_map([], |row| row.get::<_, String>(1))?;
    let mut has_local_http = false;
    for column in columns {
        if column? == "allow_local_http" {
            has_local_http = true;
        }
    }
    if !has_local_http {
        conn.execute(
            "ALTER TABLE ai_providers ADD COLUMN allow_local_http INTEGER NOT NULL DEFAULT 0",
            [],
        )?;
    }
    Ok(())
}

pub fn list(conn: &Connection) -> ProviderResult<Vec<ProviderConfig>> {
    let mut statement = conn.prepare(
        "SELECT id, kind, name, base_url, model, allow_local_http, created_at, updated_at
         FROM ai_providers ORDER BY name COLLATE NOCASE, id",
    )?;
    let rows = statement.query_map([], |row| {
        Ok(ProviderRow {
            id: row.get(0)?,
            kind: row.get(1)?,
            name: row.get(2)?,
            base_url: row.get(3)?,
            model: row.get(4)?,
            allow_local_http: row.get(5)?,
            created_at: row.get(6)?,
            updated_at: row.get(7)?,
        })
    })?;

    let mut providers = Vec::new();
    for row in rows {
        providers.push(row?.into_config()?);
    }
    Ok(providers)
}

pub(crate) fn get(conn: &Connection, id: &str) -> ProviderResult<ProviderConfig> {
    validate_id(id)?;
    find(conn, id)?.ok_or(ProviderError::NotFound)
}

pub fn upsert(conn: &Connection, input: ProviderInput) -> ProviderResult<ProviderConfig> {
    let id = match input.id {
        Some(id) => {
            validate_id(&id)?;
            id
        }
        None => Uuid::new_v4().to_string(),
    };
    let name = required(input.name, "name")?;
    let model = required(input.model, "model")?;
    let base_url = normalize_base_url(input.base_url, input.allow_local_http)?;

    if input.clear_api_key && input.api_key.is_some() {
        return Err(ProviderError::ConflictingKeyChange);
    }
    let key_change = match input.api_key {
        Some(value) => KeyChange::Set(required(value, "apiKey")?),
        None if input.clear_api_key => KeyChange::Clear,
        None => KeyChange::None,
    };

    // SQLite and Credential Manager cannot share an atomic transaction. Keep
    // SQLite open until the vault operation succeeds, then compensate if the
    // database commit fails. The application serializes provider writes.
    let entry = credential_entry(&id)?;
    let old_key = if key_change.is_change() {
        optional_password(&entry)?
    } else {
        None
    };
    let transaction = conn.unchecked_transaction()?;
    let now = Utc::now().to_rfc3339();
    transaction.execute(
        "INSERT INTO ai_providers (id, kind, name, base_url, model, allow_local_http, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7)
         ON CONFLICT(id) DO UPDATE SET
           kind = excluded.kind,
           name = excluded.name,
           base_url = excluded.base_url,
           model = excluded.model,
           allow_local_http = excluded.allow_local_http,
           updated_at = excluded.updated_at",
        params![id, input.kind.as_db(), name, base_url, model, input.allow_local_http, now],
    )?;

    match &key_change {
        KeyChange::Set(secret) => entry
            .set_password(secret)
            .map_err(ProviderError::Credential)?,
        KeyChange::Clear => delete_password_if_present(&entry)?,
        KeyChange::None => {}
    }

    if let Err(error) = transaction.commit() {
        if key_change.is_change() {
            restore_password(&entry, old_key.as_deref())?;
        }
        return Err(ProviderError::Database(error));
    }
    find(conn, &id)?.ok_or(ProviderError::NotFound)
}

pub fn delete(conn: &Connection, id: &str) -> ProviderResult<()> {
    validate_id(id)?;
    let entry = credential_entry(id)?;
    let old_key = optional_password(&entry)?;
    let transaction = conn.unchecked_transaction()?;
    let changed = transaction.execute("DELETE FROM ai_providers WHERE id = ?1", [id])?;
    if changed == 0 {
        return Err(ProviderError::NotFound);
    }
    delete_password_if_present(&entry)?;
    if let Err(error) = transaction.commit() {
        restore_password(&entry, old_key.as_deref())?;
        return Err(ProviderError::Database(error));
    }
    Ok(())
}

/// A Rust-only accessor for a future provider client. Never expose this value
/// from a Tauri command or include it in logs, errors, history, or ActionPlans.
pub(crate) fn load_api_key_for_core(id: &str) -> ProviderResult<Option<String>> {
    validate_id(id)?;
    optional_password(&credential_entry(id)?)
}

struct ProviderRow {
    id: String,
    kind: String,
    name: String,
    base_url: String,
    model: String,
    allow_local_http: bool,
    created_at: String,
    updated_at: String,
}

impl ProviderRow {
    fn into_config(self) -> ProviderResult<ProviderConfig> {
        let has_api_key = optional_password(&credential_entry(&self.id)?)?.is_some();
        Ok(ProviderConfig {
            id: self.id,
            kind: ProviderKind::from_db(&self.kind)?,
            name: self.name,
            base_url: self.base_url,
            model: self.model,
            allow_local_http: self.allow_local_http,
            has_api_key,
            created_at: self.created_at,
            updated_at: self.updated_at,
        })
    }
}

enum KeyChange {
    None,
    Set(String),
    Clear,
}

impl KeyChange {
    fn is_change(&self) -> bool {
        !matches!(self, Self::None)
    }
}

fn find(conn: &Connection, id: &str) -> ProviderResult<Option<ProviderConfig>> {
    let row = conn
        .query_row(
            "SELECT id, kind, name, base_url, model, allow_local_http, created_at, updated_at
             FROM ai_providers WHERE id = ?1",
            [id],
            |row| {
                Ok(ProviderRow {
                    id: row.get(0)?,
                    kind: row.get(1)?,
                    name: row.get(2)?,
                    base_url: row.get(3)?,
                    model: row.get(4)?,
                    allow_local_http: row.get(5)?,
                    created_at: row.get(6)?,
                    updated_at: row.get(7)?,
                })
            },
        )
        .optional()?;
    row.map(ProviderRow::into_config).transpose()
}

fn required(value: String, field: &'static str) -> ProviderResult<String> {
    let value = value.trim();
    if value.is_empty() {
        Err(ProviderError::Required(field))
    } else {
        Ok(value.to_owned())
    }
}

fn validate_id(id: &str) -> ProviderResult<()> {
    Uuid::parse_str(id)
        .map(|_| ())
        .map_err(|_| ProviderError::InvalidId)
}

fn normalize_base_url(value: String, allow_local_http: bool) -> ProviderResult<String> {
    let value = value.trim();
    let parsed = Url::parse(value).map_err(|_| ProviderError::InvalidBaseUrl)?;
    if parsed.host_str().is_none()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.query().is_some()
        || parsed.fragment().is_some()
        || value
            .chars()
            .any(|c| c.is_whitespace() || c.is_control() || c == '\\')
    {
        return Err(ProviderError::InvalidBaseUrl);
    }
    let host = parsed.host_str().unwrap_or_default();
    let loopback = matches!(host, "localhost" | "127.0.0.1" | "[::1]");
    if parsed.scheme() != "https" && !(parsed.scheme() == "http" && loopback && allow_local_http) {
        return Err(ProviderError::InvalidBaseUrl);
    }
    Ok(parsed.as_str().trim_end_matches('/').to_owned())
}

fn credential_entry(id: &str) -> ProviderResult<Entry> {
    if !cfg!(windows) {
        return Err(ProviderError::UnsupportedPlatform);
    }
    Entry::new(CREDENTIAL_SERVICE, id).map_err(ProviderError::Credential)
}

fn optional_password(entry: &Entry) -> ProviderResult<Option<String>> {
    match entry.get_password() {
        Ok(secret) => Ok(Some(secret)),
        Err(KeyringError::NoEntry) => Ok(None),
        Err(error) => Err(ProviderError::Credential(error)),
    }
}

fn delete_password_if_present(entry: &Entry) -> ProviderResult<()> {
    match entry.delete_credential() {
        Ok(()) | Err(KeyringError::NoEntry) => Ok(()),
        Err(error) => Err(ProviderError::Credential(error)),
    }
}

fn restore_password(entry: &Entry, old_key: Option<&str>) -> ProviderResult<()> {
    let result = match old_key {
        Some(secret) => entry.set_password(secret),
        None => entry.delete_credential(),
    };
    match result {
        Ok(()) | Err(KeyringError::NoEntry) => Ok(()),
        Err(_) => Err(ProviderError::RecoveryFailed),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_provider_requires_https() {
        assert!(normalize_base_url("https://api.example.com/v1/".into(), false).is_ok());
        assert!(normalize_base_url("http://api.example.com/v1".into(), true).is_err());
        assert!(normalize_base_url("http://localhost:11434/v1".into(), false).is_err());
        assert!(normalize_base_url("http://localhost:11434/v1".into(), true).is_ok());
        assert!(normalize_base_url("https://secret@api.example.com/v1".into(), false).is_err());
        assert!(normalize_base_url("https://api.example.com/v1?key=x".into(), false).is_err());
    }

    #[cfg(windows)]
    #[test]
    fn provider_key_lives_in_windows_vault_not_sqlite() {
        let conn = Connection::open_in_memory().unwrap();
        init_schema(&conn).unwrap();
        let config = upsert(
            &conn,
            ProviderInput {
                id: None,
                kind: ProviderKind::Custom,
                name: "Test provider".into(),
                base_url: "https://example.com/v1".into(),
                model: "test-model".into(),
                allow_local_http: false,
                api_key: Some("dummy-test-key".into()),
                clear_api_key: false,
            },
        )
        .unwrap();
        assert!(config.has_api_key);
        assert_eq!(
            load_api_key_for_core(&config.id).unwrap().as_deref(),
            Some("dummy-test-key")
        );
        let database_value: String = conn
            .query_row(
                "SELECT name || base_url || model FROM ai_providers WHERE id=?1",
                [&config.id],
                |row| row.get(0),
            )
            .unwrap();
        assert!(!database_value.contains("dummy-test-key"));
        delete(&conn, &config.id).unwrap();
        assert!(load_api_key_for_core(&config.id).unwrap().is_none());
    }
}
