//! 冻结模式的静态底图窗口。
//!
//! **为什么必须是独立的不透明窗口**：
//! 遮罩是 `WS_EX_LAYERED` + `LWA_ALPHA=120` 的。任何画在遮罩上的像素都会被
//! 和它身后的实时桌面按 47% 混合一次 —— 底图贴上去就会发灰、有重影，视频还在动。
//!
//! 所以底图放在遮罩**下面**：遮罩用 `SetWindowRgn` 挖洞，洞内露出的是这个窗口的像素，
//! 不经过任何混合，绝对干净、绝对静止。
//!
//! 内存：只持有一个指向遮罩那侧全屏 DIB 的裸指针，自己零分配。

use std::mem;
use std::ptr;

use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{BeginPaint, BitBlt, EndPaint, PAINTSTRUCT, SRCCOPY};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

use crate::platform::gdi::Dib;
use crate::platform::win_util::to_wide;

const BG_CLASS: &str = "MySnipTool.FrozenBg";

struct Bg {
    /// 指向 Overlay 里那个全屏 DIB。生命周期由 Overlay 保证：
    /// Overlay 销毁时先 DestroyWindow(底图)，再释放 DIB。
    dib: *const Dib,
}

pub unsafe fn create(vx: i32, vy: i32, vw: i32, vh: i32) -> Option<HWND> {
    let hinstance = GetModuleHandleW(ptr::null());
    if hinstance.is_null() {
        return None;
    }

    let class_name = to_wide(BG_CLASS);
    let mut wc: WNDCLASSW = mem::zeroed();
    wc.lpfnWndProc = Some(wnd_proc);
    wc.hInstance = hinstance;
    wc.hbrBackground = ptr::null_mut();
    wc.lpszClassName = class_name.as_ptr();
    RegisterClassW(&wc);

    let empty = to_wide("");
    let hwnd = CreateWindowExW(
        WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
        class_name.as_ptr(),
        empty.as_ptr(),
        WS_POPUP,
        vx,
        vy,
        vw,
        vh,
        ptr::null_mut(),
        ptr::null_mut(),
        hinstance,
        ptr::null(),
    );
    if hwnd.is_null() {
        return None;
    }

    let st = Box::new(Bg {
        dib: ptr::null::<Dib>(),
    });
    SetWindowLongPtrW(hwnd, GWLP_USERDATA, Box::into_raw(st) as isize);

    Some(hwnd)
}

/// 底图 DIB 就绪后挂上去（遮罩状态先建好，指针才是稳定的）。
pub unsafe fn attach(hwnd: HWND, dib: *const Dib) {
    let p = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Bg;
    if let Some(b) = p.as_mut() {
        b.dib = dib;
    }
}

unsafe extern "system" fn wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_PAINT => {
            let mut ps: PAINTSTRUCT = mem::zeroed();
            let hdc = BeginPaint(hwnd, &mut ps);
            let p = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const Bg;
            if !hdc.is_null() && !p.is_null() {
                let b = &*p;
                if !b.dib.is_null() {
                    let d = &*b.dib;
                    BitBlt(hdc, 0, 0, d.w, d.h, d.dc(), 0, 0, SRCCOPY);
                }
            }
            EndPaint(hwnd, &ps);
            0
        }
        WM_ERASEBKGND => 1,
        WM_NCDESTROY => {
            let p = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Bg;
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
            if !p.is_null() {
                drop(Box::from_raw(p));
            }
            0
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}
