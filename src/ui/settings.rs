//! 设置窗口 · 全自绘（除数字框用一个原生 EDIT）。
//!
//! 为什么不用原生控件：Windows 一旦启用视觉主题，BUTTON / 复选框会**直接忽略**
//! `WM_CTLCOLORBTN`，想改颜色只能 `BS_OWNERDRAW` 自己画一遍 —— 那已经是自绘，
//! 只是套了个原生壳。所以这里索性整窗自绘，只保留一个原生 `EDIT` 输入数字：
//! 输入法、光标、选区、退格、右键菜单全是白送的。
//!
//! # GDI 预算
//! 所有字体/画刷/画笔在 `Theme::new()` 建一次，`Drop` 里统一删。
//! 绘制路径上**零创建**：`RoundRect` / `Ellipse` 只是 `SelectObject` 换对象。
//! 常驻 4 字体 + 11 画刷 + 4 画笔 = 19 个，与开关次数无关。

use std::mem;
use std::ptr;

use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Dwm::{
    DwmSetWindowAttribute, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND,
};
use windows_sys::Win32::Graphics::Gdi::{
    BeginPaint, BitBlt, CreateBrushIndirect, CreateCompatibleBitmap, CreateCompatibleDC,
    CreateFontW, CreatePen, CreateRoundRectRgn, CreateSolidBrush, DeleteDC, DeleteObject,
    DrawTextW, Ellipse, EndPaint, FillRect, GetDC, IntersectClipRect, InvalidateRect, LineTo,
    MoveToEx, ReleaseDC, RestoreDC, RoundRect, SaveDC, ScreenToClient, SelectObject, SetBkColor,
    SetBkMode, SetTextColor, SetWindowRgn, BS_HOLLOW, CLEARTYPE_QUALITY, CLIP_DEFAULT_PRECIS,
    DEFAULT_CHARSET, DEFAULT_PITCH, DT_CENTER, DT_LEFT, DT_NOPREFIX, DT_RIGHT, DT_SINGLELINE,
    DT_VCENTER, FF_DONTCARE, FW_NORMAL, FW_SEMIBOLD, HBITMAP, HBRUSH, HDC, HFONT, HGDIOBJ, HPEN,
    LOGBRUSH, OUT_DEFAULT_PRECIS, PAINTSTRUCT, PS_NULL, PS_SOLID, SRCCOPY, TRANSPARENT,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
use windows_sys::Win32::UI::HiDpi::GetDpiForSystem;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    SetActiveWindow, SetFocus, TrackMouseEvent, TME_LEAVE, TRACKMOUSEEVENT, VK_ESCAPE, VK_RETURN,
    VK_TAB,
};
use windows_sys::Win32::UI::WindowsAndMessaging::*;

use crate::config::Config;
use crate::platform::autostart;
use crate::platform::hotkey::{self, Combo};
use crate::platform::win_util::to_wide;

const CLASS: &str = "MySnipTool.Settings";
const TITLE: &str = "设置";

const ID_HOURS: usize = 4001;
const TIMER_ANIM: usize = 1;
/// 160ms 走完 / 16ms 一跳 = 10 帧，每帧 0.1
const ANIM_STEP: f32 = 0.1;

const WM_MOUSELEAVE: u32 = 0x02A3;
const WM_CTLCOLOREDIT: u32 = 0x0133;
const EM_SETLIMITTEXT: u32 = 0x00C5;
const WM_SETFONT: u32 = 0x0030;
/// 子类化的 EDIT 把 Tab / Enter 转给父窗口
const MSG_TAB: u32 = WM_APP + 40;
const MSG_ENTER: u32 = WM_APP + 41;

// ---------------------------------------------------------------------------
// 色板（参考图实测值）
// ---------------------------------------------------------------------------

const fn rgb(r: u8, g: u8, b: u8) -> u32 {
    (r as u32) | ((g as u32) << 8) | ((b as u32) << 16)
}

const C_BG: u32 = rgb(0xF5, 0xF5, 0xF5);
const C_CARD: u32 = rgb(0xFF, 0xFF, 0xFF);
const C_TEXT: u32 = rgb(0x1F, 0x1F, 0x1F);
const C_MUTED: u32 = rgb(0x8A, 0x8A, 0x8A);
const C_ACCENT: u32 = rgb(0x00, 0x66, 0xFF);
const C_ACCENT_HI: u32 = rgb(0x33, 0x85, 0xFF);
const C_ACCENT_DN: u32 = rgb(0x00, 0x52, 0xCC);
const C_GRAY: u32 = rgb(0xEF, 0xEF, 0xEF);
const C_GRAY_HI: u32 = rgb(0xE4, 0xE4, 0xE4);
const C_GRAY_DN: u32 = rgb(0xD6, 0xD6, 0xD6);
const C_DIVIDER: u32 = rgb(0xED, 0xED, 0xED);
const C_BORDER: u32 = rgb(0xE0, 0xE0, 0xE0);
const C_TRACK_OFF: u32 = rgb(0xD9, 0xD9, 0xD9);
const C_DANGER: u32 = rgb(0xD9, 0x30, 0x25);
const C_WHITE: u32 = rgb(0xFF, 0xFF, 0xFF);
const C_BTN_TEXT: u32 = rgb(0x33, 0x33, 0x33);

// ---------------------------------------------------------------------------
// 布局（100% 缩放下的逻辑坐标，用时乘 scale）
// ---------------------------------------------------------------------------

const CLIENT_W: i32 = 480;
const CLIENT_H: i32 = 552;
const CARD_L: i32 = 16;
const CARD_R: i32 = 464;
const IN_L: i32 = 40;
const IN_R: i32 = 440;
const ROW_H: i32 = 52;
const FIELD_W: i32 = 152;
const FIELD_H: i32 = 32;
const BTN_W: i32 = 112;
const BTN_H: i32 = 36;
const TRACK_W: i32 = 52;
const TRACK_H: i32 = 30;
const KNOB_D: i32 = 22;
const HEADER_H: i32 = 56;

