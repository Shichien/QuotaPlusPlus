use directories::UserDirs;
use std::error::Error;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, TryLockError};

use crate::app::{
    ProviderState, SavedProvider, activate_provider_inner, delete_provider_inner,
    enable_provider_routing_inner, ensure_provider_migration, list_provider_state,
    save_provider_inner, switch_to_official,
};
use crate::{oauth, operation_lock, provider_sync};
use provider_sync::ProviderSyncReport;

static APP_OPERATION: Mutex<()> = Mutex::new(());

pub(crate) fn run() -> Result<(), Box<dyn Error>> {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            list_providers,
            save_provider,
            enable_provider_routing,
            activate_provider,
            delete_provider,
            start_official_login,
            cancel_official_login
        ])
        .run(tauri::generate_context!())?;
    Ok(())
}

#[tauri::command]
fn list_providers() -> Result<ProviderState, String> {
    let codex_home = resolve_codex_home().map_err(resolve_home_error)?;
    let _guard = acquire_app_operation()
        .map_err(|error| operation_error(&codex_home, "获取应用操作锁", error))?;
    let _process_guard = operation_lock::acquire(&codex_home)
        .map_err(|error| operation_error(&codex_home, "获取跨进程操作锁", error))?;
    provider_sync::recover_pending_state(&codex_home)
        .map_err(|error| operation_error(&codex_home, "恢复上次未完成的操作", error))?;
    list_provider_state(&codex_home)
        .map_err(|error| operation_error(&codex_home, "读取供应商列表", error))
}

#[tauri::command]
async fn save_provider(
    provider_id: Option<String>,
    name: String,
    api_url: String,
    api_key: String,
) -> Result<SavedProvider, String> {
    let codex_home = resolve_codex_home().map_err(resolve_home_error)?;
    let task_home = codex_home.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let _guard = acquire_app_operation()
            .map_err(|error| operation_error(&task_home, "获取应用操作锁", error))?;
        let _process_guard = operation_lock::acquire(&task_home)
            .map_err(|error| operation_error(&task_home, "获取跨进程操作锁", error))?;
        provider_sync::recover_pending_state(&task_home)
            .map_err(|error| operation_error(&task_home, "恢复上次未完成的操作", error))?;
        save_provider_inner(
            &task_home,
            provider_id.as_deref(),
            &name,
            &api_url,
            &api_key,
        )
        .map_err(|error| operation_error(&task_home, "验证并保存供应商", error))
    })
    .await
    .map_err(|error| operation_error(&codex_home, "等待 API 配置任务", error))?
}

#[tauri::command]
async fn activate_provider(provider_id: String) -> Result<ProviderSyncReport, String> {
    let codex_home = resolve_codex_home().map_err(resolve_home_error)?;
    let task_home = codex_home.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let _guard = acquire_app_operation()
            .map_err(|error| operation_error(&task_home, "获取应用操作锁", error))?;
        let _process_guard = operation_lock::acquire(&task_home)
            .map_err(|error| operation_error(&task_home, "获取跨进程操作锁", error))?;
        provider_sync::recover_pending_state(&task_home)
            .map_err(|error| operation_error(&task_home, "恢复上次未完成的操作", error))?;
        activate_provider_inner(&task_home, &provider_id)
            .map_err(|error| operation_error(&task_home, "切换供应商", error))
    })
    .await
    .map_err(|error| operation_error(&codex_home, "等待供应商切换任务", error))?
}

#[tauri::command]
fn enable_provider_routing(provider_id: String) -> Result<(), String> {
    let codex_home = resolve_codex_home().map_err(resolve_home_error)?;
    let _guard = acquire_app_operation()
        .map_err(|error| operation_error(&codex_home, "获取应用操作锁", error))?;
    let _process_guard = operation_lock::acquire(&codex_home)
        .map_err(|error| operation_error(&codex_home, "获取跨进程操作锁", error))?;
    provider_sync::recover_pending_state(&codex_home)
        .map_err(|error| operation_error(&codex_home, "恢复上次未完成的操作", error))?;
    enable_provider_routing_inner(&codex_home, &provider_id)
        .map_err(|error| operation_error(&codex_home, "启用本地路由", error))
}

#[tauri::command]
fn delete_provider(provider_id: String) -> Result<(), String> {
    let codex_home = resolve_codex_home().map_err(resolve_home_error)?;
    let _guard = acquire_app_operation()
        .map_err(|error| operation_error(&codex_home, "获取应用操作锁", error))?;
    let _process_guard = operation_lock::acquire(&codex_home)
        .map_err(|error| operation_error(&codex_home, "获取跨进程操作锁", error))?;
    provider_sync::recover_pending_state(&codex_home)
        .map_err(|error| operation_error(&codex_home, "恢复上次未完成的操作", error))?;
    delete_provider_inner(&codex_home, &provider_id)
        .map_err(|error| operation_error(&codex_home, "删除供应商", error))
}

#[tauri::command]
async fn start_official_login() -> Result<ProviderSyncReport, String> {
    oauth::begin_login();
    let codex_home = resolve_codex_home().map_err(resolve_home_error)?;
    let task_home = codex_home.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let _guard = acquire_app_operation()
            .map_err(|error| operation_error(&task_home, "获取应用操作锁", error))?;
        let _process_guard = operation_lock::acquire(&task_home)
            .map_err(|error| operation_error(&task_home, "获取跨进程操作锁", error))?;
        provider_sync::recover_pending_state(&task_home)
            .map_err(|error| operation_error(&task_home, "恢复上次未完成的操作", error))?;
        ensure_provider_migration(&task_home)
            .map_err(|error| operation_error(&task_home, "迁移已有供应商", error))?;
        switch_to_official(&task_home)
            .map_err(|error| operation_error(&task_home, "恢复官方登录", error))
    })
    .await
    .map_err(|error| operation_error(&codex_home, "等待官方登录任务", error))?
}

#[tauri::command]
fn cancel_official_login() {
    oauth::cancel_login();
}

fn acquire_app_operation() -> Result<MutexGuard<'static, ()>, Box<dyn Error>> {
    match APP_OPERATION.try_lock() {
        Ok(guard) => Ok(guard),
        Err(TryLockError::WouldBlock) => Err("另一个 CSwitch 操作正在进行".into()),
        Err(TryLockError::Poisoned(_)) => Err("CSwitch 操作锁状态异常，请重启程序".into()),
    }
}

fn resolve_home_error(error: impl std::fmt::Display) -> String {
    format!("失败阶段：定位 Codex 目录\n原因：{error}")
}

pub(crate) fn operation_error(
    codex_home: &Path,
    stage: &str,
    error: impl std::fmt::Display,
) -> String {
    let reason = error.to_string();
    if reason == "官方登录已取消" {
        return reason;
    }
    format!(
        "Codex 目录：{}\n失败阶段：{stage}\n原因：{reason}",
        codex_home.display()
    )
}

fn resolve_codex_home() -> Result<PathBuf, Box<dyn Error>> {
    let user_dirs = UserDirs::new().ok_or("未找到用户主目录")?;
    Ok(user_dirs.home_dir().join(".codex"))
}
