//! 历史截图面板（默认 Alt+V，可在设置里改绑）。
//!
//! 内存纪律（技术方案 §4）：
//!   - 列表只读文件名 + PNG 头（`image_dimensions`），**不解码原图**；
//!   - 只加载 10 张 256×144 的缩略图（合计约 1.5MB），面板关闭即全部释放；
//!   - 点击某一行才去解码原图，写完剪贴板立刻丢弃。
//!
//! 缩略图缺失时（例如 Spike 3 时代留下的老文件）会在打开面板时补生成一次，
//! 只补缺的那几张，之后就都是缓存命中。

use std::mem;
use std::path::PathBuf;
use std::ptr;

use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{
    BeginPaint, BitBlt, CreateFontW, CreateRoundRectRgn, CreateSolidBrush, DeleteObject, DrawTextW,
    EndPaint, FillRect, InvalidateRect, SelectObject, SetBkMode, SetTextColor, SetWindowRgn,
    UpdateWindow, CLEARTYPE_QUALITY, CLIP_DEFAULT_PRECIS, DEFAULT_CHARSET, DEFAULT_PITCH, DT_LEFT,
    DT_NOPREFIX, DT_SINGLELINE, DT_VCENTER, FF_DONTCARE, FW_NORMAL, HBRUSH, HFONT,
    OUT_DEFAULT_PRECIS, PAINTSTRUCT, SRCCOPY, TRANSPARENT,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::HiDpi::GetDpiForSystem;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    SetFocus, VK_DOWN, VK_ESCAPE, VK_RETURN, VK_UP,
};
use windows_sys::Win32::UI::WindowsAndMessaging::*;

use crate::platform::clipboard;
use crate::platform::gdi::Dib;
use crate::platform::win_util::to_wide;
use crate::store::{self, Entry};

const PANEL_CLASS: &str = "MySnipTool.History";
const MAX_ITEMS: usize = 10;
const BASE_W: i32 = 380;
const BASE_ROW: i32 = 64;
const BASE_PAD: i32 = 8;
const BASE_THUMB_W: i32 = 96;
const BASE_THUMB_H: i32 = 54;

struct Item {
    entry: Entry,
    thumb: Option<Dib>,
}

struct Panel {
    items: Vec<Item>,
    sel: Option<usize>,
    hover: Option<usize>,
    scale: i32,
    row_h: i32,
    pad: i32,
    hbr_bg: HBRUSH,
    hbr_hover: HBRUSH,
    hbr_sel: HBRUSH,
    hfont: HFONT,
    hfont_small: HFONT,
    /// 关闭后用它弹 HUD
    ok_text: String,
}

const fn rgb(r: u8, g: u8, b: u8) -> u32 {
    (r as u32) | ((g as u32) << 8) | ((b as u32) << 16)
}

unsafe fn state<'a>(hwnd: HWND) -> Option<&'a mut Panel> {
    (GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Panel).as_mut()
}

/// 缩略图解码失败就现生成一张（只补缺失的）。
unsafe fn load_thumb(e: &Entry) -> Option<Dib> {
    if !e.thumb.exists() {
        let img = image::open(&e.path).ok()?;
        let rgb = img.to_rgb8();
        let (w, h) = (rgb.width() as i32, rgb.height() as i32);
        let src = Dib::from_rgb(rgb.as_raw(), w, h)?;
        let stem = e.name.trim_end_matches(".png");
        store::save_thumb(&src, stem);
    }
    let img = image::open(&e.thumb).ok()?;
    let rgb = img.to_rgb8();
    let (w, h) = (rgb.width() as i32, rgb.height() as i32);
    Dib::from_rgb(rgb.as_raw(), w, h)
}

