#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::thread;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use chrono::Local;
use robit_link_mover_lib::core::journal;
use robit_link_mover_lib::core::models::{
    HelperAction, HelperInvocation, ItemKind, MoveStrategy, OperationSnapshot, OperationStatus,
};
use robit_link_mover_lib::core::robocopy::{
    robocopy_exit_description, robocopy_exit_ok, robocopy_line_indicates_access_block,
};
use walkdir::WalkDir;

#[derive(Debug, Clone, Copy)]
struct Workload {
    files: u64,
    bytes: u64,
    entries: u64,
    copy_units: u64,
}

enum RobocopyEvent {
    Line(String),
    ReadError(String),
}

fn main() {
    if let Err(error) = run() {
        eprintln!("{error:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let request_path = parse_request_path()?;
    let data = fs::read_to_string(&request_path)
        .with_context(|| format!("cannot read request file: {}", request_path.display()))?;
    let invocation: HelperInvocation = serde_json::from_str(&data)?;

    let result = match invocation.action {
        HelperAction::Move => move_operation(&invocation),
        HelperAction::Rollback => rollback_operation(&invocation),
    };

    if let Err(error) = result {
        let message = format!("{error:#}");
        let _ = write_log(&invocation.operation, &format!("ERROR: {message}"));
        let _ = journal::update_status(
            &invocation.operation.id,
            OperationStatus::Failed,
            Some(&message),
        );
        return Err(error);
    }

    Ok(())
}

fn parse_request_path() -> Result<PathBuf> {
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--request" {
            if let Some(path) = args.next() {
                return Ok(PathBuf::from(path));
            }
        }
    }
    bail!("usage: robit-helper --request <path>");
}

fn move_operation(invocation: &HelperInvocation) -> Result<()> {
    let op = &invocation.operation;
    let source = Path::new(&op.source_path);
    let destination = Path::new(&op.destination_path);

    write_log(op, "Запущен перенос с повышенными правами")?;
    ensure_destination_parent(destination)?;
    let workload = measure_workload(source, op.item_kind)?;
    write_log(
        op,
        &format!(
            "Оценка объема: файлов {}, элементов {}, байт {}",
            workload.files, workload.entries, workload.bytes
        ),
    )?;
    let delete_units = delete_progress_units(op.item_kind, op.strategy, workload);
    let total_units = workload.copy_units + delete_units + 1;
    journal::update_progress(&op.id, 0, total_units, "Подготовка")?;

    journal::update_status(&op.id, OperationStatus::Copying, None)?;
    match (op.item_kind, op.strategy) {
        (ItemKind::Directory, MoveStrategy::SafeCopyDelete) => run_robocopy(
            source,
            destination,
            false,
            &invocation.cancel_path,
            op,
            workload,
            total_units,
        )?,
        (ItemKind::Directory, MoveStrategy::RobocopyMove) => run_robocopy(
            source,
            destination,
            true,
            &invocation.cancel_path,
            op,
            workload,
            total_units,
        )?,
        (ItemKind::File, _) => {
            copy_file(source, destination, op, workload.copy_units, total_units)?
        }
    }
    journal::update_progress(
        &op.id,
        workload.copy_units,
        total_units,
        "Копирование завершено",
    )?;

    if cancel_requested(&invocation.cancel_path) {
        write_log(op, "Отмена после копирования: удаляю частичное назначение")?;
        cleanup_destination(destination, op.item_kind)?;
        journal::update_status(&op.id, OperationStatus::Cancelled, None)?;
        return Ok(());
    }

    journal::update_status(&op.id, OperationStatus::Copied, None)?;

    if op.strategy == MoveStrategy::SafeCopyDelete || op.item_kind == ItemKind::File {
        journal::update_status(&op.id, OperationStatus::DeletingSource, None)?;
        delete_source(source, op.item_kind, op, workload.copy_units, total_units)?;
    } else {
        remove_empty_source_root(source, op.item_kind)?;
    }

    if cancel_requested(&invocation.cancel_path) {
        write_log(
            op,
            "Отмена перед созданием ссылки: восстанавливаю исходный путь",
        )?;
        restore_source_from_destination(destination, source, op.item_kind)?;
        journal::update_status(&op.id, OperationStatus::Cancelled, None)?;
        return Ok(());
    }

    journal::update_status(&op.id, OperationStatus::Linking, None)?;
    journal::update_progress(&op.id, total_units - 1, total_units, "Создание ссылки")?;
    create_link(source, destination, op.item_kind, op)?;
    journal::update_status(&op.id, OperationStatus::Completed, None)?;
    journal::update_progress(&op.id, total_units, total_units, "Готово")?;
    write_log(op, "Готово: старый путь теперь указывает на новое место")?;
    Ok(())
}

