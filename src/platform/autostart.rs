//! 开机自启：往 `HKCU\Software\Microsoft\Windows\CurrentVersion\Run` 写一项。
//!
//! 用 HKCU 而不是 HKLM：不需要管理员权限，也不需要服务或计划任务。
//! 程序本身没有主界面，被拉起后直接进托盘，所以这里不需要额外参数。

use std::ptr;

use windows_sys::Win32::Foundation::ERROR_SUCCESS;
use windows_sys::Win32::System::LibraryLoader::GetModuleFileNameW;
use windows_sys::Win32::System::Registry::{
    RegCloseKey, RegDeleteValueW, RegOpenKeyExW, RegQueryValueExW, RegSetValueExW, HKEY,
    HKEY_CURRENT_USER, KEY_QUERY_VALUE, KEY_SET_VALUE, REG_SZ,
};

use super::win_util::to_wide;

const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const VALUE_NAME: &str = "MySnipTool";
/// 值不存在时 RegDeleteValueW 返回这个，不算失败。
const ERROR_FILE_NOT_FOUND: u32 = 2;

pub unsafe fn exe_path() -> Option<String> {
    let mut buf = [0u16; 1024];
    let n = GetModuleFileNameW(ptr::null_mut(), buf.as_mut_ptr(), buf.len() as u32);
    if n == 0 || n as usize >= buf.len() {
        return None;
    }
    Some(String::from_utf16_lossy(&buf[..n as usize]))
}

pub unsafe fn is_enabled() -> bool {
    let mut key: HKEY = ptr::null_mut();
    if RegOpenKeyExW(
        HKEY_CURRENT_USER,
        to_wide(RUN_KEY).as_ptr(),
        0,
        KEY_QUERY_VALUE,
        &mut key,
    ) != ERROR_SUCCESS
    {
        return false;
    }

    let mut buf = [0u16; 1024];
    let mut cb = (buf.len() * 2) as u32;
    let status = RegQueryValueExW(
        key,
        to_wide(VALUE_NAME).as_ptr(),
        ptr::null(),
        ptr::null_mut(),
        buf.as_mut_ptr() as *mut u8,
        &mut cb,
    );
    RegCloseKey(key);
    status == ERROR_SUCCESS
}

/// 打开/关闭自启。返回是否成功（失败时上层保持界面原状即可，不弹窗）。
pub unsafe fn set_enabled(on: bool) -> bool {
    let mut key: HKEY = ptr::null_mut();
    if RegOpenKeyExW(
        HKEY_CURRENT_USER,
        to_wide(RUN_KEY).as_ptr(),
        0,
        KEY_SET_VALUE,
        &mut key,
    ) != ERROR_SUCCESS
    {
        return false;
    }

    let ok = if on {
        match exe_path() {
            Some(path) => {
                // 加引号：路径里有空格也不会被系统截断
                let data = to_wide(&format!("\"{}\"", path));
                RegSetValueExW(
                    key,
                    to_wide(VALUE_NAME).as_ptr(),
                    0,
                    REG_SZ,
                    data.as_ptr() as *const u8,
                    (data.len() * 2) as u32,
                ) == ERROR_SUCCESS
            }
            None => false,
        }
    } else {
        let r = RegDeleteValueW(key, to_wide(VALUE_NAME).as_ptr());
        r == ERROR_SUCCESS || r == ERROR_FILE_NOT_FOUND
    };

    RegCloseKey(key);
    ok
}
