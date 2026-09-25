# Security model

## Non-bypassable boundary

LLM → Structured ActionPlan → Validator → Policy Engine → Transaction
Executor → File System.

The LLM, a skill, a rule, and React cannot call PowerShell, a shell plugin,
or a Tauri filesystem plugin to modify files. Native Core commands form the
only mutation boundary. A generated plan does not grant execution authority;
the user must preview and execute it.

## Move policy

- Resolve Desktop with the Windows Known Folder API through Tauri.
- Accept source IDs from SQLite, then verify each source is an immediate
  Desktop child and an ordinary file.
- Accept one normal destination folder name; reject separators, traversal,
  device names, trailing spaces/dots, links, and redirects.
- Recheck source metadata and destination absence at preview and immediately
  before each move. Never replace an existing target.
- Move only within Desktop so source and destination share a volume.
- Use a no-replace Windows rename, journal progress, and compensate failures.
- Undo requires unchanged moved files and vacant original paths.

There remains a narrow filesystem race between validation and Windows rename.
No-replace semantics prevent destination overwrite. A malicious process able
to replace Desktop directories concurrently may still cause an operation to
fail or require manual recovery. The current policy does not support elevated
privileges or an untrusted multi-user Desktop.

## Provider and privacy policy

Only provider metadata is stored in SQLite. API keys use Windows Credential
Manager and are write-only from the UI perspective; errors and history never
include key material. Remote custom endpoints require HTTPS. Loopback HTTP
requires explicit local-use selection.

The index does not inspect file contents. Before a user-triggered AI request,
the UI shows the request summary. It sends selected names/extensions and the
user instruction only; no absolute paths or unselected files. Model responses
are untrusted and cannot contain executable filesystem commands.

## Tauri capability

The packaged app has one trusted local window with core:default only. No
plugin-fs or plugin-shell permissions are granted. If additional windows or
remote content are introduced, app commands must be restricted with a
Tauri app manifest and per-window capabilities before shipping.