fn rollback_operation(invocation: &HelperInvocation) -> Result<()> {
    let op = &invocation.operation;
    let source = Path::new(&op.source_path);
    let destination = Path::new(&op.destination_path);

    write_log(op, "Запущен откат операции")?;
    journal::update_status(&op.id, OperationStatus::RollingBack, None)?;

    if !destination.exists() {
        bail!("destination no longer exists: {}", destination.display());
    }
    if source.exists() && !is_reparse_point(source) {
        bail!(
            "source path exists but is not a link created by this operation: {}",
            source.display()
        );
    }

    remove_link(source, op.item_kind)?;
    restore_source_from_destination(destination, source, op.item_kind)?;

    journal::update_status(&op.id, OperationStatus::RolledBack, None)?;
    write_log(op, "Откат выполнен")?;
    Ok(())
}

fn copy_file(
    source: &Path,
    destination: &Path,
    op: &OperationSnapshot,
    copy_units: u64,
    total_units: u64,
) -> Result<()> {
    ensure_destination_parent(destination)?;
    write_log(
        op,
        &format!(
            "Копирую файл: {} -> {}",
            source.display(),
            destination.display()
        ),
    )?;
    journal::update_progress(&op.id, 0, total_units, "Копирование файла")?;
    fs::copy(source, destination).with_context(|| {
        format!(
            "cannot copy file {} to {}",
            source.display(),
            destination.display()
        )
    })?;

    let source_len = fs::metadata(source)?.len();
    let destination_len = fs::metadata(destination)?.len();
    if source_len != destination_len {
        bail!(
            "file verification failed: source size {source_len}, destination size {destination_len}"
        );
    }
    journal::update_progress(&op.id, copy_units, total_units, "Файл скопирован")?;
    Ok(())
}

fn measure_workload(path: &Path, item_kind: ItemKind) -> Result<Workload> {
    match item_kind {
        ItemKind::File => {
            let bytes = fs::metadata(path)?.len();
            Ok(Workload {
                files: 1,
                bytes,
                entries: 1,
                copy_units: bytes.max(1),
            })
        }
        ItemKind::Directory => {
            let mut files = 0;
            let mut bytes = 0;
            let mut entries = 0;
            for entry in WalkDir::new(path)
                .min_depth(1)
                .into_iter()
                .filter_map(|entry| entry.ok())
            {
                entries += 1;
                if entry.file_type().is_file() {
                    files += 1;
                    bytes += entry.metadata().map(|metadata| metadata.len()).unwrap_or(0);
                }
            }
            Ok(Workload {
                files,
                bytes,
                entries,
                copy_units: if bytes > 0 { bytes } else { entries.max(1) },
            })
        }
    }
}

fn delete_progress_units(item_kind: ItemKind, strategy: MoveStrategy, workload: Workload) -> u64 {
    if strategy == MoveStrategy::SafeCopyDelete || item_kind == ItemKind::File {
        workload.entries.max(1)
    } else {
        0
    }
}

fn start_copy_progress_monitor(
    op: OperationSnapshot,
    destination: PathBuf,
    workload: Workload,
    total_units: u64,
    stop: Arc<AtomicBool>,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        let interval = if workload.entries > 10_000 { 5 } else { 2 };
        while !stop.load(Ordering::Relaxed) {
            thread::sleep(Duration::from_secs(interval));
            let copied = measure_copied_units(&destination, workload).min(workload.copy_units);
            let _ = journal::update_progress(&op.id, copied, total_units, "Копирование папки");
        }
    })
}

fn measure_copied_units(path: &Path, workload: Workload) -> u64 {
    if workload.bytes > 0 {
        measure_path_bytes(path).unwrap_or(0)
    } else {
        measure_path_entries(path).unwrap_or(0)
    }
}

