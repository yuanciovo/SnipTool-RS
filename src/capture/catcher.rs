//! 选区上的「点击捕获窗口」。
//!
//! 待确认态里遮罩仍然挖着洞（这样选区内容才干净、不发灰）。代价是洞内像素
//! **不属于我们的窗口**，第一次按下的点击会直接穿透到下层程序 —— `SetCapture`
//! 是在收到按下之后才生效的，救不了这一下。
//!
//! 所以在选区正上方盖一个 `alpha = 1` 的分层窗口：视觉上几乎不可见（0.4%），
//! 但鼠标点击会落在它身上。它把消息换算成遮罩的客户区坐标后**同步转发**给遮罩，
//! 由遮罩那套统一的状态机处理。
//!
//! 用 SendMessage 而不是 PostMessage：同步转发能保证「按下」先于「抬起」被处理，
//! 否则快速点击时抬起会落到捕获窗口自己身上，遮罩就会卡在一个没有 up 的拖拽里。

use std::mem;
use std::ptr;

use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

use crate::platform::win_util::to_wide;

const CATCHER_CLASS: &str = "MySnipTool.Catcher";

struct Catcher {
    overlay: HWND,
    vx: i32,
    vy: i32,
}

pub unsafe fn create(overlay: HWND, vx: i32, vy: i32, sel_screen: &RECT) -> Option<HWND> {
    let hinstance = GetModuleHandleW(ptr::null());
    if hinstance.is_null() {
        return None;
    }

    let class_name = to_wide(CATCHER_CLASS);
    let mut wc: WNDCLASSW = mem::zeroed();
    wc.lpfnWndProc = Some(wnd_proc);
    wc.hInstance = hinstance;
    wc.hCursor = LoadCursorW(ptr::null_mut(), IDC_SIZEALL);
    wc.lpszClassName = class_name.as_ptr();
    RegisterClassW(&wc);

    let w = sel_screen.right - sel_screen.left;
    let h = sel_screen.bottom - sel_screen.top;
    if w <= 0 || h <= 0 {
        return None;
    }

    let empty = to_wide("");
    let hwnd = CreateWindowExW(
        WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_LAYERED,
        class_name.as_ptr(),
        empty.as_ptr(),
        WS_POPUP,
        sel_screen.left,
        sel_screen.top,
        w,
        h,
        ptr::null_mut(),
        ptr::null_mut(),
        hinstance,
        ptr::null(),
    );
    if hwnd.is_null() {
        return None;
    }

    let st = Box::new(Catcher { overlay, vx, vy });
    SetWindowLongPtrW(hwnd, GWLP_USERDATA, Box::into_raw(st) as isize);

    // alpha = 1/255 ≈ 0.4%，肉眼不可见，但仍然接收鼠标
    SetLayeredWindowAttributes(hwnd, 0, 1, LWA_ALPHA);
    ShowWindow(hwnd, SW_SHOWNOACTIVATE);

    Some(hwnd)
}

/// 选区移动/缩放时跟着走。
pub unsafe fn relocate(hwnd: HWND, sel_screen: &RECT) {
    SetWindowPos(
        hwnd,
        HWND_TOPMOST,
        sel_screen.left,
        sel_screen.top,
        sel_screen.right - sel_screen.left,
        sel_screen.bottom - sel_screen.top,
        SWP_NOACTIVATE,
    );
}

unsafe fn state<'a>(hwnd: HWND) -> Option<&'a mut Catcher> {
    (GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Catcher).as_mut()
}

unsafe extern "system" fn wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_LBUTTONDOWN | WM_MOUSEMOVE | WM_LBUTTONUP => {
            if let Some(s) = state(hwnd) {
                // 自己的客户区坐标 → 屏幕坐标 → 遮罩的客户区坐标
                let mut rc: RECT = mem::zeroed();
                GetWindowRect(hwnd, &mut rc);
                let cx = (lparam & 0xFFFF) as u16 as i16 as i32;
                let cy = ((lparam >> 16) & 0xFFFF) as u16 as i16 as i32;
                let ox = rc.left + cx - s.vx;
                let oy = rc.top + cy - s.vy;
                let lp = (((oy as isize) << 16) | (ox as isize & 0xFFFF)) as LPARAM;
                SendMessageW(s.overlay, msg, wparam, lp);
            }
            0
        }

        WM_SETCURSOR => {
            SetCursor(LoadCursorW(ptr::null_mut(), IDC_SIZEALL));
            1
        }

        WM_NCDESTROY => {
            let p = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Catcher;
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
            if !p.is_null() {
                drop(Box::from_raw(p));
            }
            0
        }

        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}