/// 打开面板。返回窗口句柄，失败返回 None（列表为空也算成功，会显示空态）。
pub unsafe fn open() -> Option<HWND> {
    let hinstance = GetModuleHandleW(ptr::null());
    if hinstance.is_null() {
        return None;
    }

    let entries = store::list_recent(MAX_ITEMS);
    let mut items = Vec::with_capacity(entries.len());
    for e in entries {
        let thumb = load_thumb(&e);
        items.push(Item { entry: e, thumb });
    }

    let dpi = {
        let d = GetDpiForSystem();
        if d == 0 {
            96
        } else {
            d as i32
        }
    };
    let scale = (dpi as f32 / 96.0).round() as i32;
    let pad = BASE_PAD * scale;
    let row_h = BASE_ROW * scale;
    let w = BASE_W * scale;
    let rows = items.len().max(1) as i32;
    let h = pad * 2 + row_h * rows;

    // 跟随光标，别贴着屏幕边
    let mut pt = POINT { x: 0, y: 0 };
    GetCursorPos(&mut pt);
    let vx = GetSystemMetrics(SM_XVIRTUALSCREEN);
    let vy = GetSystemMetrics(SM_YVIRTUALSCREEN);
    let vw = GetSystemMetrics(SM_CXVIRTUALSCREEN);
    let vh = GetSystemMetrics(SM_CYVIRTUALSCREEN);
    let mut x = pt.x - w / 2;
    let mut y = pt.y - h / 2;
    if x < vx + pad {
        x = vx + pad;
    }
    if y < vy + pad {
        y = vy + pad;
    }
    if x + w > vx + vw - pad {
        x = vx + vw - w - pad;
    }
    if y + h > vy + vh - pad {
        y = vy + vh - h - pad;
    }

    let class_name = to_wide(PANEL_CLASS);
    let title = to_wide("MySnipTool History");
    let face = to_wide("Microsoft YaHei UI");

    let mut wc: WNDCLASSW = mem::zeroed();
    wc.lpfnWndProc = Some(wnd_proc);
    wc.hInstance = hinstance;
    wc.hCursor = LoadCursorW(ptr::null_mut(), IDC_ARROW);
    wc.lpszClassName = class_name.as_ptr();
    RegisterClassW(&wc);

    let hwnd = CreateWindowExW(
        WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
        class_name.as_ptr(),
        title.as_ptr(),
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

    let st = Box::new(Panel {
        sel: if items.is_empty() { None } else { Some(0) },
        items,
        hover: None,
        scale,
        row_h,
        pad,
        hbr_bg: CreateSolidBrush(rgb(28, 28, 30)),
        hbr_hover: CreateSolidBrush(rgb(48, 48, 50)),
        hbr_sel: CreateSolidBrush(rgb(0, 92, 68)),
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
        ok_text: String::new(),
    });
    SetWindowLongPtrW(hwnd, GWLP_USERDATA, Box::into_raw(st) as isize);

    let rgn = CreateRoundRectRgn(0, 0, w + 1, h + 1, 12 * scale, 12 * scale);
    SetWindowRgn(hwnd, rgn, 0);

    ShowWindow(hwnd, SW_SHOW);
    SetForegroundWindow(hwnd);
    SetFocus(hwnd);
    UpdateWindow(hwnd);

    Some(hwnd)
}

fn row_rect(s: &Panel, i: usize) -> RECT {
    let top = s.pad + i as i32 * s.row_h;
    RECT {
        left: s.pad,
        top,
        right: BASE_W * s.scale - s.pad,
        bottom: top + s.row_h,
    }
}

/// 缩略图按等比塞进 thumb 框并居中，避免拉伸变形。
fn thumb_dst(s: &Panel, row: &RECT, tw: i32, th: i32) -> RECT {
    let bw = BASE_THUMB_W * s.scale;
    let bh = BASE_THUMB_H * s.scale;
    let k = (bw as f32 / tw.max(1) as f32).min(bh as f32 / th.max(1) as f32);
    let dw = (tw as f32 * k).round().max(1.0) as i32;
    let dh = (th as f32 * k).round().max(1.0) as i32;
    let cx = row.left + s.pad + bw / 2;
    let cy = (row.top + row.bottom) / 2;
    RECT {
        left: cx - dw / 2,
        top: cy - dh / 2,
        right: cx + dw / 2,
        bottom: cy + dh / 2,
    }
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

    SetBkMode(hdc, TRANSPARENT as i32);

    if s.items.is_empty() {
        SetTextColor(hdc, rgb(150, 150, 156));
        SelectObject(hdc, s.hfont);
        let t = to_wide("还没有截图");
        let mut r = RECT {
            left: s.pad * 3,
            top: 0,
            right: rc.right,
            bottom: rc.bottom,
        };
        DrawTextW(
            hdc,
            t.as_ptr(),
            -1,
            &mut r,
            DT_LEFT | DT_SINGLELINE | DT_NOPREFIX | DT_VCENTER,
        );
        EndPaint(hwnd, &ps);
        return;
    }

    for i in 0..s.items.len() {
        let row = row_rect(s, i);
        if s.sel == Some(i) {
            FillRect(hdc, &row, s.hbr_sel);
        } else if s.hover == Some(i) {
            FillRect(hdc, &row, s.hbr_hover);
        }

        let it = &s.items[i];
        if let Some(t) = &it.thumb {
            let dst = thumb_dst(s, &row, t.w, t.h);
            BitBlt(
                hdc,
                dst.left,
                dst.top,
                dst.right - dst.left,
                dst.bottom - dst.top,
                t.dc(),
                0,
                0,
                SRCCOPY,
            );
        }

        let text_left = row.left + s.pad + BASE_THUMB_W * s.scale + 12 * s.scale;

        SetTextColor(hdc, rgb(240, 240, 242));
        SelectObject(hdc, s.hfont);
        let t1 = to_wide(&it.entry.time_label());
        let mut r1 = RECT {
            left: text_left,
            top: row.top + 10 * s.scale,
            right: row.right,
            bottom: row.top + 34 * s.scale,
        };
        DrawTextW(
            hdc,
            t1.as_ptr(),
            -1,
            &mut r1,
            DT_LEFT | DT_SINGLELINE | DT_NOPREFIX,
        );

        SetTextColor(hdc, rgb(150, 150, 156));
        SelectObject(hdc, s.hfont_small);
        let t2 = to_wide(&format!(
            "{} × {}   ·   {} KB",
            it.entry.w, it.entry.h, it.entry.kb
        ));
        let mut r2 = RECT {
            left: text_left,
            top: row.top + 34 * s.scale,
            right: row.right,
            bottom: row.bottom - 6 * s.scale,
        };
        DrawTextW(
            hdc,
            t2.as_ptr(),
            -1,
            &mut r2,
            DT_LEFT | DT_SINGLELINE | DT_NOPREFIX,
        );
    }

    EndPaint(hwnd, &ps);
}

fn hit_row(s: &Panel, y: i32) -> Option<usize> {
    if s.items.is_empty() {
        return None;
    }
    let rel = y - s.pad;
    if rel < 0 {
        return None;
    }
    let i = (rel / s.row_h) as usize;
    if i < s.items.len() {
        Some(i)
    } else {
        None
    }
}

/// 选中某项：解码原图 → 写剪贴板 → 关面板。
unsafe fn activate(hwnd: HWND) {
    let Some(s) = state(hwnd) else { return };
    let Some(i) = s.sel else { return };
    let Some(it) = s.items.get(i) else { return };
    let path: PathBuf = it.entry.path.clone();
    let (w, h) = (it.entry.w as i32, it.entry.h as i32);

    let Ok(img) = image::open(&path) else {
        crate::toast_err("打开失败", "文件可能已被清理");
        DestroyWindow(hwnd);
        return;
    };
    let rgbimg = img.to_rgb8();
    let (iw, ih) = (rgbimg.width() as i32, rgbimg.height() as i32);
    let Some(dib) = Dib::from_rgb(rgbimg.as_raw(), iw, ih) else {
        DestroyWindow(hwnd);
        return;
    };

    let png = std::fs::read(&path).unwrap_or_default();
    let copied = clipboard::set_image(dib.pixels(), iw, ih, &png);
    let stem = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();

    if let Some(st) = state(hwnd) {
        st.ok_text = format!("{} × {}  {}", w.max(iw), h.max(ih), stem);
    }

    DestroyWindow(hwnd);

    if copied {
        crate::toast("已复制到剪贴板", &format!("{} × {}", iw, ih));
    } else {
        crate::toast_err("剪贴板被占用", "请稍后重试");
    }
    crate::trim_working_set();
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
                let y = ((lparam >> 16) & 0xFFFF) as u16 as i16 as i32;
                let h = hit_row(s, y);
                if h != s.hover {
                    s.hover = h;
                    InvalidateRect(hwnd, ptr::null(), 0);
                }
            }
            0
        }

        WM_LBUTTONUP => {
            if let Some(s) = state(hwnd) {
                let y = ((lparam >> 16) & 0xFFFF) as u16 as i16 as i32;
                if let Some(i) = hit_row(s, y) {
                    s.sel = Some(i);
                    activate(hwnd);
                } else {
                    DestroyWindow(hwnd);
                }
            }
            0
        }

        WM_KEYDOWN => {
            if let Some(s) = state(hwnd) {
                match wparam as u32 {
                    v if v == VK_ESCAPE as u32 => {
                        DestroyWindow(hwnd);
                    }
                    v if v == VK_RETURN as u32 => activate(hwnd),
                    v if (v == VK_UP as u32 || v == VK_DOWN as u32) && !s.items.is_empty() => {
                        let cur = s.sel.unwrap_or(0);
                        let next = if v == VK_UP as u32 {
                            cur.saturating_sub(1)
                        } else {
                            (cur + 1).min(s.items.len() - 1)
                        };
                        s.sel = Some(next);
                        InvalidateRect(hwnd, ptr::null(), 0);
                    }
                    _ => {}
                }
            }
            0
        }

        // 点到别处就关掉
        WM_ACTIVATE => {
            if (wparam & 0xFFFF) == 0 {
                DestroyWindow(hwnd);
            }
            0
        }

        WM_NCDESTROY => {
            let p = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Panel;
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
            if !p.is_null() {
                let b = Box::from_raw(p);
                DeleteObject(b.hbr_bg);
                DeleteObject(b.hbr_hover);
                DeleteObject(b.hbr_sel);
                DeleteObject(b.hfont);
                DeleteObject(b.hfont_small);
                drop(b); // items 里的缩略图 Dib 在这里统一释放
            }
            crate::on_history_closed();
            0
        }

        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}
