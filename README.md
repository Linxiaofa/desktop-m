# Desktop Manager v0.1

Windows Desktop organizer with a Rust-only file operation boundary. The app
indexes Desktop's immediate children, lets the user preview a Move ActionPlan,
executes it as a journaled transaction, and offers conflict-safe Undo. Rules
and AI can suggest a destination but cannot touch the filesystem.

## Run

Prerequisites: Windows, Node.js, the MSVC Rust toolchain, Visual Studio C++
Build Tools, and WebView2.

    npm install
    npm run tauri -- dev

## Verify and build

    npm run build
    cd src-tauri
    cargo fmt --check
    cargo check
    cargo test
    cd ..
    npm run tauri -- build --debug

The final command writes a runnable debug executable at
src-tauri/target/debug/desktop-manager.exe. Bundled installers are not
configured yet. SQLite is stored under the current user's local app data
directory; provider API keys are stored in Windows Credential Manager.

## Safety and scope

Only ordinary files directly on the current user's Desktop may be moved to
one direct child folder. Existing targets, changed files, links, and path
traversal are rejected. AI calls require a separate request preview and send
action, then produce a plan that still needs user execution.

Read SPEC.md, ARCHITECTURE.md, SECURITY.md, and ACTION_PLAN.md for the
accepted Q1–Q11 decisions and implementation details. The original Q1–Q68
record was not available in this workspace.
