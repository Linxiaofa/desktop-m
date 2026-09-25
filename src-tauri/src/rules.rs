//! Deterministic suggestions. Rules can only propose a destination folder.

use crate::core::{
    load_file_summaries, validate_folder, ActionPlan, CoreError, CoreResult, DesktopCore,
    PlanPreview,
};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};
use std::path::Path;
use uuid::Uuid;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MatchType {
    Extension,
    Name,
}

impl MatchType {
    fn as_db(&self) -> &'static str {
        match self {
            Self::Extension => "extension",
            Self::Name => "name",
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuleInput {
    pub id: Option<String>,
    pub match_type: MatchType,
    pub pattern: String,
    pub destination: String,
    pub enabled: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Rule {
    pub id: String,
    pub match_type: MatchType,
    pub pattern: String,
    pub destination: String,
    pub enabled: bool,
    pub priority: i64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuleSuggestion {
    pub file_id: i64,
    pub file_name: String,
    pub destination: Option<String>,
    pub rule_id: Option<String>,
}

pub fn init_schema(conn: &Connection) -> CoreResult<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS rules (
           id TEXT PRIMARY KEY,
           match_type TEXT NOT NULL CHECK (match_type IN ('extension','name')),
           pattern TEXT NOT NULL,
           destination TEXT NOT NULL,
           enabled INTEGER NOT NULL CHECK (enabled IN (0,1)),
           priority INTEGER NOT NULL UNIQUE
         );",
    )?;
    Ok(())
}

pub fn list(conn: &Connection) -> CoreResult<Vec<Rule>> {
    let mut statement = conn.prepare(
        "SELECT id,match_type,pattern,destination,enabled,priority
         FROM rules ORDER BY priority",
    )?;
    let rows = statement.query_map([], |row| {
        let match_type: String = row.get(1)?;
        Ok(Rule {
            id: row.get(0)?,
            match_type: if match_type == "extension" {
                MatchType::Extension
            } else {
                MatchType::Name
            },
            pattern: row.get(2)?,
            destination: row.get(3)?,
            enabled: row.get(4)?,
            priority: row.get(5)?,
        })
    })?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}

pub fn upsert(conn: &Connection, input: RuleInput) -> CoreResult<Rule> {
    let destination = validate_folder(&input.destination)?;
    let pattern = normalize_pattern(&input.match_type, &input.pattern)?;
    let id = if let Some(id) = input.id {
        Uuid::parse_str(&id).map_err(|_| CoreError::Invalid("Invalid rule ID".into()))?;
        let exists: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM rules WHERE id=?1)",
            [&id],
            |row| row.get(0),
        )?;
        if !exists {
            return Err(CoreError::NotFound);
        }
        id
    } else {
        Uuid::new_v4().to_string()
    };
    let priority: i64 = conn
        .query_row("SELECT priority FROM rules WHERE id=?1", [&id], |row| {
            row.get(0)
        })
        .optional()?
        .unwrap_or(
            conn.query_row("SELECT COALESCE(MAX(priority),0)+1 FROM rules", [], |row| {
                row.get(0)
            })?,
        );
    conn.execute(
        "INSERT INTO rules(id,match_type,pattern,destination,enabled,priority)
         VALUES(?1,?2,?3,?4,?5,?6)
         ON CONFLICT(id) DO UPDATE SET
           match_type=excluded.match_type,pattern=excluded.pattern,
           destination=excluded.destination,enabled=excluded.enabled",
        params![
            id,
            input.match_type.as_db(),
            pattern,
            destination,
            input.enabled,
            priority
        ],
    )?;
    Ok(Rule {
        id,
        match_type: input.match_type,
        pattern,
        destination,
        enabled: input.enabled,
        priority,
    })
}

pub fn delete(conn: &Connection, id: &str) -> CoreResult<()> {
    Uuid::parse_str(id).map_err(|_| CoreError::Invalid("Invalid rule ID".into()))?;
    if conn.execute("DELETE FROM rules WHERE id=?1", [id])? == 0 {
        return Err(CoreError::NotFound);
    }
    Ok(())
}

