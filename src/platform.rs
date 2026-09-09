use std::path::Path;

#[cfg(windows)]
use windows::Win32::UI::Shell::ShellExecuteW;
#[cfg(windows)]
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
#[cfg(windows)]
use windows::core::PCWSTR;

/// Windows 標準の ShellExecuteW を使ってファイル・アプリを安全に開く
#[cfg(windows)]
pub fn shell_open(path: &str, working_dir: Option<&Path>) -> Result<(), String> {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;

    let path_wide: Vec<u16> = OsStr::new(path)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let op_wide: Vec<u16> = OsStr::new("open")
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let dir_wide: Option<Vec<u16>> = working_dir.map(|d| {
        d.as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect()
    });

    let res = unsafe {
        ShellExecuteW(
            None,
            PCWSTR(op_wide.as_ptr()),
            PCWSTR(path_wide.as_ptr()),
            PCWSTR::null(),
            dir_wide
                .as_ref()
                .map(|d| PCWSTR(d.as_ptr()))
                .unwrap_or(PCWSTR::null()),
            SW_SHOWNORMAL,
        )
    };

    if (res.0 as usize) > 32 {
        Ok(())
    } else {
        Err(format!(
            "起動に失敗しました (エラーコード: {})",
            res.0 as usize
        ))
    }
}

#[cfg(not(windows))]
pub fn shell_open(path: &str, _working_dir: Option<&Path>) -> Result<(), String> {
    std::process::Command::new("xdg-open")
        .arg(path)
        .spawn()
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// 重複比較用の正規化キー（大文字小文字・スラッシュを統一）
pub fn normalize_path_key(path_str: &str) -> String {
    Path::new(path_str)
        .canonicalize()
        .map(|p| {
            let s = p.to_string_lossy().to_string();
            s.strip_prefix(r"\\?\").unwrap_or(&s).to_lowercase()
        })
        .unwrap_or_else(|_| path_str.replace('/', "\\").to_lowercase())
}
