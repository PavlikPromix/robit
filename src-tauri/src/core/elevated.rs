use std::ffi::OsStr;
use std::iter;
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{bail, Context, Result};
use windows_sys::Win32::Foundation::CloseHandle;
use windows_sys::Win32::Security::{
    GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY,
};
use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
use windows_sys::Win32::UI::Shell::{ShellExecuteExW, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW};
use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

pub fn start_helper_elevated(request_path: &Path) -> Result<()> {
    let helper = locate_helper()?;
    if !helper.exists() {
        bail!(
            "helper executable is missing: {}. Build the robit-helper binary first.",
            helper.display()
        );
    }

    if is_running_elevated() {
        let mut command = Command::new(helper);
        command.arg("--request").arg(request_path);
        hide_child_window(&mut command);
        command.spawn().context("cannot start helper process")?;
        return Ok(());
    }

    shell_execute_runas(
        &helper,
        Some(&format!("--request \"{}\"", request_path.display())),
    )?;
    Ok(())
}

pub fn restart_current_app_elevated() -> Result<()> {
    let current_exe = std::env::current_exe().context("cannot locate current executable")?;
    shell_execute_runas(&current_exe, None)
}

pub fn is_running_elevated() -> bool {
    let mut token = std::ptr::null_mut();
    let opened = unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) };
    if opened == 0 || token.is_null() {
        return false;
    }

    let mut elevation = TOKEN_ELEVATION { TokenIsElevated: 0 };
    let mut returned_size = 0u32;
    let ok = unsafe {
        GetTokenInformation(
            token,
            TokenElevation,
            &mut elevation as *mut _ as *mut _,
            std::mem::size_of::<TOKEN_ELEVATION>() as u32,
            &mut returned_size,
        )
    };
    unsafe {
        CloseHandle(token);
    }

    ok != 0 && elevation.TokenIsElevated != 0
}

fn shell_execute_runas(file_path: &Path, params_text: Option<&str>) -> Result<()> {
    let verb = wide("runas");
    let file = wide_os(file_path.as_os_str());
    let params = params_text.map(wide);
    let params_ptr = params
        .as_ref()
        .map(|value| value.as_ptr())
        .unwrap_or(std::ptr::null());

    let mut execute_info = SHELLEXECUTEINFOW {
        cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOCLOSEPROCESS,
        hwnd: std::ptr::null_mut(),
        lpVerb: verb.as_ptr(),
        lpFile: file.as_ptr(),
        lpParameters: params_ptr,
        lpDirectory: std::ptr::null(),
        nShow: SW_SHOWNORMAL,
        hInstApp: std::ptr::null_mut(),
        lpIDList: std::ptr::null_mut(),
        lpClass: std::ptr::null(),
        hkeyClass: std::ptr::null_mut(),
        dwHotKey: 0,
        Anonymous: unsafe { std::mem::zeroed() },
        hProcess: std::ptr::null_mut(),
    };

    let ok = unsafe { ShellExecuteExW(&mut execute_info) };
    if ok == 0 {
        bail!("UAC helper launch was cancelled or failed");
    }

    if !execute_info.hProcess.is_null() {
        unsafe {
            CloseHandle(execute_info.hProcess);
        }
    }

    Ok(())
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

fn locate_helper() -> Result<PathBuf> {
    let current_exe = std::env::current_exe().context("cannot locate current executable")?;
    let same_dir = current_exe.with_file_name("robit-helper.exe");
    if same_dir.exists() {
        return Ok(same_dir);
    }

    let debug_target = current_exe
        .parent()
        .and_then(|path| path.parent())
        .map(|target| target.join("robit-helper.exe"));
    if let Some(path) = debug_target {
        if path.exists() {
            return Ok(path);
        }
    }

    Ok(same_dir)
}

fn wide(value: &str) -> Vec<u16> {
    OsStr::new(value)
        .encode_wide()
        .chain(iter::once(0))
        .collect()
}

fn wide_os(value: &OsStr) -> Vec<u16> {
    value.encode_wide().chain(iter::once(0)).collect()
}
