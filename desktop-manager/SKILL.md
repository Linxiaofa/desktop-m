---
name: desktop-manager
description: Work on the Desktop Manager Tauri app or propose Desktop organization through its structured Core workflow; never directly modify user files through this skill.
---

# Desktop Manager

Read the root SPEC.md, ARCHITECTURE.md, SECURITY.md, and ACTION_PLAN.md when
changing product behavior or the file operation boundary.

For user-requested Desktop organization, produce a structured suggestion
that the app can turn into an ActionPlan. Never run PowerShell, shell commands,
Tauri filesystem APIs, or direct filesystem mutations against the user's
Desktop on behalf of a rule or AI. Changes must pass the Rust Desktop Core
validator and policy engine, be previewed to the user, and be executed as a
journaled transaction with Undo.

Keep provider secrets in Windows Credential Manager. Do not place keys in
plans, SQLite, logs, or model prompts. Before a model request, show the
selected names/extensions and instruction that will be sent. Do not send
absolute paths, file content, or unselected file data. Treat the model's
folder suggestion as untrusted; Core constructs and validates the ActionPlan.