#[derive(Clone, Copy, PartialEq)]
enum Field {
    None,
    Quick,
    Frozen,
    History,
    Hours,
    Toggle,
    Save,
    Cancel,
    Close,
    Header,
}

struct Layout {
    header: i32,
    card1: RECT,
    card2: RECT,
    card3: RECT,
    row_quick: RECT,
    row_frozen: RECT,
    row_history: RECT,
    row_hours: RECT,
    row_auto: RECT,
    f_quick: RECT,
    f_frozen: RECT,
    f_history: RECT,
    f_hours: RECT,
    edit_hours: RECT,
    toggle: RECT,
    btn_save: RECT,
    btn_cancel: RECT,
    close: RECT,
    status: RECT,
    sec1: i32,
    sec2: i32,
    sec3: i32,
}

impl Layout {
    fn new(sc: i32) -> Self {
        let s = |v: i32| v * sc;
        let row = |top: i32| RECT {
            left: s(CARD_L),
            top: s(top),
            right: s(CARD_R),
            bottom: s(top + ROW_H),
        };
        let field = |r: &RECT| RECT {
            left: s(IN_R) - s(FIELD_W),
            top: (r.top + r.bottom) / 2 - s(FIELD_H) / 2,
            right: s(IN_R),
            bottom: (r.top + r.bottom) / 2 + s(FIELD_H) / 2,
        };

        let row_quick = row(106);
        let row_frozen = row(158);
        let row_history = row(210);
        let row_hours = row(306);
        let row_auto = RECT {
            left: s(CARD_L),
            top: s(406),
            right: s(CARD_R),
            bottom: s(466),
        };

        let f_hours = RECT {
            left: s(IN_R) - s(72),
            top: (row_hours.top + row_hours.bottom) / 2 - s(FIELD_H) / 2,
            right: s(IN_R),
            bottom: (row_hours.top + row_hours.bottom) / 2 + s(FIELD_H) / 2,
        };
        // 内缩到圆角里，原生 EDIT 的方块四角就看不见了
        let edit_hours = RECT {
            left: f_hours.left + s(10),
            top: f_hours.top + s(6),
            right: f_hours.right - s(10),
            bottom: f_hours.bottom - s(6),
        };

        let cy = (row_auto.top + row_auto.bottom) / 2;
        let toggle = RECT {
            left: s(IN_R) - s(TRACK_W),
            top: cy - s(TRACK_H) / 2,
            right: s(IN_R),
            bottom: cy + s(TRACK_H) / 2,
        };

        Self {
            header: s(HEADER_H),
            card1: RECT {
                left: s(CARD_L),
                top: s(106),
                right: s(CARD_R),
                bottom: s(262),
            },
            card2: RECT {
                left: s(CARD_L),
                top: s(306),
                right: s(CARD_R),
                bottom: s(362),
            },
            card3: RECT {
                left: s(CARD_L),
                top: s(406),
                right: s(CARD_R),
                bottom: s(466),
            },
            f_quick: field(&row_quick),
            f_frozen: field(&row_frozen),
            f_history: field(&row_history),
            edit_hours,
            f_hours,
            toggle,
            row_quick,
            row_frozen,
            row_history,
            row_hours,
            row_auto,
            btn_save: RECT {
                left: s(CARD_R) - s(BTN_W),
                top: s(500),
                right: s(CARD_R),
                bottom: s(500 + BTN_H),
            },
            btn_cancel: RECT {
                left: s(CARD_R) - s(2 * BTN_W) - s(8),
                top: s(500),
                right: s(CARD_R) - s(BTN_W) - s(8),
                bottom: s(500 + BTN_H),
            },
            close: RECT {
                left: s(CLIENT_W) - s(40),
                top: s(20),
                right: s(CLIENT_W) - s(8),
                bottom: s(52),
            },
            status: RECT {
                left: s(IN_L),
                top: s(468),
                right: s(IN_R),
                bottom: s(492),
            },
            sec1: s(96),
            sec2: s(296),
            sec3: s(396),
        }
    }

    fn hit(&self, p: POINT) -> Field {
        let ins = |r: &RECT| p.x >= r.left && p.x < r.right && p.y >= r.top && p.y < r.bottom;
        if ins(&self.close) {
            return Field::Close;
        }
        if ins(&self.f_quick) {
            return Field::Quick;
        }
        if ins(&self.f_frozen) {
            return Field::Frozen;
        }
        if ins(&self.f_history) {
            return Field::History;
        }
        if ins(&self.f_hours) {
            return Field::Hours;
        }
        if ins(&self.toggle) {
            return Field::Toggle;
        }
        if ins(&self.btn_save) {
            return Field::Save;
        }
        if ins(&self.btn_cancel) {
            return Field::Cancel;
        }
        if p.y < self.header {
            return Field::Header;
        }
        Field::None
    }
}

// ---------------------------------------------------------------------------
// 主题对象：建一次，Drop 统一删
// ---------------------------------------------------------------------------

struct Theme {
    f_title: HFONT,
    f_label: HFONT,
    f_body: HFONT,
    f_small: HFONT,
    brushes: [HBRUSH; 11],
    br_card: HBRUSH,
    br_accent: HBRUSH,
    br_accent_hi: HBRUSH,
    br_accent_dn: HBRUSH,
    br_gray: HBRUSH,
    br_gray_hi: HBRUSH,
    br_gray_dn: HBRUSH,
    br_track_off: HBRUSH,
    br_divider: HBRUSH,
    br_hollow: HBRUSH,
    pen_null: HPEN,
    pen_accent: HPEN,
    pen_border: HPEN,
    pen_close: HPEN,
}

