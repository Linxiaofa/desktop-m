# Desktop Manager architecture

## Trust and data flow

React UI / Rule Engine / LLM
→ structured request
→ Rust Desktop Core command boundary
→ ActionPlan schema and source snapshot
→ Validator → Policy Engine → Preview
→ explicit user Execute
→ Transaction Executor ↔ SQLite journal
→ Windows filesystem

The UI has no Tauri filesystem or shell plugin capability. The only native
mutation entry points are Core commands. Each command distrusts its inputs
even when they come from the packaged UI.

## Components

- React/TypeScript renders indexed Desktop entries, collects selection,
  displays plan checks and history, and calls explicit Tauri commands.
- Rust Desktop Core resolves Windows Known Folder Desktop, owns the SQLite
  connection and policy checks, creates persistent immutable plan items, and
  serializes transactions through a mutex.
- SQLite stores the current top-level index, plans, transaction journal, item
  states, history, rules, and non-secret provider metadata. WAL and
  synchronous FULL are enabled.
- The watcher observes Desktop's immediate children, debounces events, and
  rescans the authoritative directory. Execute and Undo also rescan. Watcher
  events are hints; filesystem validation is authoritative.
- Credential Manager stores each provider API key under the app's service
  name and provider UUID. Keys never enter SQLite.

## Command contract

Stage 1 commands: scan_desktop, list_files, create_move_plan, validate_plan,
execute_plan, list_history, undo_transaction. Provider configuration commands:
list_providers, save_provider, delete_provider. IDs are resolved inside Core;
the UI never supplies raw paths to the executor.

## Transaction states

An ActionPlan starts as draft. Execution stores an executing journal before
the first rename, then marks each moved item. Success marks the transaction
and plan executed. Failure compensates previous moves in reverse order; an
uncompensated or interrupted operation is marked recovery_needed and cannot
be silently replayed. Undo prevalidates all items, records undoing, renames
in reverse order, and ends at undone. Conflicting Undo leaves data untouched;
a mid-Undo failure is compensated where possible.

SQLite and NTFS cannot share one atomic commit. The journal is a durable
record of intended and observed steps, not a cross-system ACID guarantee.
Recovery-needed records require explicit inspection before further action.