fn measure_path_bytes(path: &Path) -> Result<u64> {
    if !path.exists() {
        return Ok(0);
    }
    if path.is_file() {
        return Ok(fs::metadata(path)?.len());
    }

    let mut bytes = 0;
    for entry in WalkDir::new(path)
        .min_depth(1)
        .into_iter()
        .filter_map(|entry| entry.ok())
    {
        if entry.file_type().is_file() {
            bytes += entry.metadata().map(|metadata| metadata.len()).unwrap_or(0);
        }
    }
    Ok(bytes)
}

fn measure_path_entries(path: &Path) -> Result<u64> {
    if !path.exists() {
        return Ok(0);
    }
    if path.is_file() {
        return Ok(1);
    }

    Ok(WalkDir::new(path)
        .min_depth(1)
        .into_iter()
        .filter_map(|entry| entry.ok())
        .count() as u64)
}

fn run_robocopy(
    source: &Path,
    destination: &Path,
    move_files: bool,
    cancel_path: &str,
    op: &OperationSnapshot,
    workload: Workload,
    total_units: u64,
) -> Result<()> {
    fs::create_dir_all(destination)?;
    let log_file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&op.log_path)?;
    let mut args = vec![
        windows_path_arg(source),
        windows_path_arg(destination),
        "/E".to_string(),
        "/COPY:DATSO".to_string(),
        "/R:3".to_string(),
        "/W:5".to_string(),
        "/NP".to_string(),
        "/NFL".to_string(),
        "/NDL".to_string(),
        "/NJH".to_string(),
        "/NJS".to_string(),
    ];
    if move_files {
        args.push("/MOVE".to_string());
    }

    write_log(op, &format!("Запускаю robocopy {}", args.join(" ")))?;
    journal::update_progress(&op.id, 0, total_units, "Копирование папки")?;
    let stop_progress = Arc::new(AtomicBool::new(false));
    let progress_thread = start_copy_progress_monitor(
        op.clone(),
        destination.to_path_buf(),
        workload,
        total_units,
        Arc::clone(&stop_progress),
    );
    let mut command = Command::new("robocopy");
    command
        .args(&args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    hide_child_window(&mut command);
    let mut child = command.spawn().context("cannot start robocopy")?;
    let stdout = child.stdout.take().context("cannot read robocopy stdout")?;
    let stderr = child.stderr.take().context("cannot read robocopy stderr")?;
    let (tx, rx) = mpsc::channel();
    let stdout_thread = spawn_robocopy_log_reader(stdout, tx.clone());
    let stderr_thread = spawn_robocopy_log_reader(stderr, tx);

    loop {
        if let Some(line) = drain_robocopy_events(&rx, &log_file, true)? {
            write_log(
                op,
                &format!("Обнаружена ошибка доступа robocopy: {line}. Останавливаю перенос."),
            )?;
            let _ = child.kill();
            let _ = child.wait();
            stop_progress.store(true, Ordering::Relaxed);
            let _ = progress_thread.join();
            let _ = stdout_thread.join();
            let _ = stderr_thread.join();
            let _ = drain_robocopy_events(&rx, &log_file, false);
            cleanup_interrupted_robocopy(source, destination, move_files, op)?;
            bail!("robocopy stopped after access error: {line}");
        }

        if cancel_requested(cancel_path) {
            write_log(op, "Получена отмена: останавливаю robocopy")?;
            let _ = child.kill();
            let _ = child.wait();
            stop_progress.store(true, Ordering::Relaxed);
            let _ = progress_thread.join();
            let _ = stdout_thread.join();
            let _ = stderr_thread.join();
            let _ = drain_robocopy_events(&rx, &log_file, false);
            cleanup_interrupted_robocopy(source, destination, move_files, op)?;
            journal::update_status(&op.id, OperationStatus::Cancelled, None)?;
            return Ok(());
        }

        if let Some(status) = child.try_wait()? {
            let code = status.code().unwrap_or(16);
            stop_progress.store(true, Ordering::Relaxed);
            let _ = progress_thread.join();
            let _ = stdout_thread.join();
            let _ = stderr_thread.join();
            if let Some(line) = drain_robocopy_events(&rx, &log_file, true)? {
                write_log(
                    op,
                    &format!("Обнаружена ошибка доступа robocopy: {line}. Выполняю откат."),
                )?;
                cleanup_interrupted_robocopy(source, destination, move_files, op)?;
                bail!("robocopy stopped after access error: {line}");
            }
            write_log(
                op,
                &format!(
                    "Robocopy завершился с кодом {code}: {}",
                    robocopy_exit_description(code)
                ),
            )?;
            if !robocopy_exit_ok(code) {
                cleanup_interrupted_robocopy(source, destination, move_files, op)?;
                bail!("robocopy failed with exit code {code}");
            }
            journal::update_progress(
                &op.id,
                workload.copy_units,
                total_units,
                "Копирование папки завершено",
            )?;
            return Ok(());
        }

        thread::sleep(Duration::from_millis(500));
    }
}

