//! 用资源管理器打开文件或文件夹。

use std::path::Path;

use windows_sys::Win32::UI::Shell::ShellExecuteW;
use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

use super::win_util::to_wide;

pub unsafe fn open_path(path: &Path) {
    let op = to_wide("open");
    let file = to_wide(&path.to_string_lossy());
    let _ = ShellExecuteW(
        std::ptr::null_mut(),
        op.as_ptr(),
        file.as_ptr(),
        std::ptr::null(),
        std::ptr::null(),
        SW_SHOWNORMAL,
    );
}