unsafe fn font(size: i32, weight: u32) -> HFONT {
    let face = to_wide("Microsoft YaHei UI");
    CreateFontW(
        -size,
        0,
        0,
        0,
        weight as i32,
        0,
        0,
        0,
        DEFAULT_CHARSET as u32,
        OUT_DEFAULT_PRECIS as u32,
        CLIP_DEFAULT_PRECIS as u32,
        CLEARTYPE_QUALITY as u32,
        (DEFAULT_PITCH | FF_DONTCARE) as u32,
        face.as_ptr(),
    )
}

impl Theme {
    unsafe fn new(sc: i32) -> Self {
        let hollow = LOGBRUSH {
            lbStyle: BS_HOLLOW,
            lbColor: 0,
            lbHatch: 0,
        };
        let br_card = CreateSolidBrush(C_CARD);
        let br_accent = CreateSolidBrush(C_ACCENT);
        let br_accent_hi = CreateSolidBrush(C_ACCENT_HI);
        let br_accent_dn = CreateSolidBrush(C_ACCENT_DN);
        let br_gray = CreateSolidBrush(C_GRAY);
        let br_gray_hi = CreateSolidBrush(C_GRAY_HI);
        let br_gray_dn = CreateSolidBrush(C_GRAY_DN);
        let br_track_off = CreateSolidBrush(C_TRACK_OFF);
        let br_divider = CreateSolidBrush(C_DIVIDER);
        let br_hollow = CreateBrushIndirect(&hollow);
        let brushes = [
            CreateSolidBrush(C_BG),
            br_card,
            br_accent,
            br_accent_hi,
            br_accent_dn,
            br_gray,
            br_gray_hi,
            br_gray_dn,
            br_track_off,
            br_divider,
            br_hollow,
        ];
        Self {
            f_title: font(20 * sc, FW_SEMIBOLD),
            f_label: font(14 * sc, FW_SEMIBOLD),
            f_body: font(13 * sc, FW_NORMAL),
            f_small: font(12 * sc, FW_NORMAL),
            brushes,
            br_card,
            br_accent,
            br_accent_hi,
            br_accent_dn,
            br_gray,
            br_gray_hi,
            br_gray_dn,
            br_track_off,
            br_divider,
            br_hollow,
            pen_null: CreatePen(PS_NULL, 0, 0),
            pen_accent: CreatePen(PS_SOLID, sc, C_ACCENT),
            pen_border: CreatePen(PS_SOLID, sc, C_BORDER),
            pen_close: CreatePen(PS_SOLID, sc + sc / 2, C_MUTED),
        }
    }
}