pub fn reorder(conn: &Connection, ids: Vec<String>) -> CoreResult<Vec<Rule>> {
    let existing = list(conn)?;
    let expected: HashSet<_> = existing.iter().map(|rule| rule.id.as_str()).collect();
    let provided: HashSet<_> = ids.iter().map(String::as_str).collect();
    if ids.len() != existing.len() || provided != expected {
        return Err(CoreError::Invalid(
            "Rule order must contain every rule once".into(),
        ));
    }
    let transaction = conn.unchecked_transaction()?;
    for (index, id) in ids.iter().enumerate() {
        transaction.execute(
            "UPDATE rules SET priority=?2 WHERE id=?1",
            params![id, -((index as i64) + 1)],
        )?;
    }
    for (index, id) in ids.iter().enumerate() {
        transaction.execute(
            "UPDATE rules SET priority=?2 WHERE id=?1",
            params![id, (index as i64) + 1],
        )?;
    }
    transaction.commit()?;
    list(conn)
}

pub fn suggest(conn: &Connection, file_ids: Vec<i64>) -> CoreResult<Vec<RuleSuggestion>> {
    if file_ids.is_empty() || file_ids.len() > 100 {
        return Err(CoreError::Invalid("Select 1–100 files".into()));
    }
    let unique: HashSet<_> = file_ids.iter().copied().collect();
    if unique.len() != file_ids.len() {
        return Err(CoreError::Invalid("Duplicate file selection".into()));
    }
    let rules = list(conn)?;
    let summaries = load_file_summaries(conn, &file_ids)?;
    let mut results = Vec::with_capacity(file_ids.len());
    for id in file_ids {
        let (name, kind) = summaries.get(&id).cloned().ok_or(CoreError::NotFound)?;
        if kind != "file" {
            return Err(CoreError::Invalid(
                "Rules apply to ordinary files only".into(),
            ));
        }
        let matched = rules
            .iter()
            .filter(|rule| rule.enabled)
            .find(|rule| matches_rule(rule, &name));
        results.push(RuleSuggestion {
            file_id: id,
            file_name: name,
            destination: matched.map(|rule| rule.destination.clone()),
            rule_id: matched.map(|rule| rule.id.clone()),
        });
    }
    Ok(results)
}

/// One group of files that a single enabled rule sends to one destination.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OrganizeGroup {
    pub destination: String,
    pub file_count: usize,
    pub plan: ActionPlan,
    pub preview: PlanPreview,
}

/// Result of applying every enabled rule to the whole Desktop index.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OrganizeResult {
    pub groups: Vec<OrganizeGroup>,
    pub unmatched: Vec<String>,
    pub total_files: usize,
    pub matched_files: usize,
}

/// A plan cannot exceed the Core batch limit; oversized groups are split.
const MAX_PLAN_ITEMS: usize = 100;
/// A safety ceiling so one click cannot queue an unbounded number of plans.
const MAX_GROUPS: usize = 40;

/// Applies enabled rules to every indexed top-level file and turns the result
/// into validated ActionPlans grouped by destination. Nothing is moved here:
/// each group still needs the normal preview + explicit execute path.
pub fn organize(core: &mut DesktopCore) -> CoreResult<OrganizeResult> {
    let files = core.list_files()?;
    let rules = list(core.connection())?;
    let enabled: Vec<&Rule> = rules.iter().filter(|rule| rule.enabled).collect();
    let mut grouped: BTreeMap<String, Vec<i64>> = BTreeMap::new();
    let mut unmatched = Vec::new();
    let mut total_files = 0;
    for file in &files {
        if file.kind != "file" {
            continue;
        }
        total_files += 1;
        match enabled.iter().find(|rule| matches_rule(rule, &file.name)) {
            Some(rule) => grouped
                .entry(rule.destination.clone())
                .or_default()
                .push(file.id),
            None => unmatched.push(file.name.clone()),
        }
    }
    let mut groups = Vec::new();
    'outer: for (destination, ids) in grouped {
        for chunk in ids.chunks(MAX_PLAN_ITEMS) {
            if groups.len() >= MAX_GROUPS {
                break 'outer;
            }
            let plan = core.create_move_plan(chunk.to_vec(), destination.clone())?;
            let preview = core.validate_plan(&plan.id)?;
            groups.push(OrganizeGroup {
                destination: destination.clone(),
                file_count: plan.items.len(),
                plan,
                preview,
            });
        }
    }
    let matched_files = groups.iter().map(|group| group.file_count).sum();
    Ok(OrganizeResult {
        groups,
        unmatched,
        total_files,
        matched_files,
    })
}

