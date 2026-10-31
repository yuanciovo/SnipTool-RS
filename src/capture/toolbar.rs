//! 框选后的「确认 / 取消」工具栏（微信截图那种）。
//!
//! 为什么必须是**独立窗口**而不是遮罩的子窗口：
//! 遮罩是 WS_EX_LAYERED + LWA_ALPHA=120 的，子窗口会被父窗口的 alpha 一起
//! 合成成半透明，按钮看起来像蒙了层灰。独立顶层窗口才能不透明。
//!
//! 点击后通过 PostMessage 把结果丢回遮罩窗口，两个窗口之间不共享状态。

use std::mem;
use std::ptr;

use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{
    BeginPaint, CreatePen, CreateRoundRectRgn, CreateSolidBrush, DeleteObject, EndPaint, FillRect,
    InvalidateRect, LineTo, MoveToEx, SelectObject, SetWindowRgn, UpdateWindow, HBRUSH, HPEN,
    PAINTSTRUCT, PS_SOLID,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::HiDpi::GetDpiForWindow;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    TrackMouseEvent, TME_LEAVE, TRACKMOUSEEVENT,
};
use windows_sys::Win32::UI::WindowsAndMessaging::*;

use crate::platform::win_util::to_wide;

const TOOLBAR_CLASS: &str = "MySnipTool.Toolbar";

/// windows-sys 把 WM_MOUSELEAVE 归在 Win32::UI::Controls 下。为了一个常量
/// 不值得开整个「公共控件」feature（拖进来上千个绑定），直接写值：ABI 固定。
const WM_MOUSELEAVE: u32 = 0x02A3;

const BASE_BTN: i32 = 36;
const BASE_PAD: i32 = 4;

pub const ID_CONFIRM: usize = 0;
pub const ID_CANCEL: usize = 1;

struct Bar {
    overlay: HWND,
    hover: Option<usize>,
    scale: i32,
    btn: i32,
    pad: i32,
    hbr_bg: HBRUSH,
    hbr_hover: HBRUSH,
    hpen_ok: HPEN,
    hpen_cancel: HPEN,
}

const fn rgb(r: u8, g: u8, b: u8) -> u32 {
    (r as u32) | ((g as u32) << 8) | ((b as u32) << 16)
}

unsafe fn state<'a>(hwnd: HWND) -> Option<&'a mut Bar> {
    (GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Bar).as_mut()
}

/// 算工具栏位置：默认贴选区右下角外侧，下方放不下就翻到上方，再夹到屏幕内。
unsafe fn place(w: i32, h: i32, sel: &RECT, margin: i32) -> (i32, i32) {
    let vx = GetSystemMetrics(SM_XVIRTUALSCREEN);
    let vy = GetSystemMetrics(SM_YVIRTUALSCREEN);
    let vw = GetSystemMetrics(SM_CXVIRTUALSCREEN);
    let vh = GetSystemMetrics(SM_CYVIRTUALSCREEN);

    let mut x = sel.right - w;
    let mut y = sel.bottom + margin;
    if y + h > vy + vh {
        y = sel.bottom - h - margin;
    }
    if y < vy {
        y = sel.bottom + margin;
    }
    if x < vx {
        x = vx;
    }
    if x + w > vx + vw {
        x = vx + vw - w;
    }
    (x, y)
}

/// 选区移动/缩放时跟着走。
pub unsafe fn reposition(hwnd: HWND, sel: &RECT) {
    let mut rc: RECT = mem::zeroed();
    GetWindowRect(hwnd, &mut rc);
    let w = rc.right - rc.left;
    let h = rc.bottom - rc.top;
    let scale = match state(hwnd) {
        Some(s) => s.scale,
        None => 1,
    };
    let (x, y) = place(w, h, sel, 10 * scale);
    SetWindowPos(hwnd, HWND_TOPMOST, x, y, w, h, SWP_NOACTIVATE);
}

