//! Win32 通用小工具：字符串转换、错误框。

use std::ptr;

use windows_sys::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_ICONERROR, MB_OK};

/// UTF-8 → 以 NUL 结尾的 UTF-16，供所有 *W API 使用。
pub fn to_wide(s: &str) -> Vec<u16> {
    let mut v: Vec<u16> = s.encode_utf16().collect();
    v.push(0);
    v
}

pub unsafe fn error_box(title: &str, body: &str) {
    let t = to_wide(body);
    let c = to_wide(title);
    MessageBoxW(
        ptr::null_mut(),
        t.as_ptr(),
        c.as_ptr(),
        MB_OK | MB_ICONERROR,
    );
}
