# Desktop Manager v0.1

Windows Desktop organizer with a Rust-only file operation boundary. The app
indexes Desktop's immediate children, lets the user preview a Move ActionPlan,
executes it as a journaled transaction, and offers conflict-safe Undo. Rules
and AI can suggest a destination but cannot touch the filesystem.

## Highlights

- Modular workspace: every panel is a module the user creates and removes.
  "＋ 模块" adds one, the × in a panel header removes it, and the layout is
  remembered. Nothing is hard-wired into the window.
- Settings modules collapse once they are configured. The AI Provider module
  becomes a one-line summary as soon as every provider has a stored key, and
  "管理" reopens it on demand.
- Real Windows Shell icons in the file list, extracted per type (and per file
  for executables and shortcuts), cached, and shown unmodified.
- One-click Smart Organize: enabled rules are applied to the whole index and
  grouped into per-folder plans. Every group is validated before it can run,
  blocked groups are skipped, and unmatched files stay in place.
- Search, sortable file list (name / modified / size / kind), and destination
  autocomplete for existing Desktop folders.
- Bounded, single-query history loading and batched index/rule/AI lookups keep
  refresh cost flat as the Desktop and history grow.
- Friendly Chinese messages for common Core errors, auto-dismissing notices,
  Escape to clear, and automatic dark mode following the Windows theme.

## Run

Prerequisites: Windows, Node.js, the MSVC Rust toolchain, Visual Studio C++
Build Tools, and WebView2.

For a ready-to-use Windows installation, double-click the generated
`src-tauri/target/release/bundle/nsis/Desktop Manager_0.1.0_x64-setup.exe`.
It installs for the current user without administrator rights and creates
Desktop and Start Menu shortcuts. Use either shortcut to start the app.
The installer is currently unsigned, so Windows may show a publisher warning.
The app opens as a transparent desktop workspace across the usable screen,
with the wallpaper visible behind its panels. Its top bar provides minimize
and close controls; the Windows taskbar remains available.

To develop from source:

    npm install
    npm run tauri -- dev

## Verify and build

    npm run build
    cd src-tauri
    cargo fmt --check
    cargo check
    cargo test
    cd ..
    npm run tauri -- build

The final command writes a Windows NSIS installer at
`src-tauri/target/release/bundle/nsis/` and a runnable executable at
`src-tauri/target/release/desktop-manager.exe`. SQLite is stored under the
current user's local app data directory; provider API keys are stored in
Windows Credential Manager.

## Safety and scope

Only ordinary files directly on the current user's Desktop may be moved to
one direct child folder. Existing targets, changed files, links, and path
traversal are rejected. AI calls require a separate request preview and send
action, then produce a plan that still needs user execution.

Read SPEC.md, ARCHITECTURE.md, SECURITY.md, and ACTION_PLAN.md for the
accepted Q1–Q11 decisions and implementation details. The original Q1–Q68
record was not available in this workspace.
