//! 截图结果反馈：光标旁的迷你 HUD，替代系统托盘气泡。
//!
//! 为什么不用 Shell_NotifyIcon 气泡：
//!   1. 丑，且样式完全由系统决定；
//!   2. 依赖 Explorer 的通知设置 —— 开了「专注助手」直接不显示，反馈能不能到
//!      不该由系统说了算。
//!
//! 开销：+1 窗口 +1 画刷 +2 画笔 +1 字体 +1 region，全部在 destroy 时释放；
//! 不分配任何位图。工作集增量约 0.2MB，且是瞬态的。

use std::mem;
use std::ptr;

use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{
    BeginPaint, CreateFontW, CreatePen, CreateRoundRectRgn, CreateSolidBrush, DeleteObject,
    DrawTextW, EndPaint, FillRect, LineTo, MoveToEx, SelectObject, SetBkMode, SetTextColor,
    SetWindowRgn, UpdateWindow, CLEARTYPE_QUALITY, CLIP_DEFAULT_PRECIS, DEFAULT_CHARSET,
    DEFAULT_PITCH, DT_LEFT, DT_NOPREFIX, DT_SINGLELINE, FF_DONTCARE, FW_NORMAL, HBRUSH, HFONT,
    HPEN, OUT_DEFAULT_PRECIS, PAINTSTRUCT, PS_SOLID,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::HiDpi::GetDpiForSystem;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

use crate::platform::win_util::to_wide;

const HUD_CLASS: &str = "MySnipTool.Hud";
const TICK_MS: u32 = 50;
/// 第 18 tick（900ms）开始淡出，第 24 tick（1200ms）销毁。
const FADE_START: i32 = 18;
const DESTROY_AT: i32 = 24;
const BASE_W: i32 = 224;
const BASE_H: i32 = 56;

struct Hud {
    ok: bool,
    title: String,
    detail: String,
    tick: i32,
    alpha: u8,
    scale: i32,
    hbr_bg: HBRUSH,
    hpen_ok: HPEN,
    hpen_err: HPEN,
    hfont: HFONT,
    hfont_small: HFONT,
}

const fn rgb(r: u8, g: u8, b: u8) -> u32 {
    (r as u32) | ((g as u32) << 8) | ((b as u32) << 16)
}

unsafe fn state<'a>(hwnd: HWND) -> Option<&'a mut Hud> {
    (GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Hud).as_mut()
}

/// 在光标附近弹一个反馈窗。`ok=false` 会画红色叉。
pub unsafe fn show(ok: bool, title: &str, detail: &str) {
    let hinstance = GetModuleHandleW(ptr::null());
    if hinstance.is_null() {
        return;
    }

    let class_name = to_wide(HUD_CLASS);
    let mut wc: WNDCLASSW = mem::zeroed();
    wc.lpfnWndProc = Some(wnd_proc);
    wc.hInstance = hinstance;
    wc.lpszClassName = class_name.as_ptr();
    RegisterClassW(&wc); // 已注册过会返回 0，忽略

    // 跟随光标，右下偏移，超出则往回收
    let mut pt = POINT { x: 0, y: 0 };
    GetCursorPos(&mut pt);

    // GetDpiForWindow(GetDesktopWindow()) 在 200% 缩放下会返回 96，这里用系统 DPI
    let dpi = {
        let d = GetDpiForSystem();
        if d == 0 {
            96
        } else {
            d as i32
        }
    };
    let scale = (dpi as f32 / 96.0).round() as i32;
    let w = BASE_W * scale;
    let h = BASE_H * scale;

    let vx = GetSystemMetrics(SM_XVIRTUALSCREEN);
    let vy = GetSystemMetrics(SM_YVIRTUALSCREEN);
    let vw = GetSystemMetrics(SM_CXVIRTUALSCREEN);
    let vh = GetSystemMetrics(SM_CYVIRTUALSCREEN);

    let mut x = pt.x + 18 * scale;
    let mut y = pt.y + 24 * scale;
    if x + w > vx + vw {
        x = pt.x - w - 18 * scale;
    }
    if y + h > vy + vh {
        y = pt.y - h - 24 * scale;
    }
    if x < vx {
        x = vx;
    }
    if y < vy {
        y = vy;
    }

    let title_w = to_wide(title);
    let hwnd = CreateWindowExW(
        WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_LAYERED | WS_EX_NOACTIVATE,
        class_name.as_ptr(),
        title_w.as_ptr(),
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
        return;
    }

    let face = to_wide("Microsoft YaHei UI");
    let st = Box::new(Hud {
        ok,
        title: title.to_string(),
        detail: detail.to_string(),
        tick: 0,
        alpha: 245,
        scale,
        hbr_bg: CreateSolidBrush(rgb(28, 28, 30)),
        hpen_ok: CreatePen(PS_SOLID, 3 * scale, rgb(48, 209, 88)),
        hpen_err: CreatePen(PS_SOLID, 3 * scale, rgb(255, 69, 58)),
        hfont: CreateFontW(
            -15 * scale,
            0,
            0,
            0,
            FW_NORMAL as i32,
            0,
            0,
            0,
            DEFAULT_CHARSET as u32,
            OUT_DEFAULT_PRECIS as u32,
            CLIP_DEFAULT_PRECIS as u32,
            CLEARTYPE_QUALITY as u32,
            (DEFAULT_PITCH | FF_DONTCARE) as u32,
            face.as_ptr(),
        ),
        hfont_small: CreateFontW(
            -13 * scale,
            0,
            0,
            0,
            FW_NORMAL as i32,
            0,
            0,
            0,
            DEFAULT_CHARSET as u32,
            OUT_DEFAULT_PRECIS as u32,
            CLIP_DEFAULT_PRECIS as u32,
            CLEARTYPE_QUALITY as u32,
            (DEFAULT_PITCH | FF_DONTCARE) as u32,
            face.as_ptr(),
        ),
    });
    SetWindowLongPtrW(hwnd, GWLP_USERDATA, Box::into_raw(st) as isize);

    // 圆角：region 交给系统，窗口销毁时自动释放
    let rgn = CreateRoundRectRgn(0, 0, w + 1, h + 1, 12 * scale, 12 * scale);
    SetWindowRgn(hwnd, rgn, 0);

    SetLayeredWindowAttributes(hwnd, 0, 245, LWA_ALPHA);
    ShowWindow(hwnd, SW_SHOWNOACTIVATE);
    UpdateWindow(hwnd);
    SetTimer(hwnd, 1, TICK_MS, None);
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

    // 左侧图标：绿色勾 / 红色叉，纯线条，不需要位图
    let sc = s.scale;
    let bx = 18 * sc;
    let by = rc.bottom / 2 - 9 * sc;
    SelectObject(hdc, if s.ok { s.hpen_ok } else { s.hpen_err });
    if s.ok {
        MoveToEx(hdc, bx, by + 9 * sc, ptr::null_mut());
        LineTo(hdc, bx + 5 * sc, by + 15 * sc);
        LineTo(hdc, bx + 16 * sc, by + sc);
    } else {
        MoveToEx(hdc, bx, by, ptr::null_mut());
        LineTo(hdc, bx + 16 * sc, by + 16 * sc);
        MoveToEx(hdc, bx + 16 * sc, by, ptr::null_mut());
        LineTo(hdc, bx, by + 16 * sc);
    }

    let text_left = 46 * sc;
    SetBkMode(hdc, windows_sys::Win32::Graphics::Gdi::TRANSPARENT as i32);
    SetTextColor(hdc, rgb(245, 245, 247));
    SelectObject(hdc, s.hfont);
    let title = to_wide(&s.title);
    let mut r1 = RECT {
        left: text_left,
        top: 9 * sc,
        right: rc.right - 10 * sc,
        bottom: 30 * sc,
    };
    DrawTextW(
        hdc,
        title.as_ptr(),
        -1,
        &mut r1,
        DT_LEFT | DT_SINGLELINE | DT_NOPREFIX,
    );

    if !s.detail.is_empty() {
        SetTextColor(hdc, rgb(152, 152, 158));
        SelectObject(hdc, s.hfont_small);
        let detail = to_wide(&s.detail);
        let mut r2 = RECT {
            left: text_left,
            top: 29 * sc,
            right: rc.right - 10 * sc,
            bottom: 48 * sc,
        };
        DrawTextW(
            hdc,
            detail.as_ptr(),
            -1,
            &mut r2,
            DT_LEFT | DT_SINGLELINE | DT_NOPREFIX,
        );
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
        WM_TIMER => {
            if let Some(s) = state(hwnd) {
                s.tick += 1;
                if s.tick >= FADE_START {
                    let left = DESTROY_AT - s.tick;
                    if left <= 0 {
                        DestroyWindow(hwnd);
                    } else {
                        s.alpha = (245 / (DESTROY_AT - FADE_START) * left).clamp(0, 245) as u8;
                        SetLayeredWindowAttributes(hwnd, 0, s.alpha, LWA_ALPHA);
                    }
                }
            }
            0
        }
        WM_NCDESTROY => {
            KillTimer(hwnd, 1);
            let p = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Hud;
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
            if !p.is_null() {
                let b = Box::from_raw(p);
                DeleteObject(b.hbr_bg);
                DeleteObject(b.hpen_ok);
                DeleteObject(b.hpen_err);
                DeleteObject(b.hfont);
                DeleteObject(b.hfont_small);
                drop(b);
            }
            0
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}