fn normalize_pattern(kind: &MatchType, value: &str) -> CoreResult<String> {
    let pattern = value.trim();
    if pattern.is_empty() || pattern.chars().count() > 128 || pattern.chars().any(char::is_control)
    {
        return Err(CoreError::Invalid(
            "Rule pattern must be 1–128 characters".into(),
        ));
    }
    match kind {
        MatchType::Extension => {
            let extension = pattern.trim_start_matches('.');
            if extension.is_empty()
                || extension.len() > 24
                || !extension.chars().all(|c| c.is_ascii_alphanumeric())
            {
                return Err(CoreError::Invalid("Invalid extension rule".into()));
            }
            Ok(extension.to_ascii_lowercase())
        }
        MatchType::Name => Ok(pattern.to_lowercase()),
    }
}

fn matches_rule(rule: &Rule, name: &str) -> bool {
    match rule.match_type {
        MatchType::Extension => Path::new(name)
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| ext.eq_ignore_ascii_case(&rule.pattern)),
        MatchType::Name => glob_matches(&rule.pattern, &name.to_lowercase()),
    }
}

fn glob_matches(pattern: &str, text: &str) -> bool {
    let pattern: Vec<char> = pattern.chars().collect();
    let text: Vec<char> = text.chars().collect();
    let mut previous = vec![false; text.len() + 1];
    previous[0] = true;
    for token in pattern {
        let mut current = vec![false; text.len() + 1];
        if token == '*' {
            current[0] = previous[0];
        }
        for index in 1..=text.len() {
            current[index] = match token {
                '*' => previous[index] || current[index - 1],
                '?' => previous[index - 1],
                literal => previous[index - 1] && literal == text[index - 1],
            };
        }
        previous = current;
    }
    previous[text.len()]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ordered_rules_suggest_only_selected_names() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE files(id INTEGER PRIMARY KEY,name TEXT NOT NULL,kind TEXT NOT NULL);
             INSERT INTO files(id,name,kind) VALUES(1,'report.pdf','file'),(2,'photo.jpg','file');",
        )
        .unwrap();
        init_schema(&conn).unwrap();
        upsert(
            &conn,
            RuleInput {
                id: None,
                match_type: MatchType::Extension,
                pattern: ".pdf".into(),
                destination: "Documents".into(),
                enabled: true,
            },
        )
        .unwrap();
        let suggestions = suggest(&conn, vec![1, 2]).unwrap();
        assert_eq!(suggestions[0].destination.as_deref(), Some("Documents"));
        assert_eq!(suggestions[1].destination, None);
        let rules = list(&conn).unwrap();
        assert_eq!(reorder(&conn, vec![rules[0].id.clone()]).unwrap().len(), 1);
    }

    #[test]
    fn name_globs_are_deterministic() {
        assert!(glob_matches("*report?.pdf", "annual-report1.pdf"));
        assert!(!glob_matches("*report?.pdf", "annual-report.pdf"));
    }

    #[test]
    fn organize_groups_files_by_rule_destination() {
        use std::fs;

        let temp = tempfile::tempdir().unwrap();
        let desktop = temp.path().join("Desktop");
        fs::create_dir(&desktop).unwrap();
        fs::write(desktop.join("report.pdf"), b"a").unwrap();
        fs::write(desktop.join("notes.txt"), b"b").unwrap();
        fs::write(desktop.join("misc.bin"), b"c").unwrap();
        let mut core = DesktopCore::open(&desktop, &temp.path().join("index.sqlite")).unwrap();
        init_schema(core.connection()).unwrap();
        core.scan_desktop().unwrap();
        upsert(
            core.connection(),
            RuleInput {
                id: None,
                match_type: MatchType::Extension,
                pattern: "pdf".into(),
                destination: "Documents".into(),
                enabled: true,
            },
        )
        .unwrap();
        upsert(
            core.connection(),
            RuleInput {
                id: None,
                match_type: MatchType::Extension,
                pattern: "txt".into(),
                destination: "Documents".into(),
                enabled: true,
            },
        )
        .unwrap();

        let result = organize(&mut core).unwrap();
        assert_eq!(result.total_files, 3);
        assert_eq!(result.matched_files, 2);
        assert_eq!(result.groups.len(), 1);
        assert_eq!(result.groups[0].destination, "Documents");
        assert_eq!(result.groups[0].file_count, 2);
        assert!(result.groups[0].preview.valid);
        assert_eq!(result.unmatched, vec!["misc.bin".to_string()]);
        // Organizing only proposes plans; it never moves files.
        assert!(desktop.join("report.pdf").exists());
        assert!(!desktop.join("Documents").exists());
    }
}
