# Desktop Manager v0.1 — Product specification

## Decision record

The original Q1–Q68 grill-me transcript was not present in the accessible local
conversation or workspace. The user explicitly reran grill-me and accepted the
Q1–Q11 recommendations in this conversation. This document records those
accepted decisions; it does not claim to reconstruct missing answers.

## Goal and platform

Desktop Manager is a local Windows app built with Tauri 2, React/TypeScript,
Rust Desktop Core, SQLite, and a file watcher. The first delivery path is:

Desktop scan → SQLite index → file selection → persistent Move ActionPlan →
validation and policy preview → explicit execute → transaction history → Undo.

Only the current user's Windows Known Folder Desktop is managed. The first
version indexes its immediate children. Directories may be shown but only
ordinary top-level files can be moved. Links, junctions, special files, and
recursive folder moves are outside the first Move policy.

The main window is a desktop-sized, borderless, transparent workspace. Its
background leaves the current Windows wallpaper visible, while Desktop
Manager's file, plan, history, rule, provider, and AI panels are arranged
across the available work area. The taskbar remains available, and the
workspace has explicit minimize and close controls. "All windows" here
means these Desktop Manager panels, not windows belonging to other programs.

The destination is one ordinary folder name directly under Desktop. Core may
create that folder during execution. A target collision invalidates the whole
plan; no overwrite or automatic renaming is allowed. Batch execution moves
completed items back if a later item fails.

Every Move requires a user selected plan, successful preview, and an explicit
Execute action. Rules and AI only suggest or construct plans. They never
execute a file operation. Undo is allowed only while each moved file remains
unchanged and its original path is vacant. History is retained locally.

## Rules and AI

The rule engine uses ordered, editable extension and file-name rules to suggest
a destination. Unmatched files have no automatic category. A user must select
a suggestion to create a plan. Rule output follows the same validation path
as a manually entered destination.

Provider configuration supports OpenAI, DeepSeek, Xiaomi MiMo,
OpenAI-compatible services, and custom Base URL/model. Preset URLs and models
are editable. Remote URLs require HTTPS; loopback HTTP requires explicit
local-use choice. API keys are stored through Windows Credential Manager,
never in SQLite or returned provider objects.

An AI request is user-triggered for selected files. Its request summary is
shown before sending. It may contain selected names, extensions, and the
user's instruction; it must not contain absolute paths, file content, or
unselected file data. Model output is untrusted structured ActionPlan input
and follows the same Core validator, policy, transaction, and Undo path.

The local index records file name, path, type, size, and modification time,
plus transaction metadata. It does not read file contents. SQLite is stored
under the current Windows user's app-local data directory and relies on
per-user filesystem permissions rather than additional database encryption.

## Delivery stages

1. Stage 1: manual Move closed loop, watcher, local history, and Undo.
2. Stage 2: editable deterministic rule suggestions and provider
   configuration with Credential Manager key storage.
3. Stage 3: user-triggered provider inference that produces structured
   plans; privacy review and the same Core execution pipeline.

Each stage must compile and pass relevant tests. A Git commit is a rollback
point for each completed stage. Features not yet implemented are called out
in the delivery report.

The current implementation contains all three stages. Provider requests use
the OpenAI-compatible Chat Completions protocol. Live calls require the user
to configure a valid endpoint and key; automated tests use a local mock
provider and do not spend external API credits.