fn spawn_robocopy_log_reader<R>(reader: R, tx: Sender<RobocopyEvent>) -> thread::JoinHandle<()>
where
    R: Read + Send + 'static,
{
    thread::spawn(move || {
        let reader = BufReader::new(reader);
        for line in reader.lines() {
            match line {
                Ok(line) => {
                    if tx.send(RobocopyEvent::Line(line)).is_err() {
                        return;
                    }
                }
                Err(error) => {
                    let _ = tx.send(RobocopyEvent::ReadError(error.to_string()));
                    return;
                }
            }
        }
    })
}

fn drain_robocopy_events(
    rx: &Receiver<RobocopyEvent>,
    mut log_file: &fs::File,
    stop_on_access_block: bool,
) -> Result<Option<String>> {
    let mut access_block = None;
    while let Ok(event) = rx.try_recv() {
        match event {
            RobocopyEvent::Line(line) => {
                writeln!(log_file, "{line}")?;
                if stop_on_access_block
                    && access_block.is_none()
                    && robocopy_line_indicates_access_block(&line)
                {
                    access_block = Some(line);
                }
            }
            RobocopyEvent::ReadError(error) => {
                writeln!(log_file, "ERROR: cannot read robocopy output: {error}")?;
            }
        }
    }
    Ok(access_block)
}

fn cleanup_interrupted_robocopy(
    source: &Path,
    destination: &Path,
    move_files: bool,
    op: &OperationSnapshot,
) -> Result<()> {
    if move_files {
        if destination.exists() {
            write_log(
                op,
                "Возвращаю частично перенесенные файлы из назначения обратно в источник",
            )?;
            restore_source_from_destination(destination, source, ItemKind::Directory)?;
            write_log(op, "Частичный перенос возвращен в исходное место")?;
        }
    } else {
        cleanup_destination(destination, ItemKind::Directory)?;
    }
    Ok(())
}

fn delete_source(
    source: &Path,
    item_kind: ItemKind,
    op: &OperationSnapshot,
    copied_units: u64,
    total_units: u64,
) -> Result<()> {
    match item_kind {
        ItemKind::File => {
            fs::remove_file(source)
                .with_context(|| format!("cannot delete source file: {}", source.display()))?;
            journal::update_progress(
                &op.id,
                copied_units + 1,
                total_units,
                "Исходный файл удален",
            )?;
        }
        ItemKind::Directory => {
            delete_directory_with_progress(source, op, copied_units, total_units)?;
        }
    }
    Ok(())
}

fn delete_directory_with_progress(
    source: &Path,
    op: &OperationSnapshot,
    copied_units: u64,
    total_units: u64,
) -> Result<()> {
    let mut removed = 0;
    for entry in WalkDir::new(source)
        .min_depth(1)
        .contents_first(true)
        .into_iter()
        .filter_map(|entry| entry.ok())
    {
        let path = entry.path();
        if entry.file_type().is_dir() {
            fs::remove_dir(path)
                .with_context(|| format!("cannot delete source directory: {}", path.display()))?;
        } else {
            fs::remove_file(path)
                .with_context(|| format!("cannot delete source file: {}", path.display()))?;
        }
        removed += 1;
        if removed % 100 == 0 {
            journal::update_progress(
                &op.id,
                (copied_units + removed).min(total_units - 1),
                total_units,
                "Удаление исходника",
            )?;
        }
    }
    fs::remove_dir(source)
        .with_context(|| format!("cannot delete source directory: {}", source.display()))?;
    journal::update_progress(
        &op.id,
        total_units - 1,
        total_units,
        "Исходная папка удалена",
    )?;
    Ok(())
}

fn remove_empty_source_root(source: &Path, item_kind: ItemKind) -> Result<()> {
    if item_kind == ItemKind::Directory && source.exists() {
        let _ = fs::remove_dir(source);
    }
    Ok(())
}

