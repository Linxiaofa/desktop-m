//! The only component allowed to mutate managed Desktop files.
//! UI, rules, skills, and model output can submit plans, never filesystem operations.

use chrono::{DateTime, Utc};
use rusqlite::{params, params_from_iter, Connection, OptionalExtension};
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Error)]
pub enum CoreError {
    #[error("{0}")]
    Invalid(String),
    #[error("record not found")]
    NotFound,
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Database(#[from] rusqlite::Error),
}

pub type CoreResult<T> = Result<T, CoreError>;

#[derive(Clone, Debug, Serialize)]
pub struct ScanSummary {
    pub scanned: usize,
    pub updated: usize,
    pub removed: usize,
}

#[derive(Clone, Debug, Serialize)]
pub struct FileEntry {
    pub id: i64,
    pub path: String,
    pub name: String,
    pub kind: String,
    pub size: i64,
    pub modified_at: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct PlanItem {
    pub file_id: i64,
    pub source: String,
    pub destination: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct ActionPlan {
    pub id: String,
    pub items: Vec<PlanItem>,
    pub status: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct PreviewItem {
    pub source: String,
    pub destination: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct PlanPreview {
    pub plan_id: String,
    pub valid: bool,
    pub checks: Vec<String>,
    pub issues: Vec<String>,
    pub items: Vec<PreviewItem>,
}

#[derive(Clone, Debug, Serialize)]
pub struct TransactionResult {
    pub id: String,
    pub status: String,
    pub executed_count: usize,
}

#[derive(Clone, Debug, Serialize)]
pub struct HistoryEntry {
    pub id: String,
    pub created_at: String,
    pub status: String,
    pub summary: String,
    pub undo_available: bool,
}

#[derive(Clone)]
struct StoredItem {
    file_id: i64,
    source: PathBuf,
    destination: PathBuf,
    expected_size: i64,
    expected_modified_ns: i64,
}

/// Bounded history keeps the joined query and per-item undo checks cheap even
/// after months of use. Newer transactions always win.
const HISTORY_LIMIT: i64 = 200;

struct HistoryGroup {
    id: String,
    created_at: String,
    status: String,
    folder: String,
    items: Vec<StoredItem>,
}

pub struct DesktopCore {
    conn: Connection,
    desktop: PathBuf,
}

impl DesktopCore {
    pub fn open(desktop: &Path, database: &Path) -> CoreResult<Self> {
        let desktop = desktop.canonicalize()?;
        if !desktop.is_dir() {
            return Err(CoreError::Invalid(
                "Desktop directory is unavailable".into(),
            ));
        }
        if let Some(parent) = database.parent() {
            fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(database)?;
        conn.execute_batch(
            "PRAGMA journal_mode=WAL;
             PRAGMA synchronous=FULL;
             PRAGMA foreign_keys=ON;
             CREATE TABLE IF NOT EXISTS files (
               id INTEGER PRIMARY KEY,
               path TEXT NOT NULL UNIQUE,
               name TEXT NOT NULL,
               kind TEXT NOT NULL CHECK (kind IN ('file','directory')),
               size INTEGER NOT NULL,
               modified_at TEXT NOT NULL,
               modified_ns INTEGER NOT NULL,
               scan_id TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS action_plans (
               id TEXT PRIMARY KEY,
               created_at TEXT NOT NULL,
               status TEXT NOT NULL,
               destination_folder TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS plan_items (
               plan_id TEXT NOT NULL REFERENCES action_plans(id),
               ordinal INTEGER NOT NULL,
               file_id INTEGER NOT NULL,
               source TEXT NOT NULL,
               destination TEXT NOT NULL,
               expected_size INTEGER NOT NULL,
               expected_modified_ns INTEGER NOT NULL,
               PRIMARY KEY (plan_id, ordinal)
             );
             CREATE TABLE IF NOT EXISTS transactions (
               id TEXT PRIMARY KEY,
               plan_id TEXT NOT NULL REFERENCES action_plans(id),
               created_at TEXT NOT NULL,
               status TEXT NOT NULL,
               destination_folder TEXT NOT NULL,
               created_dir INTEGER NOT NULL DEFAULT 0
             );
             CREATE TABLE IF NOT EXISTS transaction_items (
               transaction_id TEXT NOT NULL REFERENCES transactions(id),
               ordinal INTEGER NOT NULL,
               source TEXT NOT NULL,
               destination TEXT NOT NULL,
               expected_size INTEGER NOT NULL,
               expected_modified_ns INTEGER NOT NULL,
               state TEXT NOT NULL,
               PRIMARY KEY (transaction_id, ordinal)
             );
             CREATE INDEX IF NOT EXISTS idx_transactions_created ON transactions(created_at DESC);",
        )?;
        let mut core = Self { conn, desktop };
        core.recover_interrupted()?;
        Ok(core)
    }

    pub fn connection(&self) -> &Connection {
        &self.conn
    }

    pub fn scan_desktop(&mut self) -> CoreResult<ScanSummary> {
        let scan_id = Uuid::new_v4().to_string();
        let mut entries = Vec::new();
        for entry in fs::read_dir(&self.desktop)? {
            let entry = entry?;
            let path = entry.path();
            let metadata = match fs::symlink_metadata(&path) {
                Ok(metadata) => metadata,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(error) => return Err(error.into()),
            };
            if is_link_or_reparse(&metadata) {
                continue;
            }
            let kind = if metadata.is_file() {
                "file"
            } else if metadata.is_dir() {
                "directory"
            } else {
                continue;
            };
            let name = entry.file_name().to_string_lossy().into_owned();
            let modified = metadata.modified()?;
            entries.push((
                path.to_string_lossy().into_owned(),
                name,
                kind,
                i64::try_from(metadata.len()).unwrap_or(i64::MAX),
                DateTime::<Utc>::from(modified).to_rfc3339(),
                modified_ns(modified),
            ));
        }
        let transaction = self.conn.transaction()?;
        // Prefetch the whole index once so the diff is a single query instead
        // of one SELECT per Desktop entry (N+1 -> 1).
        let mut previous_index: HashMap<String, (String, i64, i64)> = HashMap::new();
        {
            let mut statement =
                transaction.prepare("SELECT path, kind, size, modified_ns FROM files")?;
            let rows = statement.query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    (
                        row.get::<_, String>(1)?,
                        row.get::<_, i64>(2)?,
                        row.get::<_, i64>(3)?,
                    ),
                ))
            })?;
            for row in rows {
                let (path, snapshot) = row?;
                previous_index.insert(path, snapshot);
            }
        }
        let mut updated = 0;
        for (path, name, kind, size, modified_at, modified_ns) in &entries {
            let changed = previous_index
                .get(path)
                .is_none_or(|snapshot| snapshot != &(kind.to_string(), *size, *modified_ns));
            if changed {
                updated += 1;
            }
            transaction.execute(
                "INSERT INTO files(path,name,kind,size,modified_at,modified_ns,scan_id)
                 VALUES(?1,?2,?3,?4,?5,?6,?7)
                 ON CONFLICT(path) DO UPDATE SET
                   name=excluded.name,kind=excluded.kind,size=excluded.size,
                   modified_at=excluded.modified_at,modified_ns=excluded.modified_ns,
                   scan_id=excluded.scan_id",
                params![path, name, kind, size, modified_at, modified_ns, scan_id],
            )?;
        }
        let removed = transaction.execute("DELETE FROM files WHERE scan_id<>?1", [&scan_id])?;
        transaction.commit()?;
        Ok(ScanSummary {
            scanned: entries.len(),
            updated,
            removed,
        })
    }

    pub fn list_files(&self) -> CoreResult<Vec<FileEntry>> {
        let mut statement = self.conn.prepare(
            "SELECT id,path,name,kind,size,modified_at FROM files
             ORDER BY kind DESC, name COLLATE NOCASE",
        )?;
        let rows = statement.query_map([], |row| {
            Ok(FileEntry {
                id: row.get(0)?,
                path: row.get(1)?,
                name: row.get(2)?,
                kind: row.get(3)?,
                size: row.get(4)?,
                modified_at: row.get(5)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    pub fn create_move_plan(
        &mut self,
        file_ids: Vec<i64>,
        destination: String,
    ) -> CoreResult<ActionPlan> {
        let folder = validate_folder(&destination)?;
        if file_ids.is_empty() || file_ids.len() > 100 {
            return Err(CoreError::Invalid("Select 1–100 files".into()));
        }
        let unique: HashSet<_> = file_ids.iter().copied().collect();
        if unique.len() != file_ids.len() {
            return Err(CoreError::Invalid("Duplicate file selection".into()));
        }
        let destination_dir = self.desktop.join(&folder);
        self.check_destination_dir(&destination_dir)?;
        let mut items = Vec::with_capacity(file_ids.len());
        for id in &file_ids {
            let row: Option<(String, String, String, i64, i64)> = self
                .conn
                .query_row(
                    "SELECT path,name,kind,size,modified_ns FROM files WHERE id=?1",
                    [id],
                    |row| {
                        Ok((
                            row.get(0)?,
                            row.get(1)?,
                            row.get(2)?,
                            row.get(3)?,
                            row.get(4)?,
                        ))
                    },
                )
                .optional()?;
            let (source, name, kind, size, modified) = row.ok_or(CoreError::NotFound)?;
            if kind != "file" {
                return Err(CoreError::Invalid(format!("{name} is not a regular file")));
            }
            let source = PathBuf::from(source);
            if source.parent() != Some(self.desktop.as_path()) {
                return Err(CoreError::Invalid(
                    "Source is outside Desktop top level".into(),
                ));
            }
            items.push(StoredItem {
                file_id: *id,
                source,
                destination: destination_dir.join(name),
                expected_size: size,
                expected_modified_ns: modified,
            });
        }
        let plan_id = Uuid::new_v4().to_string();
        let transaction = self.conn.transaction()?;
        transaction.execute(
            "INSERT INTO action_plans(id,created_at,status,destination_folder) VALUES(?1,?2,'draft',?3)",
            params![plan_id, Utc::now().to_rfc3339(), folder],
        )?;
        for (ordinal, item) in items.iter().enumerate() {
            transaction.execute(
                "INSERT INTO plan_items(plan_id,ordinal,file_id,source,destination,expected_size,expected_modified_ns)
                 VALUES(?1,?2,?3,?4,?5,?6,?7)",
                params![
                    plan_id,
                    ordinal as i64,
                    item.file_id,
                    item.source.to_string_lossy(),
                    item.destination.to_string_lossy(),
                    item.expected_size,
                    item.expected_modified_ns
                ],
            )?;
        }
        transaction.commit()?;
        Ok(ActionPlan {
            id: plan_id,
            items: items
                .into_iter()
                .map(|item| PlanItem {
                    file_id: item.file_id,
                    source: item.source.to_string_lossy().into_owned(),
                    destination: item.destination.to_string_lossy().into_owned(),
                })
                .collect(),
            status: "draft".into(),
        })
    }

    pub fn validate_plan(&self, plan_id: &str) -> CoreResult<PlanPreview> {
        let (status, folder) = self.load_plan(plan_id)?;
        let items = self.load_plan_items(plan_id)?;
        let destination_dir = self.desktop.join(&folder);
        let mut issues = Vec::new();
        let mut checks = vec![
            "来源仅限 Desktop 顶层普通文件".into(),
            "目标仅限 Desktop 下单层目录".into(),
            "目标已存在时拒绝覆盖".into(),
            "执行前会再次验证文件状态".into(),
        ];
        if status != "draft" {
            issues.push("计划已执行或失效".into());
        }
        if let Err(error) = self.check_destination_dir(&destination_dir) {
            issues.push(error.to_string());
        }
        if items.is_empty() {
            issues.push("计划没有文件".into());
        }
        let mut targets = HashSet::new();
        for item in &items {
            if item.source.parent() != Some(self.desktop.as_path())
                || item.destination.parent() != Some(destination_dir.as_path())
                || item.source.file_name() != item.destination.file_name()
            {
                issues.push(format!("路径越界：{}", item.source.display()));
                continue;
            }
            if !targets.insert(item.destination.clone()) {
                issues.push(format!("计划内目标重复：{}", item.destination.display()));
            }
            if let Err(error) =
                check_regular_file(&item.source, item.expected_size, item.expected_modified_ns)
            {
                issues.push(format!("来源已变化：{} ({error})", item.source.display()));
            }
            if path_exists(&item.destination)? {
                issues.push(format!("目标已存在：{}", item.destination.display()));
            }
        }
        if issues.is_empty() {
            checks.push(format!("{} 个文件已通过验证", items.len()));
        }
        Ok(PlanPreview {
            plan_id: plan_id.into(),
            valid: issues.is_empty(),
            checks,
            issues,
            items: items
                .into_iter()
                .map(|item| PreviewItem {
                    source: item.source.to_string_lossy().into_owned(),
                    destination: item.destination.to_string_lossy().into_owned(),
                })
                .collect(),
        })
    }

    pub fn execute_plan(&mut self, plan_id: &str) -> CoreResult<TransactionResult> {
        let preview = self.validate_plan(plan_id)?;
        if !preview.valid {
            return Err(CoreError::Invalid(preview.issues.join("; ")));
        }
        let (_, folder) = self.load_plan(plan_id)?;
        let items = self.load_plan_items(plan_id)?;
        let destination_dir = self.desktop.join(&folder);
        let transaction_id = Uuid::new_v4().to_string();
        let journal = self.conn.transaction()?;
        journal.execute(
            "INSERT INTO transactions(id,plan_id,created_at,status,destination_folder)
             VALUES(?1,?2,?3,'executing',?4)",
            params![transaction_id, plan_id, Utc::now().to_rfc3339(), folder],
        )?;
        for (ordinal, item) in items.iter().enumerate() {
            journal.execute(
                "INSERT INTO transaction_items
                 (transaction_id,ordinal,source,destination,expected_size,expected_modified_ns,state)
                 VALUES(?1,?2,?3,?4,?5,?6,'pending')",
                params![
                    transaction_id,
                    ordinal as i64,
                    item.source.to_string_lossy(),
                    item.destination.to_string_lossy(),
                    item.expected_size,
                    item.expected_modified_ns
                ],
            )?;
        }
        journal.commit()?;

        let created_dir = if path_exists(&destination_dir)? {
            false
        } else {
            if let Err(error) = fs::create_dir(&destination_dir) {
                self.set_transaction_status(&transaction_id, "failed")?;
                self.conn.execute(
                    "UPDATE action_plans SET status='failed' WHERE id=?1",
                    [plan_id],
                )?;
                return Err(error.into());
            }
            self.conn.execute(
                "UPDATE transactions SET created_dir=1 WHERE id=?1",
                [&transaction_id],
            )?;
            true
        };
        let mut moved = Vec::new();
        let attempt = (|| -> CoreResult<()> {
            self.check_destination_dir(&destination_dir)?;
            for (ordinal, item) in items.iter().enumerate() {
                self.check_destination_dir(&destination_dir)?;
                check_regular_file(&item.source, item.expected_size, item.expected_modified_ns)?;
                if path_exists(&item.destination)? {
                    return Err(CoreError::Invalid(format!(
                        "Target appeared: {}",
                        item.destination.display()
                    )));
                }
                move_noreplace(&item.source, &item.destination)?;
                moved.push(ordinal);
                self.conn.execute(
                    "UPDATE transaction_items SET state='moved' WHERE transaction_id=?1 AND ordinal=?2",
                    params![transaction_id, ordinal as i64],
                )?;
            }
            self.set_transaction_status(&transaction_id, "executed")?;
            self.conn.execute(
                "UPDATE action_plans SET status='executed' WHERE id=?1",
                [plan_id],
            )?;
            Ok(())
        })();
        if let Err(error) = attempt {
            let mut rollback_ok = true;
            for ordinal in moved.iter().rev() {
                let item = &items[*ordinal];
                if move_noreplace(&item.destination, &item.source).is_err() {
                    rollback_ok = false;
                } else {
                    let _ = self.conn.execute(
                        "UPDATE transaction_items SET state='rolled_back'
                         WHERE transaction_id=?1 AND ordinal=?2",
                        params![transaction_id, *ordinal as i64],
                    );
                }
            }
            let status = if rollback_ok {
                "failed"
            } else {
                "recovery_needed"
            };
            self.set_transaction_status(&transaction_id, status)?;
            self.conn.execute(
                "UPDATE action_plans SET status='failed' WHERE id=?1",
                [plan_id],
            )?;
            if created_dir && rollback_ok {
                let _ = fs::remove_dir(&destination_dir);
            }
            let _ = self.scan_desktop();
            return Err(CoreError::Invalid(format!(
                "Move failed; transaction {} is {}: {}",
                transaction_id, status, error
            )));
        }
        let _ = self.scan_desktop();
        Ok(TransactionResult {
            id: transaction_id,
            status: "executed".into(),
            executed_count: items.len(),
        })
    }

    pub fn list_history(&self) -> CoreResult<Vec<HistoryEntry>> {
        // One joined query loads the newest transactions and their items
        // together, replacing the previous one-query-per-transaction pattern.
        let mut statement = self.conn.prepare(
            "SELECT t.id,t.created_at,t.status,t.destination_folder,
                    ti.source,ti.destination,ti.expected_size,ti.expected_modified_ns
             FROM (SELECT id,created_at,status,destination_folder FROM transactions
                   ORDER BY created_at DESC LIMIT ?1) AS t
             LEFT JOIN transaction_items AS ti ON ti.transaction_id = t.id
             ORDER BY t.created_at DESC, t.id, ti.ordinal",
        )?;
        let rows = statement.query_map([HISTORY_LIMIT], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, Option<String>>(4)?,
                row.get::<_, Option<String>>(5)?,
                row.get::<_, Option<i64>>(6)?,
                row.get::<_, Option<i64>>(7)?,
            ))
        })?;
        let mut grouped: Vec<HistoryGroup> = Vec::new();
        for row in rows {
            let (id, created_at, status, folder, source, destination, size, modified_ns) = row?;
            let item = match (source, destination, size, modified_ns) {
                (
                    Some(source),
                    Some(destination),
                    Some(expected_size),
                    Some(expected_modified_ns),
                ) => StoredItem {
                    file_id: 0,
                    source: PathBuf::from(source),
                    destination: PathBuf::from(destination),
                    expected_size,
                    expected_modified_ns,
                },
                _ => {
                    // A transaction without journaled items still lists.
                    grouped.push(HistoryGroup {
                        id,
                        created_at,
                        status,
                        folder,
                        items: Vec::new(),
                    });
                    continue;
                }
            };
            match grouped.last_mut() {
                Some(group) if group.id == id => group.items.push(item),
                _ => grouped.push(HistoryGroup {
                    id,
                    created_at,
                    status,
                    folder,
                    items: vec![item],
                }),
            }
        }
        let mut history = Vec::with_capacity(grouped.len());
        for group in grouped {
            // Only executed transactions can be undone; other states skip the
            // filesystem checks entirely.
            let undo_available =
                group.status == "executed" && self.transaction_items_undoable(&group.items)?;
            history.push(HistoryEntry {
                id: group.id,
                created_at: group.created_at,
                status: group.status,
                summary: format!("{} 个文件 → Desktop\\{}", group.items.len(), group.folder),
                undo_available,
            });
        }
        Ok(history)
    }

    pub fn undo_transaction(&mut self, transaction_id: &str) -> CoreResult<TransactionResult> {
        let (status, folder, created_dir): (String, String, bool) = self
            .conn
            .query_row(
                "SELECT status,destination_folder,created_dir FROM transactions WHERE id=?1",
                [transaction_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?
            .ok_or(CoreError::NotFound)?;
        if status != "executed" {
            return Err(CoreError::Invalid("Transaction cannot be undone".into()));
        }
        if !self.can_undo(transaction_id)? {
            return Err(CoreError::Invalid(
                "Undo blocked: a source or destination changed".into(),
            ));
        }
        let items = self.load_transaction_items(transaction_id)?;
        self.set_transaction_status(transaction_id, "undoing")?;
        let mut restored = Vec::new();
        let attempt = (|| -> CoreResult<()> {
            for ordinal in (0..items.len()).rev() {
                let item = &items[ordinal];
                self.check_destination_dir(&self.desktop.join(&folder))?;
                check_regular_file(
                    &item.destination,
                    item.expected_size,
                    item.expected_modified_ns,
                )?;
                if path_exists(&item.source)? {
                    return Err(CoreError::Invalid("Original path appeared".into()));
                }
                move_noreplace(&item.destination, &item.source)?;
                restored.push(ordinal);
                self.conn.execute(
                    "UPDATE transaction_items SET state='undone'
                     WHERE transaction_id=?1 AND ordinal=?2",
                    params![transaction_id, ordinal as i64],
                )?;
            }
            self.set_transaction_status(transaction_id, "undone")?;
            Ok(())
        })();
        if let Err(error) = attempt {
            let mut compensation_ok = true;
            for ordinal in restored.iter().rev() {
                let item = &items[*ordinal];
                if move_noreplace(&item.source, &item.destination).is_err() {
                    compensation_ok = false;
                } else {
                    let _ = self.conn.execute(
                        "UPDATE transaction_items SET state='moved'
                         WHERE transaction_id=?1 AND ordinal=?2",
                        params![transaction_id, *ordinal as i64],
                    );
                }
            }
            self.set_transaction_status(
                transaction_id,
                if compensation_ok {
                    "executed"
                } else {
                    "recovery_needed"
                },
            )?;
            let _ = self.scan_desktop();
            return Err(CoreError::Invalid(format!("Undo failed: {error}")));
        }
        if created_dir {
            let _ = fs::remove_dir(self.desktop.join(folder));
        }
        let _ = self.scan_desktop();
        Ok(TransactionResult {
            id: transaction_id.into(),
            status: "undone".into(),
            executed_count: items.len(),
        })
    }

    fn can_undo(&self, transaction_id: &str) -> CoreResult<bool> {
        let items = self.load_transaction_items(transaction_id)?;
        self.transaction_items_undoable(&items)
    }

    /// Shared undo eligibility check so `list_history` can reuse it with items
    /// it already loaded instead of querying the database again.
    fn transaction_items_undoable(&self, items: &[StoredItem]) -> CoreResult<bool> {
        if items.is_empty() {
            return Ok(false);
        }
        for item in items {
            if item.source.parent() != Some(self.desktop.as_path())
                || item.destination.parent().and_then(Path::parent) != Some(self.desktop.as_path())
                || item
                    .destination
                    .parent()
                    .is_none_or(|folder| self.check_destination_dir(folder).is_err())
                || check_regular_file(
                    &item.destination,
                    item.expected_size,
                    item.expected_modified_ns,
                )
                .is_err()
                || path_exists(&item.source)?
            {
                return Ok(false);
            }
        }
        Ok(true)
    }

    fn check_destination_dir(&self, destination_dir: &Path) -> CoreResult<()> {
        if destination_dir.parent() != Some(self.desktop.as_path()) {
            return Err(CoreError::Invalid("Destination outside Desktop".into()));
        }
        match fs::symlink_metadata(destination_dir) {
            Ok(metadata) => {
                if is_link_or_reparse(&metadata) || !metadata.is_dir() {
                    return Err(CoreError::Invalid(
                        "Destination is not a normal directory".into(),
                    ));
                }
                if destination_dir.canonicalize()?.parent() != Some(self.desktop.as_path()) {
                    return Err(CoreError::Invalid(
                        "Destination redirects outside Desktop".into(),
                    ));
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        Ok(())
    }

    fn load_plan(&self, plan_id: &str) -> CoreResult<(String, String)> {
        self.conn
            .query_row(
                "SELECT status,destination_folder FROM action_plans WHERE id=?1",
                [plan_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?
            .ok_or(CoreError::NotFound)
    }

    fn load_plan_items(&self, plan_id: &str) -> CoreResult<Vec<StoredItem>> {
        self.load_items(
            "SELECT file_id,source,destination,expected_size,expected_modified_ns
             FROM plan_items WHERE plan_id=?1 ORDER BY ordinal",
            plan_id,
        )
    }

    fn load_transaction_items(&self, transaction_id: &str) -> CoreResult<Vec<StoredItem>> {
        self.load_items(
            "SELECT 0,source,destination,expected_size,expected_modified_ns
             FROM transaction_items WHERE transaction_id=?1 ORDER BY ordinal",
            transaction_id,
        )
    }

    fn load_items(&self, query: &str, id: &str) -> CoreResult<Vec<StoredItem>> {
        let mut statement = self.conn.prepare(query)?;
        let rows = statement.query_map([id], |row| {
            Ok(StoredItem {
                file_id: row.get(0)?,
                source: PathBuf::from(row.get::<_, String>(1)?),
                destination: PathBuf::from(row.get::<_, String>(2)?),
                expected_size: row.get(3)?,
                expected_modified_ns: row.get(4)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    fn set_transaction_status(&self, transaction_id: &str, status: &str) -> CoreResult<()> {
        self.conn.execute(
            "UPDATE transactions SET status=?2 WHERE id=?1",
            params![transaction_id, status],
        )?;
        Ok(())
    }

    fn recover_interrupted(&mut self) -> CoreResult<()> {
        // SQLite and NTFS cannot commit atomically. Reconcile each journaled
        // operation conservatively: only restore a file when its counterpart
        // is absent and its snapshot still matches. Never overwrite.
        let interrupted = {
            let mut statement = self.conn.prepare(
                "SELECT id,plan_id,status,destination_folder,created_dir
                 FROM transactions WHERE status IN ('executing','undoing')
                 ORDER BY created_at",
            )?;
            let rows = statement.query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, bool>(4)?,
                ))
            })?;
            rows.collect::<Result<Vec<_>, _>>()?
        };
        for (transaction_id, plan_id, old_status, folder, created_dir) in interrupted {
            let items = self.load_transaction_items(&transaction_id)?;
            let destination_dir = self.desktop.join(&folder);
            let mut safe = self.check_destination_dir(&destination_dir).is_ok();
            if safe {
                for (ordinal, item) in items.iter().enumerate().rev() {
                    if item.source.parent() != Some(self.desktop.as_path())
                        || item.destination.parent() != Some(destination_dir.as_path())
                    {
                        safe = false;
                        break;
                    }
                    if self.check_destination_dir(&destination_dir).is_err() {
                        safe = false;
                        break;
                    }
                    let source_exists = path_exists(&item.source);
                    let destination_exists = path_exists(&item.destination);
                    match (source_exists, destination_exists) {
                        (Ok(true), Ok(false)) => {}
                        (Ok(false), Ok(true))
                            if check_regular_file(
                                &item.destination,
                                item.expected_size,
                                item.expected_modified_ns,
                            )
                            .is_ok() =>
                        {
                            if move_noreplace(&item.destination, &item.source).is_err() {
                                safe = false;
                                break;
                            }
                            let _ = self.conn.execute(
                                "UPDATE transaction_items SET state='rolled_back'
                                 WHERE transaction_id=?1 AND ordinal=?2",
                                params![transaction_id, ordinal as i64],
                            );
                        }
                        _ => {
                            safe = false;
                            break;
                        }
                    }
                }
            }
            if safe {
                let status = if old_status == "undoing" {
                    "undone"
                } else {
                    "failed"
                };
                self.set_transaction_status(&transaction_id, status)?;
                if old_status == "executing" {
                    self.conn.execute(
                        "UPDATE action_plans SET status='failed' WHERE id=?1",
                        [&plan_id],
                    )?;
                }
                if created_dir {
                    let _ = fs::remove_dir(&destination_dir);
                }
            } else {
                self.set_transaction_status(&transaction_id, "recovery_needed")?;
            }
        }
        Ok(())
    }
}

/// Loads `(name, kind)` for many file IDs in one query. Used by rules and AI
/// previews so a 100-file selection costs one statement instead of 100.
pub(crate) fn load_file_summaries(
    conn: &Connection,
    ids: &[i64],
) -> CoreResult<HashMap<i64, (String, String)>> {
    if ids.is_empty() {
        return Ok(HashMap::new());
    }
    let placeholders = std::iter::repeat_n("?", ids.len())
        .collect::<Vec<_>>()
        .join(",");
    let sql = format!("SELECT id,name,kind FROM files WHERE id IN ({placeholders})");
    let mut statement = conn.prepare(&sql)?;
    let rows = statement.query_map(params_from_iter(ids.iter()), |row| {
        Ok((
            row.get::<_, i64>(0)?,
            (row.get::<_, String>(1)?, row.get::<_, String>(2)?),
        ))
    })?;
    let mut summaries = HashMap::with_capacity(ids.len());
    for row in rows {
        let (id, value) = row?;
        summaries.insert(id, value);
    }
    Ok(summaries)
}

pub(crate) fn validate_folder(value: &str) -> CoreResult<String> {
    let folder = value.trim();
    let upper = folder.trim_end_matches(['.', ' ']).to_ascii_uppercase();
    let base = upper.split('.').next().unwrap_or_default();
    const RESERVED: &[&str] = &[
        "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
        "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
    ];
    if folder.is_empty()
        || folder == "."
        || folder == ".."
        || folder.ends_with(['.', ' '])
        || folder
            .chars()
            .any(|c| c.is_control() || r#"<>:"/\|?*"#.contains(c))
        || RESERVED.contains(&base)
    {
        return Err(CoreError::Invalid(
            "Use one ordinary folder name directly under Desktop".into(),
        ));
    }
    Ok(folder.into())
}

fn modified_ns(time: std::time::SystemTime) -> i64 {
    time.duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|value| i64::try_from(value.as_nanos()).ok())
        .unwrap_or(0)
}

fn check_regular_file(
    path: &Path,
    expected_size: i64,
    expected_modified_ns: i64,
) -> CoreResult<()> {
    let metadata = fs::symlink_metadata(path)?;
    if is_link_or_reparse(&metadata)
        || !metadata.is_file()
        || i64::try_from(metadata.len()).unwrap_or(i64::MAX) != expected_size
        || modified_ns(metadata.modified()?) != expected_modified_ns
    {
        return Err(CoreError::Invalid(
            "File type, size, or modification time changed".into(),
        ));
    }
    Ok(())
}

fn is_link_or_reparse(metadata: &fs::Metadata) -> bool {
    if metadata.file_type().is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
        if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return true;
        }
    }
    false
}

fn path_exists(path: &Path) -> CoreResult<bool> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error.into()),
    }
}

#[cfg(windows)]
fn move_noreplace(source: &Path, destination: &Path) -> CoreResult<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::MoveFileW;
    let source: Vec<u16> = source.as_os_str().encode_wide().chain(Some(0)).collect();
    let destination: Vec<u16> = destination
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect();
    let result = unsafe { MoveFileW(source.as_ptr(), destination.as_ptr()) };
    if result == 0 {
        Err(std::io::Error::last_os_error().into())
    } else {
        Ok(())
    }
}

#[cfg(not(windows))]
fn move_noreplace(source: &Path, destination: &Path) -> CoreResult<()> {
    if path_exists(destination)? {
        return Err(CoreError::Invalid("Target exists".into()));
    }
    fs::rename(source, destination)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn move_preview_execute_history_and_undo() {
        let temp = tempfile::tempdir().unwrap();
        let desktop = temp.path().join("Desktop");
        fs::create_dir(&desktop).unwrap();
        fs::write(desktop.join("alpha.txt"), b"alpha").unwrap();
        let mut core = DesktopCore::open(&desktop, &temp.path().join("app/index.sqlite")).unwrap();
        assert_eq!(core.scan_desktop().unwrap().scanned, 1);
        let id = core.list_files().unwrap()[0].id;
        let plan = core.create_move_plan(vec![id], "Sorted".into()).unwrap();
        assert!(core.validate_plan(&plan.id).unwrap().valid);
        let transaction = core.execute_plan(&plan.id).unwrap();
        assert_eq!(transaction.status, "executed");
        assert!(desktop.join("Sorted/alpha.txt").exists());
        assert!(!desktop.join("alpha.txt").exists());
        assert!(core.list_history().unwrap()[0].undo_available);
        assert_eq!(
            core.undo_transaction(&transaction.id).unwrap().status,
            "undone"
        );
        assert!(desktop.join("alpha.txt").exists());
        assert!(!desktop.join("Sorted").exists());
    }

    #[test]
    fn stale_source_and_conflict_block_execution() {
        let temp = tempfile::tempdir().unwrap();
        let desktop = temp.path().join("Desktop");
        fs::create_dir(&desktop).unwrap();
        fs::write(desktop.join("alpha.txt"), b"alpha").unwrap();
        let mut core = DesktopCore::open(&desktop, &temp.path().join("index.sqlite")).unwrap();
        core.scan_desktop().unwrap();
        let id = core.list_files().unwrap()[0].id;
        let plan = core.create_move_plan(vec![id], "Sorted".into()).unwrap();
        fs::write(desktop.join("alpha.txt"), b"changed content").unwrap();
        assert!(!core.validate_plan(&plan.id).unwrap().valid);
        assert!(core.execute_plan(&plan.id).is_err());
        core.scan_desktop().unwrap();
        let plan = core.create_move_plan(vec![id], "Sorted".into()).unwrap();
        fs::create_dir(desktop.join("Sorted")).unwrap();
        fs::write(desktop.join("Sorted/alpha.txt"), b"existing").unwrap();
        assert!(!core.validate_plan(&plan.id).unwrap().valid);
        assert!(core.execute_plan(&plan.id).is_err());
        assert_eq!(
            fs::read(desktop.join("Sorted/alpha.txt")).unwrap(),
            b"existing"
        );
    }

    #[test]
    fn traversal_and_reserved_names_are_rejected() {
        for value in ["../outside", "a/b", r"a\b", "CON", "NUL.txt", "bad.", ""] {
            assert!(validate_folder(value).is_err(), "{value}");
        }
        assert!(validate_folder("Documents").is_ok());
    }

    #[test]
    fn interrupted_execution_rolls_back_on_restart() {
        let temp = tempfile::tempdir().unwrap();
        let desktop = temp.path().join("Desktop");
        fs::create_dir(&desktop).unwrap();
        fs::write(desktop.join("alpha.txt"), b"alpha").unwrap();
        let db = temp.path().join("index.sqlite");
        let mut core = DesktopCore::open(&desktop, &db).unwrap();
        core.scan_desktop().unwrap();
        let id = core.list_files().unwrap()[0].id;
        let plan = core.create_move_plan(vec![id], "Sorted".into()).unwrap();
        let item = core.load_plan_items(&plan.id).unwrap().remove(0);
        let transaction_id = Uuid::new_v4().to_string();
        core.conn.execute(
            "INSERT INTO transactions(id,plan_id,created_at,status,destination_folder,created_dir)
             VALUES(?1,?2,?3,'executing','Sorted',1)",
            params![transaction_id, plan.id, Utc::now().to_rfc3339()],
        ).unwrap();
        core.conn
            .execute(
                "INSERT INTO transaction_items
             (transaction_id,ordinal,source,destination,expected_size,expected_modified_ns,state)
             VALUES(?1,0,?2,?3,?4,?5,'pending')",
                params![
                    transaction_id,
                    item.source.to_string_lossy(),
                    item.destination.to_string_lossy(),
                    item.expected_size,
                    item.expected_modified_ns
                ],
            )
            .unwrap();
        fs::create_dir(desktop.join("Sorted")).unwrap();
        move_noreplace(&item.source, &item.destination).unwrap();
        drop(core);
        let recovered = DesktopCore::open(&desktop, &db).unwrap();
        assert!(desktop.join("alpha.txt").exists());
        assert!(!desktop.join("Sorted").exists());
        assert_eq!(recovered.list_history().unwrap()[0].status, "failed");
    }

    #[test]
    fn interrupted_undo_finishes_on_restart() {
        let temp = tempfile::tempdir().unwrap();
        let desktop = temp.path().join("Desktop");
        fs::create_dir(&desktop).unwrap();
        fs::write(desktop.join("alpha.txt"), b"alpha").unwrap();
        let db = temp.path().join("index.sqlite");
        let mut core = DesktopCore::open(&desktop, &db).unwrap();
        core.scan_desktop().unwrap();
        let id = core.list_files().unwrap()[0].id;
        let plan = core.create_move_plan(vec![id], "Sorted".into()).unwrap();
        let transaction = core.execute_plan(&plan.id).unwrap();
        core.conn
            .execute(
                "UPDATE transactions SET status='undoing' WHERE id=?1",
                [&transaction.id],
            )
            .unwrap();
        move_noreplace(
            &desktop.join("Sorted/alpha.txt"),
            &desktop.join("alpha.txt"),
        )
        .unwrap();
        drop(core);
        let recovered = DesktopCore::open(&desktop, &db).unwrap();
        assert!(desktop.join("alpha.txt").exists());
        assert_eq!(recovered.list_history().unwrap()[0].status, "undone");
    }
}