impl Drop for Theme {
    fn drop(&mut self) {
        unsafe {
            for f in [self.f_title, self.f_label, self.f_body, self.f_small] {
                DeleteObject(f as HGDIOBJ);
            }
            for b in self.brushes {
                DeleteObject(b as HGDIOBJ);
            }
            for p in [
                self.pen_null,
                self.pen_accent,
                self.pen_border,
                self.pen_close,
            ] {
                DeleteObject(p as HGDIOBJ);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// 状态
// ---------------------------------------------------------------------------

struct Settings {
    sc: i32,
    theme: Theme,
    quick: Combo,
    frozen: Combo,
    history: Combo,
    autostart: bool,
    /// 0.0 = 关，1.0 = 开
    anim: f32,
    anim_target: f32,
    animating: bool,
    focus: Field,
    capturing: bool,
    capture_bad: bool,
    hover: Field,
    pressed: Field,
    status: String,
    tracking: bool,
    edit: HWND,
    edit_old: isize,
    back: Option<BackBuf>,
}

/// 双缓冲用的离屏位图。
struct BackBuf {
    dc: HDC,
    bmp: HBITMAP,
    old: HGDIOBJ,
}

impl Drop for Settings {
    fn drop(&mut self) {
        if let Some(b) = self.back.take() {
            unsafe {
                if !b.dc.is_null() {
                    SelectObject(b.dc, b.old);
                    DeleteDC(b.dc);
                }
                if !b.bmp.is_null() {
                    DeleteObject(b.bmp as HGDIOBJ);
                }
            }
        }
    }
}

unsafe fn state<'a>(hwnd: HWND) -> Option<&'a mut Settings> {
    (GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Settings).as_mut()
}

/// 把设置窗推到最前并拿到焦点。
///
/// 托盘程序拿不到前台权限时 `SetForegroundWindow` 会被系统静默拒绝（返回 0 也没用），
/// 窗口就停在别的程序下面。`AttachThreadInput` 到当前前台线程再抢是标准绕法。
pub unsafe fn bring_to_front(hwnd: HWND) {
    let fg = GetForegroundWindow();
    let mut pid = 0u32;
    let fg_thread = if fg.is_null() {
        0
    } else {
        GetWindowThreadProcessId(fg, &mut pid)
    };
    let me = GetCurrentThreadId();
    let attached = fg_thread != 0 && fg_thread != me && AttachThreadInput(me, fg_thread, 1) != 0;
    SetForegroundWindow(hwnd);
    SetFocus(hwnd);
    if attached {
        AttachThreadInput(me, fg_thread, 0);
    }

    // 抢到前台**不等于**看得见：如果桌面上存在 topmost 窗口（任务栏、某些
    // 播放器/常驻工具、Electron 应用），我们虽然成了前台，仍然被压在它下面 ——
    // 用户看到的是"窗口没出来"，自动化点击则全部打在那个置顶窗上。
    // 实测：foreground==settings 但 WindowFromPoint(保存按钮) 返回别的进程。
    //
    // 所以这一步**无条件**执行：提到 z 序最前，再立刻退回普通层级
    // （只改 z 序，不留下置顶状态）。
    {
        SetWindowPos(
            hwnd,
            HWND_TOPMOST,
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
        );
        SetWindowPos(hwnd, HWND_NOTOPMOST, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE);
        BringWindowToTop(hwnd);
        SetActiveWindow(hwnd);
        SetForegroundWindow(hwnd);
    }
}

// ---------------------------------------------------------------------------
// 绘制原语
// ---------------------------------------------------------------------------

unsafe fn round_fill(hdc: HDC, rc: &RECT, radius: i32, brush: HBRUSH, pen: HPEN) {
    let ob = SelectObject(hdc, brush as HGDIOBJ);
    let op = SelectObject(hdc, pen as HGDIOBJ);
    RoundRect(
        hdc,
        rc.left,
        rc.top,
        rc.right,
        rc.bottom,
        radius * 2,
        radius * 2,
    );
    SelectObject(hdc, ob);
    SelectObject(hdc, op);
}

unsafe fn round_stroke(hdc: HDC, rc: &RECT, radius: i32, pen: HPEN, hollow: HBRUSH) {
    round_fill(hdc, rc, radius, hollow, pen);
}

unsafe fn circle(hdc: HDC, cx: i32, cy: i32, d: i32, brush: HBRUSH, pen: HPEN) {
    let ob = SelectObject(hdc, brush as HGDIOBJ);
    let op = SelectObject(hdc, pen as HGDIOBJ);
    Ellipse(hdc, cx - d / 2, cy - d / 2, cx + d / 2, cy + d / 2);
    SelectObject(hdc, ob);
    SelectObject(hdc, op);
}

unsafe fn text(hdc: HDC, s: &str, rc: &mut RECT, f: HFONT, color: u32, flags: u32) {
    SelectObject(hdc, f as HGDIOBJ);
    SetBkMode(hdc, TRANSPARENT as i32);
    SetTextColor(hdc, color);
    let w = to_wide(s);
    DrawTextW(hdc, w.as_ptr(), -1, rc, flags | DT_SINGLELINE | DT_NOPREFIX);
}

// ---------------------------------------------------------------------------
// 入口
// ---------------------------------------------------------------------------

pub unsafe fn open(cfg: &Config) -> Option<HWND> {
    let hinstance = GetModuleHandleW(ptr::null());
    if hinstance.is_null() {
        return None;
    }

    let dpi = {
        let d = GetDpiForSystem();
        if d == 0 {
            96
        } else {
            d as i32
        }
    };
    let sc = (dpi as f32 / 96.0).round() as i32;

    let class = to_wide(CLASS);
    let mut wc: WNDCLASSW = mem::zeroed();
    // 窗口大小固定，不需要 CS_HREDRAW/CS_VREDRAW；留着反而会引入整窗重绘
    wc.style = 0;
    wc.lpfnWndProc = Some(wnd_proc);
    wc.hInstance = hinstance;
    wc.hCursor = LoadCursorW(ptr::null_mut(), IDC_ARROW);
    wc.lpszClassName = class.as_ptr();
    RegisterClassW(&wc);

    let w = CLIENT_W * sc;
    let h = CLIENT_H * sc;
    let sw = GetSystemMetrics(SM_CXSCREEN);
    let sh = GetSystemMetrics(SM_CYSCREEN);
    let x = (sw - w) / 2;
    let y = ((sh - h) / 2 - 40 * sc).max(0);

    // WS_EX_APPWINDOW：给它一个任务栏按钮 + Alt+Tab 条目。
    // 用 TOOLWINDOW 的话，用户一点浏览器它就沉到后面、既没有任务栏图标也不在
    // Alt+Tab 里 —— 看起来就是"消失了"，只能靠托盘菜单捞回来。
    let hwnd = CreateWindowExW(
        WS_EX_APPWINDOW,
        class.as_ptr(),
        to_wide(TITLE).as_ptr(),
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

    // Win11：系统级平滑圆角（带抗锯齿，比 SetWindowRgn 的硬边好看）
    // Win10：该属性不存在，返回非 0 → 退回硬边 region
    let pref: i32 = DWMWCP_ROUND;
    let hr = DwmSetWindowAttribute(
        hwnd,
        DWMWA_WINDOW_CORNER_PREFERENCE as u32,
        &pref as *const i32 as *const core::ffi::c_void,
        4,
    );
    if hr != 0 {
        let rgn = CreateRoundRectRgn(0, 0, w + 1, h + 1, 20 * sc, 20 * sc);
        SetWindowRgn(hwnd, rgn, 1);
    }

    let auto = autostart::is_enabled();
    let theme = Theme::new(sc);
    let raw = Box::into_raw(Box::new(Settings {
        sc,
        theme,
        quick: cfg.quick,
        frozen: cfg.frozen,
        history: cfg.history,
        autostart: auto,
        anim: if auto { 1.0 } else { 0.0 },
        anim_target: if auto { 1.0 } else { 0.0 },
        animating: false,
        focus: Field::None,
        capturing: false,
        capture_bad: false,
        hover: Field::None,
        pressed: Field::None,
        status: String::new(),
        tracking: false,
        edit: ptr::null_mut(),
        edit_old: 0,
        back: None,
    }));
    SetWindowLongPtrW(hwnd, GWLP_USERDATA, raw as isize);

    let lay = Layout::new(sc);
    let edit = CreateWindowExW(
        0,
        to_wide("EDIT").as_ptr(),
        to_wide(&cfg.retention_hours.to_string()).as_ptr(),
        WS_CHILD | WS_VISIBLE | ES_NUMBER as u32 | ES_AUTOHSCROLL as u32,
        lay.edit_hours.left,
        lay.edit_hours.top,
        lay.edit_hours.right - lay.edit_hours.left,
        lay.edit_hours.bottom - lay.edit_hours.top,
        hwnd,
        ID_HOURS as isize as *mut core::ffi::c_void,
        hinstance,
        ptr::null(),
    );
    if !edit.is_null() {
        SendMessageW(edit, WM_SETFONT, (*raw).theme.f_body as usize, 1);
        SendMessageW(edit, EM_SETLIMITTEXT, 3, 0);
        let old = SetWindowLongPtrW(edit, GWLP_WNDPROC, edit_proc as *const () as isize);
        (*raw).edit_old = old;
        (*raw).edit = edit;
    }

    // 离屏缓冲一次建好，之后所有绘制都先落在它上面
    let wdc = GetDC(hwnd);
    let bdc = CreateCompatibleDC(wdc);
    let bmp = CreateCompatibleBitmap(wdc, w, h);
    ReleaseDC(hwnd, wdc);
    if !bdc.is_null() && !bmp.is_null() {
        let old = SelectObject(bdc, bmp as HGDIOBJ);
        (*raw).back = Some(BackBuf { dc: bdc, bmp, old });
    }

    ShowWindow(hwnd, SW_SHOW);
    bring_to_front(hwnd);
    Some(hwnd)
}

// ---------------------------------------------------------------------------
// 绘制
// ---------------------------------------------------------------------------

unsafe fn paint(hwnd: HWND) {
    let mut ps: PAINTSTRUCT = mem::zeroed();
    let win_dc = BeginPaint(hwnd, &mut ps);
    if win_dc.is_null() {
        EndPaint(hwnd, &ps);
        return;
    }
    let Some(st) = state(hwnd) else {
        EndPaint(hwnd, &ps);
        return;
    };
    // 双缓冲：所有绘制先落在离屏 DC，最后一次 BitBlt 上去。
    // 直接画窗口 DC 的话，一帧里几十个 GDI 调用会被 DWM 逐块合成，
    // 鼠标划过按钮时能看见"只有底色、卡片还没画上"的那一瞬间 —— 那就是闪烁。
    let hdc = match st.back.as_ref() {
        Some(b) => b.dc,
        None => win_dc,
    };
    let buffered = hdc != win_dc;
    let saved = if buffered {
        let sd = SaveDC(hdc);
        IntersectClipRect(
            hdc,
            ps.rcPaint.left,
            ps.rcPaint.top,
            ps.rcPaint.right,
            ps.rcPaint.bottom,
        );
        sd
    } else {
        0
    };
    let th = &st.theme;
    let sc = st.sc;
    let lay = Layout::new(sc);
    let mut rc: RECT = mem::zeroed();
    GetClientRect(hwnd, &mut rc);

    FillRect(hdc, &rc, st.theme.brushes[0]);
    round_stroke(hdc, &rc, 10 * sc, th.pen_border, th.br_hollow);

    // 标题
    let mut tr = RECT {
        left: 24 * sc,
        top: 0,
        right: 300 * sc,
        bottom: HEADER_H * sc,
    };
    text(
        hdc,
        TITLE,
        &mut tr,
        th.f_title,
        C_TEXT,
        DT_LEFT | DT_VCENTER,
    );

    // 关闭按钮
    if st.hover == Field::Close {
        round_fill(hdc, &lay.close, 6 * sc, th.br_gray, th.pen_null);
    }
    let cx = (lay.close.left + lay.close.right) / 2;
    let cy = (lay.close.top + lay.close.bottom) / 2;
    let arm = 6 * sc;
    let op = SelectObject(hdc, th.pen_close as HGDIOBJ);
    MoveToEx(hdc, cx - arm, cy - arm, ptr::null_mut());
    LineTo(hdc, cx + arm, cy + arm);
    MoveToEx(hdc, cx + arm, cy - arm, ptr::null_mut());
    LineTo(hdc, cx - arm, cy + arm);
    SelectObject(hdc, op);

    // 分区标题
    let mut s = RECT {
        left: 24 * sc,
        top: lay.sec1 - 10 * sc,
        right: 400 * sc,
        bottom: lay.sec1 + 8 * sc,
    };
    text(
        hdc,
        "快捷键",
        &mut s,
        th.f_small,
        C_MUTED,
        DT_LEFT | DT_VCENTER,
    );
    let mut s = RECT {
        left: 24 * sc,
        top: lay.sec2 - 10 * sc,
        right: 400 * sc,
        bottom: lay.sec2 + 8 * sc,
    };
    text(
        hdc,
        "文件清理",
        &mut s,
        th.f_small,
        C_MUTED,
        DT_LEFT | DT_VCENTER,
    );
    let mut s = RECT {
        left: 24 * sc,
        top: lay.sec3 - 10 * sc,
        right: 400 * sc,
        bottom: lay.sec3 + 8 * sc,
    };
    text(
        hdc,
        "系统设置",
        &mut s,
        th.f_small,
        C_MUTED,
        DT_LEFT | DT_VCENTER,
    );

    // 卡片
    for c in [&lay.card1, &lay.card2, &lay.card3] {
        round_fill(hdc, c, 10 * sc, th.br_card, th.pen_null);
    }

    // 三行热键
    let rows = [
        (
            Field::Quick,
            st.quick,
            "截图（动态）",
            &lay.row_quick,
            &lay.f_quick,
        ),
        (
            Field::Frozen,
            st.frozen,
            "截图（冻结）",
            &lay.row_frozen,
            &lay.f_frozen,
        ),
        (
            Field::History,
            st.history,
            "历史面板",
            &lay.row_history,
            &lay.f_history,
        ),
    ];
    for (i, (field, combo, label, row, fr)) in rows.iter().enumerate() {
        let mut lr = RECT {
            left: IN_L * sc,
            top: row.top,
            right: IN_R * sc,
            bottom: row.bottom,
        };
        text(
            hdc,
            label,
            &mut lr,
            th.f_label,
            C_TEXT,
            DT_LEFT | DT_VCENTER,
        );
        draw_hotkey_field(hdc, st, *field, fr, Some(&combo.label()));
        if i < 2 {
            let d = RECT {
                left: IN_L * sc,
                top: row.bottom - sc / 2,
                right: IN_R * sc,
                bottom: row.bottom + sc / 2,
            };
            FillRect(hdc, &d, th.br_divider);
        }
    }

    // 保留时长
    let mut lr = RECT {
        left: IN_L * sc,
        top: lay.row_hours.top,
        right: IN_R * sc,
        bottom: lay.row_hours.bottom,
    };
    text(
        hdc,
        "保留时长",
        &mut lr,
        th.f_label,
        C_TEXT,
        DT_LEFT | DT_VCENTER,
    );
    draw_hotkey_field(hdc, st, Field::Hours, &lay.f_hours, None);
    let mut hint = RECT {
        left: IN_L * sc,
        top: lay.row_hours.top,
        right: lay.f_hours.left - 12 * sc,
        bottom: lay.row_hours.bottom,
    };
    text(
        hdc,
        "小时（1–720）",
        &mut hint,
        th.f_small,
        C_MUTED,
        DT_RIGHT | DT_VCENTER,
    );

    // 开机自启
    let mut al = RECT {
        left: IN_L * sc,
        top: lay.row_auto.top,
        right: IN_R * sc,
        bottom: lay.row_auto.bottom,
    };
    text(
        hdc,
        "开机自动启动",
        &mut al,
        th.f_label,
        C_TEXT,
        DT_LEFT | DT_VCENTER,
    );
    draw_toggle(hdc, &lay.toggle, st.anim, th, sc);

    // 状态行
    if !st.status.is_empty() {
        let mut sr = lay.status;
        text(
            hdc,
            &st.status,
            &mut sr,
            th.f_small,
            C_DANGER,
            DT_LEFT | DT_VCENTER,
        );
    }

    // 按钮
    draw_button(hdc, st, &lay.btn_cancel, "取消", false);
    draw_button(hdc, st, &lay.btn_save, "保存", true);

    if buffered {
        RestoreDC(hdc, saved);
        let bw = ps.rcPaint.right - ps.rcPaint.left;
        let bh = ps.rcPaint.bottom - ps.rcPaint.top;
        if bw > 0 && bh > 0 {
            BitBlt(
                win_dc,
                ps.rcPaint.left,
                ps.rcPaint.top,
                bw,
                bh,
                hdc,
                ps.rcPaint.left,
                ps.rcPaint.top,
                SRCCOPY,
            );
        }
    }
    EndPaint(hwnd, &ps);
}

unsafe fn draw_button(hdc: HDC, st: &Settings, rc: &RECT, label: &str, primary: bool) {
    let field = if primary { Field::Save } else { Field::Cancel };
    let brush = if primary {
        if st.pressed == field {
            st.theme.br_accent_dn
        } else if st.hover == field {
            st.theme.br_accent_hi
        } else {
            st.theme.br_accent
        }
    } else if st.pressed == field {
        st.theme.br_gray_dn
    } else if st.hover == field {
        st.theme.br_gray_hi
    } else {
        st.theme.br_gray
    };
    round_fill(hdc, rc, 8 * st.sc, brush, st.theme.pen_null);
    let mut tr = *rc;
    text(
        hdc,
        label,
        &mut tr,
        st.theme.f_body,
        if primary { C_WHITE } else { C_BTN_TEXT },
        DT_CENTER | DT_VCENTER,
    );
}

fn field_rect(lay: &Layout, f: Field) -> Option<RECT> {
    match f {
        Field::Quick => Some(lay.f_quick),
        Field::Frozen => Some(lay.f_frozen),
        Field::History => Some(lay.f_history),
        Field::Hours => Some(lay.f_hours),
        Field::Toggle => Some(lay.toggle),
        Field::Save => Some(lay.btn_save),
        Field::Cancel => Some(lay.btn_cancel),
        Field::Close => Some(lay.close),
        _ => None,
    }
}

/// 只标脏一个控件（外扩 2 逻辑像素，盖住聚焦描边和悬停底色）。
/// 悬停变化非常频繁，整窗重绘会把 CPU 白烧在没变的地方。
unsafe fn invalidate_field(hwnd: HWND, sc: i32, f: Field) {
    let lay = Layout::new(sc);
    match field_rect(&lay, f) {
        Some(r) => {
            let pad = 2 * sc;
            let r = RECT {
                left: r.left - pad,
                top: r.top - pad,
                right: r.right + pad,
                bottom: r.bottom + pad,
            };
            InvalidateRect(hwnd, &r, 0);
        }
        None => {
            InvalidateRect(hwnd, ptr::null(), 0);
        }
    }
}

/// 画一个输入框底。`value` 传 `None` 表示只画底（数字框的文字由原生 EDIT 负责）。
unsafe fn draw_hotkey_field(hdc: HDC, st: &Settings, field: Field, rc: &RECT, value: Option<&str>) {
    let sc = st.sc;
    let focused = st.focus == field;
    round_fill(
        hdc,
        rc,
        8 * sc,
        if focused {
            st.theme.br_card
        } else {
            st.theme.brushes[0]
        },
        st.theme.pen_null,
    );
    if focused {
        round_stroke(hdc, rc, 8 * sc, st.theme.pen_accent, st.theme.br_hollow);
    }
    let Some(value) = value else { return };

    let (txt, color) = if st.capturing && focused {
        if st.capture_bad {
            ("需要至少一个修饰键".to_string(), C_DANGER)
        } else {
            ("按下组合…".to_string(), C_MUTED)
        }
    } else {
        (value.to_string(), C_TEXT)
    };
    let mut tr = RECT {
        left: rc.left + 12 * sc,
        top: rc.top,
        right: rc.right - 6 * sc,
        bottom: rc.bottom,
    };
    text(
        hdc,
        &txt,
        &mut tr,
        st.theme.f_body,
        color,
        DT_LEFT | DT_VCENTER,
    );
}

unsafe fn draw_toggle(hdc: HDC, rc: &RECT, anim: f32, th: &Theme, sc: i32) {
    let track = if anim >= 0.5 {
        th.br_accent
    } else {
        th.br_track_off
    };
    round_fill(hdc, rc, (rc.bottom - rc.top) / 2, track, th.pen_null);

    let d = KNOB_D * sc;
    let pad = ((rc.bottom - rc.top) - d) / 2;
    let x0 = rc.left + pad + d / 2;
    let x1 = rc.right - pad - d / 2;
    let cx = x0 + ((x1 - x0) as f32 * anim) as i32;
    circle(
        hdc,
        cx,
        (rc.top + rc.bottom) / 2,
        d,
        th.br_card,
        th.pen_null,
    );
}

// ---------------------------------------------------------------------------
// 交互
// ---------------------------------------------------------------------------

/// 按下：只切焦点，不执行动作（动作在抬起时判定，和原生按钮一致）。
unsafe fn focus_on(hwnd: HWND, f: Field) {
    let Some(st) = state(hwnd) else { return };
    let prev = st.focus;
    if st.focus != f {
        st.capturing = false;
        st.capture_bad = false;
    }
    st.focus = f;
    match f {
        Field::Quick | Field::Frozen | Field::History => {
            st.capturing = true;
            SetFocus(hwnd);
        }
        Field::Hours => {
            SetFocus(st.edit);
        }
        _ => {
            SetFocus(hwnd);
        }
    }
    let sc = st.sc;
    invalidate_field(hwnd, sc, prev);
    invalidate_field(hwnd, sc, f);
}

unsafe fn click(hwnd: HWND, f: Field) {
    let Some(st) = state(hwnd) else { return };
    match f {
        Field::Toggle => {
            st.autostart = !st.autostart;
            st.anim_target = if st.autostart { 1.0 } else { 0.0 };
            if !st.animating {
                st.animating = true;
                SetTimer(hwnd, TIMER_ANIM, 16, None);
            }
            InvalidateRect(hwnd, ptr::null(), 0);
        }
        Field::Save => save(hwnd),
        Field::Cancel | Field::Close => {
            DestroyWindow(hwnd);
        }
        _ => {}
    }
}

fn next_focus(cur: Field, back: bool) -> Field {
    const ORDER: [Field; 6] = [
        Field::Quick,
        Field::Frozen,
        Field::History,
        Field::Hours,
        Field::Toggle,
        Field::Save,
    ];
    let n = ORDER.len();
    let next = match ORDER.iter().position(|f| *f == cur) {
        None => 0,
        Some(i) => {
            if back {
                (i + n - 1) % n
            } else {
                (i + 1) % n
            }
        }
    };
    ORDER[next]
}

unsafe fn move_focus(hwnd: HWND, back: bool) {
    let Some(st) = state(hwnd) else { return };
    let prev = st.focus;
    let target = next_focus(st.focus, back);
    st.focus = target;
    st.capturing = false;
    st.capture_bad = false;
    if target == Field::Hours {
        SetFocus(st.edit);
    } else {
        SetFocus(hwnd);
    }
    let sc = st.sc;
    invalidate_field(hwnd, sc, prev);
    invalidate_field(hwnd, sc, target);
}

unsafe fn handle_key(hwnd: HWND, vk: u32) -> bool {
    let Some(st) = state(hwnd) else { return false };

    if vk == VK_ESCAPE as u32 {
        if st.capturing {
            st.capturing = false;
            st.capture_bad = false;
            InvalidateRect(hwnd, ptr::null(), 0);
        } else {
            DestroyWindow(hwnd);
        }
        return true;
    }
    if vk == VK_TAB as u32 {
        move_focus(hwnd, hotkey::shift_down());
        return true;
    }
    if vk == VK_RETURN as u32 {
        if st.capturing {
            st.capturing = false;
            InvalidateRect(hwnd, ptr::null(), 0);
        } else {
            save(hwnd);
        }
        return true;
    }

    if !matches!(st.focus, Field::Quick | Field::Frozen | Field::History) {
        return false;
    }
    if !st.capturing {
        st.capturing = true;
        st.capture_bad = false;
    }
    if hotkey::is_modifier(vk) {
        InvalidateRect(hwnd, ptr::null(), 0);
        return true;
    }
    let mods = hotkey::mods_now();
    if mods == 0 {
        st.capture_bad = true;
        InvalidateRect(hwnd, ptr::null(), 0);
        return true;
    }
    let combo = Combo::new(mods, vk);
    match st.focus {
        Field::Quick => st.quick = combo,
        Field::Frozen => st.frozen = combo,
        _ => st.history = combo,
    }
    st.capturing = false;
    st.capture_bad = false;
    st.status.clear();
    InvalidateRect(hwnd, ptr::null(), 0);
    true
}

// ---------------------------------------------------------------------------
// 保存
// ---------------------------------------------------------------------------

unsafe fn save(hwnd: HWND) {
    let Some(st) = state(hwnd) else { return };

    let mut buf = [0u16; 8];
    let n = GetWindowTextW(st.edit, buf.as_mut_ptr(), buf.len() as i32);
    let raw = String::from_utf16_lossy(&buf[..n.max(0) as usize]);
    let parsed = raw
        .trim()
        .parse::<u32>()
        .ok()
        .filter(|h| (1..=720).contains(h));
    let Some(hours) = parsed else {
        st.status = "保留时长要填 1–720 之间的整数".into();
        InvalidateRect(hwnd, ptr::null(), 0);
        return;
    };
    if st.quick == st.frozen || st.quick == st.history || st.frozen == st.history {
        st.status = "三个功能的快捷键不能重复".into();
        InvalidateRect(hwnd, ptr::null(), 0);
        return;
    }

    let (q, f) = (st.quick, st.frozen);
    let result = crate::apply_config(Config {
        quick: q,
        frozen: f,
        history: st.history,
        retention_hours: hours,
        autostart: st.autostart,
    });

    if result.all_ok() {
        DestroyWindow(hwnd);
        crate::toast(
            "设置已保存",
            &format!("{} 截图 · {} 冻结", q.label(), f.label()),
        );
        return;
    }

    let mut bad: Vec<String> = Vec::new();
    if !result.quick {
        bad.push(q.label());
    }
    if !result.frozen {
        bad.push(f.label());
    }
    if !result.history {
        bad.push(st.history.label());
    }
    st.status = format!("{} 注册失败（被占用），换一个组合", bad.join(" / "));
    InvalidateRect(hwnd, ptr::null(), 0);
}

// ---------------------------------------------------------------------------
// 数字框子类化：只为了把 Tab / Enter / Esc 转给父窗口
// ---------------------------------------------------------------------------

unsafe extern "system" fn edit_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    let parent = GetParent(hwnd);
    let old = state(parent).map(|s| s.edit_old).unwrap_or(0);

    if msg == WM_KEYDOWN {
        match wp as u32 {
            v if v == VK_TAB as u32 => {
                PostMessageW(parent, MSG_TAB, if hotkey::shift_down() { 1 } else { 0 }, 0);
                return 0;
            }
            v if v == VK_RETURN as u32 => {
                PostMessageW(parent, MSG_ENTER, 0, 0);
                return 0;
            }
            v if v == VK_ESCAPE as u32 => {
                PostMessageW(parent, WM_CLOSE, 0, 0);
                return 0;
            }
            _ => {}
        }
    }
    let proc: WNDPROC = mem::transmute(old);
    CallWindowProcW(proc, hwnd, msg, wp, lp)
}

// ---------------------------------------------------------------------------
// 窗口过程
// ---------------------------------------------------------------------------

fn point_from(lp: LPARAM) -> POINT {
    POINT {
        x: (lp & 0xFFFF) as u16 as i16 as i32,
        y: ((lp >> 16) & 0xFFFF) as u16 as i16 as i32,
    }
}

unsafe extern "system" fn wnd_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_PAINT => {
            paint(hwnd);
            0
        }

        WM_ERASEBKGND => 1,

        WM_MOUSEMOVE => {
            let Some(st) = state(hwnd) else { return 0 };
            if !st.tracking {
                let mut t: TRACKMOUSEEVENT = mem::zeroed();
                t.cbSize = mem::size_of::<TRACKMOUSEEVENT>() as u32;
                t.dwFlags = TME_LEAVE;
                t.hwndTrack = hwnd;
                TrackMouseEvent(&mut t);
                st.tracking = true;
            }
            let lay = Layout::new(st.sc);
            let f = lay.hit(point_from(lp));
            if f != st.hover {
                let old = st.hover;
                st.hover = f;
                let sc = st.sc;
                invalidate_field(hwnd, sc, old);
                invalidate_field(hwnd, sc, f);
            }
            0
        }

        WM_MOUSELEAVE => {
            if let Some(st) = state(hwnd) {
                let old = st.hover;
                st.tracking = false;
                st.hover = Field::None;
                let sc = st.sc;
                invalidate_field(hwnd, sc, old);
            }
            0
        }

        WM_LBUTTONDOWN => {
            let Some(st) = state(hwnd) else { return 0 };
            let lay = Layout::new(st.sc);
            let f = lay.hit(point_from(lp));
            st.pressed = f;
            let sc = st.sc;
            if f != Field::Header && f != Field::None {
                focus_on(hwnd, f);
            }
            invalidate_field(hwnd, sc, f);
            0
        }

        WM_LBUTTONUP => {
            let Some(st) = state(hwnd) else { return 0 };
            let lay = Layout::new(st.sc);
            let f = lay.hit(point_from(lp));
            let pressed = st.pressed;
            st.pressed = Field::None;
            let sc = st.sc;
            invalidate_field(hwnd, sc, pressed);
            if pressed == f && f != Field::None && f != Field::Header {
                click(hwnd, f);
            }
            0
        }

        WM_TIMER => {
            if wp == TIMER_ANIM {
                let Some(st) = state(hwnd) else { return 0 };
                let d = st.anim_target - st.anim;
                if d.abs() <= ANIM_STEP {
                    st.anim = st.anim_target;
                    st.animating = false;
                    KillTimer(hwnd, TIMER_ANIM);
                } else {
                    st.anim += ANIM_STEP * d.signum();
                }
                let lay = Layout::new(st.sc);
                InvalidateRect(hwnd, &lay.toggle, 0);
            }
            0
        }

        WM_KEYDOWN | WM_SYSKEYDOWN => {
            if handle_key(hwnd, wp as u32) {
                0
            } else {
                DefWindowProcW(hwnd, msg, wp, lp)
            }
        }

        // Alt 按下后会跟一个 WM_SYSCHAR，不吃掉会"叮"一声
        WM_SYSCHAR => 0,

        WM_CTLCOLOREDIT => {
            let hdc = wp as HDC;
            match state(hwnd) {
                Some(st) => {
                    let focused = st.focus == Field::Hours;
                    SetBkColor(hdc, if focused { C_CARD } else { C_BG });
                    if focused {
                        st.theme.br_card as LRESULT
                    } else {
                        st.theme.brushes[0] as LRESULT
                    }
                }
                None => DefWindowProcW(hwnd, msg, wp, lp),
            }
        }

        MSG_TAB => {
            move_focus(hwnd, wp != 0);
            0
        }

        MSG_ENTER => {
            save(hwnd);
            0
        }

        WM_NCHITTEST => {
            let r = DefWindowProcW(hwnd, msg, wp, lp);
            if r == HTCLIENT as LRESULT {
                let mut p = POINT {
                    x: (lp & 0xFFFF) as u16 as i16 as i32,
                    y: ((lp >> 16) & 0xFFFF) as u16 as i16 as i32,
                };
                ScreenToClient(hwnd, &mut p);
                if let Some(st) = state(hwnd) {
                    let lay = Layout::new(st.sc);
                    let ins =
                        |r: &RECT| p.x >= r.left && p.x < r.right && p.y >= r.top && p.y < r.bottom;
                    // 标题栏区域交给系统拖动；关闭按钮和控件仍走客户区
                    if p.y < lay.header && !ins(&lay.close) {
                        return HTCAPTION as LRESULT;
                    }
                }
            }
            r
        }

        WM_CLOSE => {
            DestroyWindow(hwnd);
            0
        }

        WM_DESTROY => {
            let p = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Settings;
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
            if !p.is_null() {
                drop(Box::from_raw(p)); // Theme::drop 里删掉全部 GDI 对象
            }
            crate::on_settings_closed();
            0
        }

        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}