/// 在选区右下方显示工具栏。放不下就翻到选区上方。
pub unsafe fn open(overlay: HWND, sel: &RECT) -> Option<HWND> {
    let hinstance = GetModuleHandleW(ptr::null());
    if hinstance.is_null() {
        return None;
    }

    let class_name = to_wide(TOOLBAR_CLASS);
    let mut wc: WNDCLASSW = mem::zeroed();
    wc.lpfnWndProc = Some(wnd_proc);
    wc.hInstance = hinstance;
    wc.lpszClassName = class_name.as_ptr();
    RegisterClassW(&wc);

    // 注意必须问 overlay 窗口的 DPI：GetDpiForWindow(GetDesktopWindow()) 返回 96，
    // 在 200% 缩放下会把工具栏画成一半大小。
    let dpi = {
        let d = GetDpiForWindow(overlay);
        if d == 0 {
            96
        } else {
            d as i32
        }
    };
    let scale = (dpi as f32 / 96.0).round() as i32;
    let pad = BASE_PAD * scale;
    let btn = BASE_BTN * scale;
    let gap = 2 * scale;
    let w = pad * 2 + btn * 2 + gap;
    let h = pad * 2 + btn;

    let (x, y) = place(w, h, sel, 10 * scale);

    let empty = to_wide("");
    let hwnd = CreateWindowExW(
        WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
        class_name.as_ptr(),
        empty.as_ptr(),
        WS_POPUP,
        x,
        y,
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

    let st = Box::new(Bar {
        overlay,
        hover: None,
        scale,
        btn,
        pad,
        hbr_bg: CreateSolidBrush(rgb(44, 44, 42)),
        hbr_hover: CreateSolidBrush(rgb(70, 70, 68)),
        hpen_ok: CreatePen(PS_SOLID, 3 * scale, rgb(48, 209, 88)),
        hpen_cancel: CreatePen(PS_SOLID, 3 * scale, rgb(208, 208, 212)),
    });
    SetWindowLongPtrW(hwnd, GWLP_USERDATA, Box::into_raw(st) as isize);

    let rgn = CreateRoundRectRgn(0, 0, w + 1, h + 1, 10 * scale, 10 * scale);
    SetWindowRgn(hwnd, rgn, 0);

    ShowWindow(hwnd, SW_SHOWNOACTIVATE);
    SetWindowPos(
        hwnd,
        HWND_TOPMOST,
        x,
        y,
        w,
        h,
        SWP_NOACTIVATE | SWP_SHOWWINDOW,
    );
    UpdateWindow(hwnd);

    Some(hwnd)
}

fn btn_rect(s: &Bar, i: usize) -> RECT {
    let gap = 2 * s.scale;
    let left = s.pad + i as i32 * (s.btn + gap);
    RECT {
        left,
        top: s.pad,
        right: left + s.btn,
        bottom: s.pad + s.btn,
    }
}

fn hit(s: &Bar, x: i32, y: i32) -> Option<usize> {
    for i in 0..2 {
        let r = btn_rect(s, i);
        if x >= r.left && x < r.right && y >= r.top && y < r.bottom {
            return Some(i);
        }
    }
    None
}

unsafe fn paint(hwnd: HWND) {
    let mut ps: PAINTSTRUCT = mem::zeroed();
    let hdc = BeginPaint(hwnd, &mut ps);
    if hdc.is_null() {
        EndPaint(hwnd, &ps);
        return;
    }
    let Some(s) = state(hwnd) else {
        EndPaint(hwnd, &ps);
        return;
    };

    let mut rc: RECT = mem::zeroed();
    GetClientRect(hwnd, &mut rc);
    FillRect(hdc, &rc, s.hbr_bg);

    for i in 0..2 {
        if s.hover == Some(i) {
            let r = btn_rect(s, i);
            FillRect(hdc, &r, s.hbr_hover);
        }
        let r = btn_rect(s, i);
        let cx = (r.left + r.right) / 2;
        let cy = (r.top + r.bottom) / 2;
        let a = 8 * s.scale;

        if i == ID_CONFIRM {
            SelectObject(hdc, s.hpen_ok);
            MoveToEx(hdc, cx - a + 2 * s.scale, cy, ptr::null_mut());
            LineTo(hdc, cx - 2 * s.scale, cy + a - 2 * s.scale);
            LineTo(hdc, cx + a, cy - a + 2 * s.scale);
        } else {
            SelectObject(hdc, s.hpen_cancel);
            MoveToEx(
                hdc,
                cx - a + 2 * s.scale,
                cy - a + 2 * s.scale,
                ptr::null_mut(),
            );
            LineTo(hdc, cx + a - 2 * s.scale, cy + a - 2 * s.scale);
            MoveToEx(
                hdc,
                cx + a - 2 * s.scale,
                cy - a + 2 * s.scale,
                ptr::null_mut(),
            );
            LineTo(hdc, cx - a + 2 * s.scale, cy + a - 2 * s.scale);
        }
    }

    EndPaint(hwnd, &ps);
}

unsafe extern "system" fn wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_PAINT => {
            paint(hwnd);
            0
        }
        WM_ERASEBKGND => 1,

        WM_MOUSEMOVE => {
            if let Some(s) = state(hwnd) {
                let x = (lparam & 0xFFFF) as u16 as i16 as i32;
                let y = ((lparam >> 16) & 0xFFFF) as u16 as i16 as i32;
                let h = hit(s, x, y);
                if h != s.hover {
                    s.hover = h;
                    InvalidateRect(hwnd, ptr::null(), 0);
                }
                if s.hover.is_some() {
                    let mut tme: TRACKMOUSEEVENT = mem::zeroed();
                    tme.cbSize = mem::size_of::<TRACKMOUSEEVENT>() as u32;
                    tme.dwFlags = TME_LEAVE;
                    tme.hwndTrack = hwnd;
                    TrackMouseEvent(&mut tme);
                }
            }
            0
        }

        WM_MOUSELEAVE => {
            if let Some(s) = state(hwnd) {
                s.hover = None;
                InvalidateRect(hwnd, ptr::null(), 0);
            }
            0
        }

        WM_LBUTTONDOWN => {
            if let Some(s) = state(hwnd) {
                let x = (lparam & 0xFFFF) as u16 as i16 as i32;
                let y = ((lparam >> 16) & 0xFFFF) as u16 as i16 as i32;
                if let Some(i) = hit(s, x, y) {
                    // 交给遮罩窗口处理，工具栏自己不做决定
                    PostMessageW(s.overlay, crate::capture::overlay::WM_TOOLBAR, i, 0);
                }
            }
            0
        }

        WM_NCDESTROY => {
            let p = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Bar;
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
            if !p.is_null() {
                let b = Box::from_raw(p);
                DeleteObject(b.hbr_bg);
                DeleteObject(b.hbr_hover);
                DeleteObject(b.hpen_ok);
                DeleteObject(b.hpen_cancel);
                drop(b);
            }
            0
        }

        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}