fn cleanup_destination(destination: &Path, item_kind: ItemKind) -> Result<()> {
    if !destination.exists() {
        return Ok(());
    }
    match item_kind {
        ItemKind::File => {
            fs::remove_file(destination)?;
        }
        ItemKind::Directory => {
            fs::remove_dir_all(destination)?;
        }
    }
    Ok(())
}

fn restore_source_from_destination(
    destination: &Path,
    source: &Path,
    item_kind: ItemKind,
) -> Result<()> {
    ensure_destination_parent(source)?;
    match item_kind {
        ItemKind::File => {
            fs::rename(destination, source).or_else(|_| {
                fs::copy(destination, source)?;
                fs::remove_file(destination)
            })?;
        }
        ItemKind::Directory => {
            run_robocopy_for_restore(destination, source)?;
            let _ = fs::remove_dir(destination);
        }
    }
    Ok(())
}

fn run_robocopy_for_restore(destination: &Path, source: &Path) -> Result<()> {
    fs::create_dir_all(source)?;
    let mut command = Command::new("robocopy");
    command
        .arg(windows_path_arg(destination))
        .arg(windows_path_arg(source))
        .args([
            "/E",
            "/COPY:DATSO",
            "/MOVE",
            "/R:3",
            "/W:5",
            "/NP",
            "/NFL",
            "/NDL",
            "/NJH",
            "/NJS",
        ]);
    hide_child_window(&mut command);
    let status = command
        .status()
        .context("cannot start robocopy during rollback")?;
    let code = status.code().unwrap_or(16);
    if !robocopy_exit_ok(code) {
        bail!("rollback robocopy failed with exit code {code}");
    }
    Ok(())
}

fn create_link(
    source: &Path,
    destination: &Path,
    item_kind: ItemKind,
    op: &OperationSnapshot,
) -> Result<()> {
    match item_kind {
        ItemKind::File => {
            write_log(op, &format!("Создаю symlink файла: {}", source.display()))?;
            #[cfg(windows)]
            {
                std::os::windows::fs::symlink_file(destination, source)?;
            }
        }
        ItemKind::Directory => {
            write_log(op, &format!("Создаю junction папки: {}", source.display()))?;
            let mut command = Command::new("cmd");
            command
                .args(["/C", "mklink", "/J"])
                .arg(windows_path_arg(source))
                .arg(windows_path_arg(destination));
            hide_child_window(&mut command);
            let status = command.status().context("cannot start mklink")?;
            if !status.success() {
                bail!("mklink /J failed");
            }
        }
    }
    Ok(())
}

fn remove_link(source: &Path, item_kind: ItemKind) -> Result<()> {
    if !source.exists() {
        return Ok(());
    }
    if !is_reparse_point(source) {
        bail!("refusing to remove non-link path: {}", source.display());
    }
    match item_kind {
        ItemKind::File => fs::remove_file(source)?,
        ItemKind::Directory => {
            let mut command = Command::new("cmd");
            command.args(["/C", "rmdir"]).arg(windows_path_arg(source));
            hide_child_window(&mut command);
            let status = command.status().context("cannot remove junction")?;
            if !status.success() {
                bail!("cannot remove junction: {}", source.display());
            }
        }
    }
    Ok(())
}

fn ensure_destination_parent(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    Ok(())
}

fn cancel_requested(cancel_path: &str) -> bool {
    Path::new(cancel_path).exists()
}

fn write_log(op: &OperationSnapshot, message: &str) -> Result<()> {
    let now = Local::now().format("%Y-%m-%d %H:%M:%S");
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&op.log_path)?;
    writeln!(file, "[{now}] {message}")?;
    Ok(())
}

fn is_reparse_point(path: &Path) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        use windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT;

        fs::symlink_metadata(path)
            .map(|metadata| metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0)
            .unwrap_or(false)
    }

    #[cfg(not(windows))]
    {
        let _ = path;
        false
    }
}

fn windows_path_arg(path: &Path) -> String {
    let value = path.to_string_lossy();
    if let Some(rest) = value.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{rest}")
    } else if let Some(rest) = value.strip_prefix(r"\\?\") {
        rest.to_string()
    } else {
        value.to_string()
    }
}

fn hide_child_window(command: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        command.creation_flags(CREATE_NO_WINDOW);
    }

    #[cfg(not(windows))]
    {
        let _ = command;
    }
}
