use std::fs;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use anyhow::{Context, Result};
use tauri::Emitter;

use crate::core::elevated::start_helper_elevated;
use crate::core::elevated::{is_running_elevated, restart_current_app_elevated};
use crate::core::journal;
use crate::core::locks::detect_locks_with_progress;
use crate::core::models::{
    HelperAction, HelperInvocation, LogRead, MovePreview, MoveRequest, OperationSnapshot,
    OperationStatus, ProgressSnapshot,
};
use crate::core::pathing::build_preview;

#[tauri::command]
pub fn is_elevated() -> Result<bool, String> {
    Ok(is_running_elevated())
}

#[tauri::command]
pub fn restart_as_admin(app: tauri::AppHandle) -> Result<(), String> {
    restart_current_app_elevated().map_err(error_string)?;
    app.exit(0);
    Ok(())
}

#[tauri::command]
pub fn preview_move(app: tauri::AppHandle, request: MoveRequest) -> Result<MovePreview, String> {
    let mut preview = build_preview(&request).map_err(error_string)?;
    preview.locks =
        detect_locks_with_progress(Path::new(&preview.source_path), |current, total, _| {
            emit_progress(
                &app,
                current,
                total,
                if total == 0 {
                    "В источнике нет файлов для проверки"
                } else {
                    "Проверяю занятые файлы"
                },
            );
        })
        .map_err(error_string)?;
    Ok(preview)
}

#[tauri::command]
pub fn start_move(
    app: tauri::AppHandle,
    request: MoveRequest,
) -> Result<OperationSnapshot, String> {
    let mut preview = build_preview(&request).map_err(error_string)?;
    if request.skip_lock_check.unwrap_or(false) {
        emit_progress(&app, 1, 1, "Использую свежую проверку блокировок");
    } else {
        preview.locks =
            detect_locks_with_progress(Path::new(&preview.source_path), |current, total, _| {
                emit_progress(
                    &app,
                    current,
                    total,
                    if total == 0 {
                        "В источнике нет файлов для проверки"
                    } else {
                        "Проверяю блокировки перед запуском"
                    },
                );
            })
            .map_err(error_string)?;
    }
    if !preview.locks.is_empty() {
        return Err(format!(
            "Найдены занятые файлы: {}. Нажмите «Проверить», чтобы увидеть процессы.",
            preview.locks.len()
        ));
    }

    let operation = journal::create_operation(
        preview.source_path,
        preview.destination_path,
        preview.item_kind,
        request.strategy,
    )
    .map_err(error_string)?;
    launch_helper(HelperAction::Move, &operation).map_err(error_string)?;
    Ok(operation)
}

#[tauri::command]
pub fn cancel_operation(id: String) -> Result<bool, String> {
    let path = journal::cancel_file_path(&id).map_err(error_string)?;
    fs::write(path, b"cancel").map_err(error_string)?;
    Ok(true)
}

#[tauri::command]
pub fn list_operations() -> Result<Vec<OperationSnapshot>, String> {
    journal::list_operations().map_err(error_string)
}

#[tauri::command]
pub fn read_operation_log(id: String, offset: u64) -> Result<LogRead, String> {
    let operation = journal::get_operation(&id).map_err(error_string)?;
    let mut file = fs::File::open(&operation.log_path).map_err(error_string)?;
    file.seek(SeekFrom::Start(offset)).map_err(error_string)?;
    let mut buffer = String::new();
    file.read_to_string(&mut buffer).map_err(error_string)?;
    let next_offset = offset + buffer.as_bytes().len() as u64;
    Ok(LogRead {
        lines: buffer.lines().map(ToOwned::to_owned).collect(),
        next_offset,
    })
}

#[tauri::command]
pub fn rollback_operation(id: String) -> Result<OperationSnapshot, String> {
    let operation = journal::get_operation(&id).map_err(error_string)?;
    if operation.status != OperationStatus::Completed {
        return Err("Откат доступен только для завершенных операций.".to_string());
    }
    journal::update_status(&id, OperationStatus::RollingBack, None).map_err(error_string)?;
    let updated = journal::get_operation(&id).map_err(error_string)?;
    launch_helper(HelperAction::Rollback, &updated).map_err(error_string)?;
    Ok(updated)
}

fn launch_helper(action: HelperAction, operation: &OperationSnapshot) -> Result<()> {
    let request_path = journal::request_file_path(&operation.id)?;
    let cancel_path = journal::cancel_file_path(&operation.id)?;
    if cancel_path.exists() {
        fs::remove_file(&cancel_path).ok();
    }
    let invocation = HelperInvocation {
        action,
        operation: operation.clone(),
        cancel_path: cancel_path.to_string_lossy().to_string(),
    };
    journal::write_json_file(&request_path, &invocation)?;
    start_helper_elevated(&request_path)
        .with_context(|| "cannot start elevated operation helper")?;
    Ok(())
}

fn emit_progress(app: &tauri::AppHandle, current: usize, total: usize, label: &str) {
    let _ = app.emit(
        "preview-progress",
        ProgressSnapshot {
            current: current as u64,
            total: total as u64,
            label: label.to_string(),
        },
    );
}

fn error_string(error: impl std::fmt::Display) -> String {
    error.to_string()
}
