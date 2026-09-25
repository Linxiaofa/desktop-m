mod ai;
mod core;
mod provider;
mod rules;

use core::{
    ActionPlan, CoreError, CoreResult, DesktopCore, FileEntry, HistoryEntry, PlanPreview,
    ScanSummary, TransactionResult,
};
use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use provider::{ProviderConfig, ProviderInput};
use rules::{Rule, RuleInput, RuleSuggestion};
use serde::Serialize;
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

#[tauri::command]
async fn list_rules(state: State<'_, AppState>) -> Result<Vec<Rule>, String> {
    call_core(state, |core| rules::list(core.connection())).await
}

#[tauri::command]
async fn save_rule(state: State<'_, AppState>, input: RuleInput) -> Result<Rule, String> {
    call_core(state, move |core| rules::upsert(core.connection(), input)).await
}

#[tauri::command]
async fn delete_rule(state: State<'_, AppState>, id: String) -> Result<(), String> {
    call_core(state, move |core| rules::delete(core.connection(), &id)).await
}

#[tauri::command]
async fn reorder_rules(state: State<'_, AppState>, ids: Vec<String>) -> Result<Vec<Rule>, String> {
    call_core(state, move |core| rules::reorder(core.connection(), ids)).await
}

#[tauri::command]
async fn suggest_rules(
    state: State<'_, AppState>,
    file_ids: Vec<i64>,
) -> Result<Vec<RuleSuggestion>, String> {
    call_core(state, move |core| {
        rules::suggest(core.connection(), file_ids)
    })
    .await
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AiPlanResult {
    plan: ActionPlan,
    preview: PlanPreview,
    destination: String,
    reason: String,
}

#[tauri::command]
async fn preview_ai_request(
    state: State<'_, AppState>,
    file_ids: Vec<i64>,
    provider_id: String,
    instruction: String,
) -> Result<ai::AiRequestPreview, String> {
    call_core(state, move |core| {
        core.scan_desktop()?;
        ai::preview(core.connection(), file_ids, provider_id, instruction)
    })
    .await
}

#[tauri::command]
async fn generate_ai_plan(
    state: State<'_, AppState>,
    request_id: String,
) -> Result<AiPlanResult, String> {
    let shared = Arc::clone(&state.core);
    let prepare_core = Arc::clone(&shared);
    let prepare_id = request_id.clone();
    let prepared = tauri::async_runtime::spawn_blocking(move || {
        let mut core = prepare_core
            .lock()
            .map_err(|_| "Desktop Core lock poisoned".to_string())?;
        core.scan_desktop().map_err(|error| error.to_string())?;
        ai::prepare(core.connection(), &prepare_id).map_err(|error| error.to_string())
    })
    .await
    .map_err(|error| error.to_string())??;

    let suggestion = match ai::request_model(&prepared).await {
        Ok(suggestion) => suggestion,
        Err(error) => {
            let failed_core = Arc::clone(&shared);
            let failed_id = request_id.clone();
            let _ = tauri::async_runtime::spawn_blocking(move || {
                if let Ok(core) = failed_core.lock() {
                    let _ = ai::finish(core.connection(), &failed_id, "failed", None);
                }
            })
            .await;
            return Err(error.to_string());
        }
    };

    tauri::async_runtime::spawn_blocking(move || {
        let mut core = shared
            .lock()
            .map_err(|_| "Desktop Core lock poisoned".to_string())?;
        let result = (|| -> CoreResult<AiPlanResult> {
            core.scan_desktop()?;
            let plan = core.create_move_plan(prepared.file_ids, suggestion.destination.clone())?;
            let preview = core.validate_plan(&plan.id)?;
            if !preview.valid {
                return Err(CoreError::Invalid(preview.issues.join("; ")));
            }
            ai::finish(core.connection(), &request_id, "completed", Some(&plan.id))?;
            Ok(AiPlanResult {
                plan,
                preview,
                destination: suggestion.destination,
                reason: suggestion.reason,
            })
        })();
        if result.is_err() {
            let _ = ai::finish(core.connection(), &request_id, "rejected", None);
        }
        result.map_err(|error| error.to_string())
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
            rules::init_schema(core.connection())?;
            ai::init_schema(core.connection())?;
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
            delete_provider,
            list_rules,
            save_rule,
            delete_rule,
            reorder_rules,
            suggest_rules,
            preview_ai_request,
            generate_ai_plan
        ])
        .run(tauri::generate_context!())
        .expect("error while running Desktop Manager");
}
