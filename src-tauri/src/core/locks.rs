use std::path::{Path, PathBuf};

use anyhow::Result;
use walkdir::WalkDir;

use super::models::FileLock;

const MAX_LOCK_SCAN_FILES: usize = 2_000;
const LOCK_BATCH_SIZE: usize = 64;

pub fn detect_locks(source: &Path) -> Result<Vec<FileLock>> {
    detect_locks_with_progress(source, |_, _, _| {})
}

pub fn detect_locks_with_progress<F>(source: &Path, mut on_progress: F) -> Result<Vec<FileLock>>
where
    F: FnMut(usize, usize, &Path),
{
    let paths = paths_to_probe(source);
    let total = paths.len();
    let mut locks = Vec::new();
    on_progress(0, total, source);

    for (chunk_index, chunk) in paths.chunks(LOCK_BATCH_SIZE).enumerate() {
        let batch_locks = detect_locks_for_path_batch(chunk)?;
        if !batch_locks.is_empty() {
            for path in chunk {
                locks.extend(detect_locks_for_single_path(path)?);
            }
        }

        let current = ((chunk_index + 1) * LOCK_BATCH_SIZE).min(total);
        on_progress(current, total, chunk.last().unwrap());
    }
    Ok(locks)
}

fn paths_to_probe(source: &Path) -> Vec<PathBuf> {
    if source.is_file() {
        return vec![source.to_path_buf()];
    }

    WalkDir::new(source)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_file())
        .take(MAX_LOCK_SCAN_FILES)
        .map(|entry| entry.path().to_path_buf())
        .collect()
}

#[cfg(windows)]
fn detect_locks_for_single_path(path: &Path) -> Result<Vec<FileLock>> {
    detect_locks_for_paths(&[path], Some(path))
}

#[cfg(windows)]
fn detect_locks_for_path_batch(paths: &[PathBuf]) -> Result<Vec<FileLock>> {
    let path_refs: Vec<&Path> = paths.iter().map(PathBuf::as_path).collect();
    detect_locks_for_paths(&path_refs, None)
}

#[cfg(windows)]
fn detect_locks_for_paths(paths: &[&Path], exact_path: Option<&Path>) -> Result<Vec<FileLock>> {
    use std::iter;
    use std::mem::zeroed;
    use std::os::windows::ffi::OsStrExt;

    use windows_sys::Win32::Foundation::ERROR_MORE_DATA;
    use windows_sys::Win32::System::RestartManager::{
        RmEndSession, RmGetList, RmRegisterResources, RmStartSession, CCH_RM_SESSION_KEY,
        RM_PROCESS_INFO,
    };

    if paths.is_empty() {
        return Ok(Vec::new());
    }

    let mut session = 0u32;
    let mut session_key = vec![0u16; CCH_RM_SESSION_KEY as usize + 1];
    let start_code = unsafe { RmStartSession(&mut session, 0, session_key.as_mut_ptr()) };
    if start_code != 0 {
        return Ok(Vec::new());
    }

    let result = (|| {
        let wide_paths: Vec<Vec<u16>> = paths
            .iter()
            .map(|path| {
                path.as_os_str()
                    .encode_wide()
                    .chain(iter::once(0))
                    .collect()
            })
            .collect();
        let resources: Vec<*const u16> = wide_paths.iter().map(|path| path.as_ptr()).collect();
        let register_code = unsafe {
            RmRegisterResources(
                session,
                resources.len() as u32,
                resources.as_ptr(),
                0,
                std::ptr::null(),
                0,
                std::ptr::null(),
            )
        };
        if register_code != 0 {
            return Ok(Vec::new());
        }

        let mut needed = 0u32;
        let mut count = 0u32;
        let mut reboot_reasons = 0u32;
        let first_code = unsafe {
            RmGetList(
                session,
                &mut needed,
                &mut count,
                std::ptr::null_mut(),
                &mut reboot_reasons,
            )
        };
        if first_code != ERROR_MORE_DATA {
            return Ok(Vec::new());
        }

        let mut processes: Vec<RM_PROCESS_INFO> =
            (0..needed).map(|_| unsafe { zeroed() }).collect();
        count = needed;
        let second_code = unsafe {
            RmGetList(
                session,
                &mut needed,
                &mut count,
                processes.as_mut_ptr(),
                &mut reboot_reasons,
            )
        };
        if second_code != 0 {
            return Ok(Vec::new());
        }

        Ok(processes
            .into_iter()
            .take(count as usize)
            .map(|process| FileLock {
                path: exact_path
                    .map(|path| path.to_string_lossy().to_string())
                    .unwrap_or_else(|| format!("Пачка из {} файлов", paths.len())),
                pid: process.Process.dwProcessId,
                process_name: wide_fixed_to_string(&process.strAppName),
                application_name: wide_fixed_to_string(&process.strServiceShortName),
            })
            .collect())
    })();

    unsafe {
        RmEndSession(session);
    }

    result
}

#[cfg(not(windows))]
fn detect_locks_for_single_path(_path: &Path) -> Result<Vec<FileLock>> {
    Ok(Vec::new())
}

#[cfg(not(windows))]
fn detect_locks_for_path_batch(_paths: &[PathBuf]) -> Result<Vec<FileLock>> {
    Ok(Vec::new())
}

#[cfg(windows)]
fn wide_fixed_to_string(value: &[u16]) -> String {
    let end = value
        .iter()
        .position(|item| *item == 0)
        .unwrap_or(value.len());
    String::from_utf16_lossy(&value[..end])
}
