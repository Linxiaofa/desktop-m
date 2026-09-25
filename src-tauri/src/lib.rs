mod core;
mod provider;

use core::{
    ActionPlan, CoreResult, DesktopCore, FileEntry, HistoryEntry, PlanPreview, ScanSummary,
    TransactionResult,
};
use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use provider::{ProviderConfig, ProviderInput};
use std::sync::{mpsc, Arc, Mutex};
use std::time::Duration;
use tauri::{Emitter, Manager, State};

struct AppState {
    core: Arc<Mutex<DesktopCore>>,
    _watcher: Mutex<RecommendedWatcher>,
}

async fn call_core<T, F>(state: State<'_, AppState>, work: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce(&mut DesktopCore) -> CoreResult<T> + Send + 'static,
{
    let core = Arc::clone(&state.core);
    tauri::async_runtime::spawn_blocking(move || {
        let mut core = core
            .lock()
            .map_err(|_| "Desktop Core lock poisoned".to_string())?;
        work(&mut core).map_err(|error| error.to_string())
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
async fn scan_desktop(state: State<'_, AppState>) -> Result<ScanSummary, String> {
    call_core(state, |core| core.scan_desktop()).await
}

#[tauri::command]
async fn list_files(state: State<'_, AppState>) -> Result<Vec<FileEntry>, String> {
    call_core(state, |core| core.list_files()).await
}

#[tauri::command]
async fn create_move_plan(
    state: State<'_, AppState>,
    file_ids: Vec<i64>,
    destination: String,
) -> Result<ActionPlan, String> {
    call_core(state, move |core| {
        core.create_move_plan(file_ids, destination)
    })
    .await
}

#[tauri::command]
async fn validate_plan(state: State<'_, AppState>, plan_id: String) -> Result<PlanPreview, String> {
    call_core(state, move |core| core.validate_plan(&plan_id)).await
}

#[tauri::command]
async fn execute_plan(
    state: State<'_, AppState>,
    plan_id: String,
) -> Result<TransactionResult, String> {
    call_core(state, move |core| core.execute_plan(&plan_id)).await
}

#[tauri::command]
async fn list_history(state: State<'_, AppState>) -> Result<Vec<HistoryEntry>, String> {
    call_core(state, |core| core.list_history()).await
}

#[tauri::command]
async fn undo_transaction(
    state: State<'_, AppState>,
    transaction_id: String,
) -> Result<TransactionResult, String> {
    call_core(state, move |core| core.undo_transaction(&transaction_id)).await
}

#[tauri::command]
async fn list_providers(state: State<'_, AppState>) -> Result<Vec<ProviderConfig>, String> {
    let core = Arc::clone(&state.core);
    tauri::async_runtime::spawn_blocking(move || {
        let core = core
            .lock()
            .map_err(|_| "Desktop Core lock poisoned".to_string())?;
        provider::list(core.connection()).map_err(|error| error.to_string())
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
async fn save_provider(
    state: State<'_, AppState>,
    input: ProviderInput,
) -> Result<ProviderConfig, String> {
    let core = Arc::clone(&state.core);
    tauri::async_runtime::spawn_blocking(move || {
        let core = core
            .lock()
            .map_err(|_| "Desktop Core lock poisoned".to_string())?;
        provider::upsert(core.connection(), input).map_err(|error| error.to_string())
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
async fn delete_provider(state: State<'_, AppState>, id: String) -> Result<(), String> {
    let core = Arc::clone(&state.core);
    tauri::async_runtime::spawn_blocking(move || {
        let core = core
            .lock()
            .map_err(|_| "Desktop Core lock poisoned".to_string())?;
        provider::delete(core.connection(), &id).map_err(|error| error.to_string())
    })
    .await
    .map_err(|error| error.to_string())?
}

pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            let desktop = app.path().desktop_dir()?;
            let data_dir = app.path().app_local_data_dir()?;
            let mut core = DesktopCore::open(&desktop, &data_dir.join("index.sqlite"))?;
            provider::init_schema(core.connection())?;
            core.scan_desktop()?;
            let shared = Arc::new(Mutex::new(core));
            let (sender, receiver) = mpsc::channel();
            let mut watcher = notify::recommended_watcher(move |event| {
                let _ = sender.send(event);
            })?;
            watcher.watch(&desktop, RecursiveMode::NonRecursive)?;
            let worker_core = Arc::clone(&shared);
            let app_handle = app.handle().clone();
            std::thread::spawn(move || {
                while receiver.recv().is_ok() {
                    while receiver.recv_timeout(Duration::from_millis(350)).is_ok() {}
                    let summary = worker_core
                        .lock()
                        .ok()
                        .and_then(|mut core| core.scan_desktop().ok());
                    if let Some(summary) = summary {
                        let _ = app_handle.emit("desktop-index-updated", summary);
                    }
                }
            });
            app.manage(AppState {
                core: shared,
                _watcher: Mutex::new(watcher),
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            scan_desktop,
            list_files,
            create_move_plan,
            validate_plan,
            execute_plan,
            list_history,
            undo_transaction,
            list_providers,
            save_provider,
            delete_provider
        ])
        .run(tauri::generate_context!())
        .expect("error while running Desktop Manager");
}
