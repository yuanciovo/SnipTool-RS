//! 框选遮罩（双模式）。
//!
//! # 两种模式
//!
//! **Dynamic（Alt+A，默认）** —— 不抓全屏。
//! 遮罩用 `SetWindowRgn` 挖洞，洞内直接是**实时桌面**，不经过任何混合，所以不发灰。
//! 代价：选区里的视频还在动（"随手截个图"够用），确认时才 BitBlt 选区。
//!
//! **Frozen（Ctrl+Alt+A，精细）** —— 按下热键的瞬间就把整个虚拟屏幕抓成一张 DIB，
//! 由 `frozen_bg` 那个**不透明**窗口垫在遮罩下面。遮罩挖洞露出来的就是那张静态照片，
//! 绝对静止、绝对不发灰、没有穿透风险。确认时直接从这张 DIB 里裁选区，不再碰屏幕。
//!
//! # 为什么不能把截图贴在遮罩上
//!
//! 遮罩是 `WS_EX_LAYERED` + `LWA_ALPHA=120`。画在它上面的任何像素都会和身后的
//! 实时桌面再混合一次 —— 结果就是发灰、重影、视频仍在动。Spike 3.5 犯过这个错。
//!
//! # 待确认态的交互（只有冻结模式有）
//!
//! 洞一直留着（这样选区内容是干净的），靠 `SetCapture` 保证拖拽期间鼠标事件不丢：
//!   - 内部按下并拖动 → 整体移动选区
//!   - 边框/角上按下并拖动 → 调整大小（8 个方向）
//!   - 选区外按下 → 取消
//!   - √ / Enter 确认，× / Esc 取消
//!
//! 动态模式**没有待确认态**：松手即落盘、写剪贴板、关窗。要的就是快。

use std::mem;
use std::ptr;

use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{
    BeginPaint, CombineRgn, CreateFontW, CreateRectRgn, CreateSolidBrush, DeleteObject, DrawTextW,
    EndPaint, FillRect, FrameRect, InvalidateRect, ScreenToClient, SelectObject, SetBkMode,
    SetTextColor, SetWindowRgn, UpdateWindow, CLEARTYPE_QUALITY, CLIP_DEFAULT_PRECIS,
    DEFAULT_CHARSET, DEFAULT_PITCH, DT_LEFT, DT_NOPREFIX, DT_SINGLELINE, DT_VCENTER, FF_DONTCARE,
    FW_NORMAL, HBRUSH, HFONT, OUT_DEFAULT_PRECIS, PAINTSTRUCT, RGN_DIFF, TRANSPARENT,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
use windows_sys::Win32::UI::HiDpi::GetDpiForWindow;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    ReleaseCapture, SetCapture, SetFocus, VK_ESCAPE, VK_RETURN,
};
use windows_sys::Win32::UI::WindowsAndMessaging::*;

use super::catcher;
use super::frozen_bg;
use super::hud;
use super::toolbar;
use crate::platform::clipboard;
use crate::platform::gdi::Dib;
use crate::platform::win_util::to_wide;
use crate::store;

const OVERLAY_CLASS: &str = "MySnipTool.Overlay";
const MASK_ALPHA: u8 = 120;
const MIN_SIZE: i32 = 4;
/// 尺寸标签盒宽度（按 scale 放大）。够放 "455 × 260  拖内部移动 · 拖边角缩放 · Enter 确认"。
const LABEL_W: i32 = 460;

/// 工具栏把点击结果 PostMessage 回遮罩窗口。
pub const WM_TOOLBAR: u32 = WM_APP + 20;

#[derive(Clone, Copy, PartialEq)]
pub enum Mode {
    /// 不抓全屏，洞内看实时桌面
    Dynamic,
    /// 全屏 DIB 垫底，绝对静止
    Frozen,
}

#[derive(Clone, Copy, PartialEq)]
enum Phase {
    Idle,
    Dragging,
    Pending,
}

/// 8 个缩放手柄方向。
#[derive(Clone, Copy, PartialEq)]
enum Handle {
    NW,
    N,
    NE,
    E,
    SE,
    S,
    SW,
    W,
}

#[derive(Clone, Copy, PartialEq)]
enum Drag {
    None,
    New,
    Move,
    Size(Handle),
}

/// 鼠标松开后该干什么。
enum AfterUp {
    Nothing,
    /// 进入待确认态（冻结模式）
    Pending,
    /// 直接落盘并关窗（动态模式）
    Commit,
}

enum Hit {
    Outside,
    Inside,
    Edge(Handle),
}

struct Overlay {
    mode: Mode,
    phase: Phase,
    drag: Drag,
    anchor: POINT,
    start_rect: RECT,
    start_pt: POINT,
    cur: RECT,
    /// 上一帧实际画到的包围盒（选区外框 + 尺寸标签），用来算这一帧的脏区
    prev_frame: RECT,
    vx: i32,
    vy: i32,
    vw: i32,
    vh: i32,
    scale: i32,
    bg: Option<HWND>,
    /// 冻结模式的全屏底图；其它模式为 None。
    frozen: Option<Dib>,
    bar: Option<HWND>,
    /// 选区上的点击捕获窗（洞内点击不会穿透到下层程序）
    catcher: Option<HWND>,
    hbr_mask: HBRUSH,
    hbr_accent: HBRUSH,
    hbr_ok: HBRUSH,
    hbr_handle: HBRUSH,
    hfont: HFONT,
}

impl Overlay {
    fn sel(&self) -> Option<RECT> {
        if self.cur.right - self.cur.left >= MIN_SIZE && self.cur.bottom - self.cur.top >= MIN_SIZE
        {
            Some(self.cur)
        } else {
            None
        }
    }

    fn bounds(&self) -> RECT {
        RECT {
            left: 0,
            top: 0,
            right: self.vw,
            bottom: self.vh,
        }
    }

    fn to_screen(&self, r: &RECT) -> RECT {
        RECT {
            left: r.left + self.vx,
            top: r.top + self.vy,
            right: r.right + self.vx,
            bottom: r.bottom + self.vy,
        }
    }

    /// 命中测试。先角、再边、再内部。
    fn hit(&self, p: POINT) -> Hit {
        let s = self.cur;
        let hs = 5 * self.scale;

        let near_l = (p.x - s.left).abs() <= hs;
        let near_r = (p.x - s.right).abs() <= hs;
        let near_t = (p.y - s.top).abs() <= hs;
        let near_b = (p.y - s.bottom).abs() <= hs;
        let span_x = p.x >= s.left - hs && p.x <= s.right + hs;
        let span_y = p.y >= s.top - hs && p.y <= s.bottom + hs;

        if near_l && near_t {
            return Hit::Edge(Handle::NW);
        }
        if near_r && near_t {
            return Hit::Edge(Handle::NE);
        }
        if near_l && near_b {
            return Hit::Edge(Handle::SW);
        }
        if near_r && near_b {
            return Hit::Edge(Handle::SE);
        }
        if near_l && span_y {
            return Hit::Edge(Handle::W);
        }
        if near_r && span_y {
            return Hit::Edge(Handle::E);
        }
        if near_t && span_x {
            return Hit::Edge(Handle::N);
        }
        if near_b && span_x {
            return Hit::Edge(Handle::S);
        }
        if p.x > s.left && p.x < s.right && p.y > s.top && p.y < s.bottom {
            return Hit::Inside;
        }
        Hit::Outside
    }
}

const fn rgb(r: u8, g: u8, b: u8) -> u32 {
    (r as u32) | ((g as u32) << 8) | ((b as u32) << 16)
}

fn client_xy(lparam: LPARAM) -> POINT {
    POINT {
        x: (lparam & 0xFFFF) as u16 as i16 as i32,
        y: ((lparam >> 16) & 0xFFFF) as u16 as i16 as i32,
    }
}

fn rect_from(a: POINT, p: POINT) -> RECT {
    RECT {
        left: a.x.min(p.x),
        top: a.y.min(p.y),
        right: a.x.max(p.x),
        bottom: a.y.max(p.y),
    }
}

fn normalize(mut r: RECT, min: i32) -> RECT {
    if r.right < r.left {
        mem::swap(&mut r.left, &mut r.right);
    }
    if r.bottom < r.top {
        mem::swap(&mut r.top, &mut r.bottom);
    }
    if r.right - r.left < min {
        r.right = r.left + min;
    }
    if r.bottom - r.top < min {
        r.bottom = r.top + min;
    }
    r
}

fn resized(start: &RECT, h: Handle, p: POINT, min: i32) -> RECT {
    let mut r = *start;
    match h {
        Handle::NW => {
            r.left = p.x;
            r.top = p.y;
        }
        Handle::N => r.top = p.y,
        Handle::NE => {
            r.right = p.x;
            r.top = p.y;
        }
        Handle::E => r.right = p.x,
        Handle::SE => {
            r.right = p.x;
            r.bottom = p.y;
        }
        Handle::S => r.bottom = p.y,
        Handle::SW => {
            r.left = p.x;
            r.bottom = p.y;
        }
        Handle::W => r.left = p.x,
    }
    normalize(r, min)
}

fn moved(start: &RECT, dx: i32, dy: i32, b: &RECT) -> RECT {
    let w = start.right - start.left;
    let h = start.bottom - start.top;
    let mut x = start.left + dx;
    let mut y = start.top + dy;
    if x < b.left {
        x = b.left;
    }
    if y < b.top {
        y = b.top;
    }
    if x + w > b.right {
        x = b.right - w;
    }
    if y + h > b.bottom {
        y = b.bottom - h;
    }
    RECT {
        left: x,
        top: y,
        right: x + w,
        bottom: y + h,
    }
}

/// 两块矩形的并集（外加 pad）。
fn union(a: &RECT, b: &RECT, pad: i32) -> RECT {
    RECT {
        left: a.left.min(b.left) - pad,
        top: a.top.min(b.top) - pad,
        right: a.right.max(b.right) + pad,
        bottom: a.bottom.max(b.bottom) + pad,
    }
}

/// 尺寸标签盒的位置。**绘制和脏区计算必须走同一个函数**，一旦两边算得不一样，
/// 标签就会有一块永远不被重绘 —— 那就是残影。
fn label_rect(sel: &RECT, client: &RECT, scale: i32) -> RECT {
    let lh = 20 * scale;
    let mut top = sel.top - lh - 4 * scale;
    if top < client.top {
        top = sel.bottom + 6 * scale; // 顶上放不下就翻到选区下方
    }
    RECT {
        left: sel.left + 2 * scale,
        top,
        right: sel.left + LABEL_W * scale,
        bottom: top + lh,
    }
}

/// 这一帧会画到的**全部**区域：外框（含 2px 描边）+ 尺寸标签盒。
///
/// 只算选区是不够的：标签从 `sel.left` 往右铺开 `LABEL_W`，宽度远大于窄选区的
/// 右边界。快速往右/下拖拽时，标签里的数字在变、后面的提示文字跟着左右平移，
/// 而那次平移落在脏区之外 —— 旧字形就留在屏幕上，看起来就是"文字被拉长、重影"。
fn frame_bbox(sel: &RECT, client: &RECT, scale: i32) -> RECT {
    let pad = 2 * scale;
    let mut b = RECT {
        left: sel.left - pad,
        top: sel.top - pad,
        right: sel.right + pad,
        bottom: sel.bottom + pad,
    };
    let l = label_rect(sel, client, scale);
    b.left = b.left.min(l.left);
    b.top = b.top.min(l.top);
    b.right = b.right.max(l.right);
    b.bottom = b.bottom.max(l.bottom);
    b
}

unsafe fn client_rect(hwnd: HWND) -> RECT {
    let mut rc: RECT = mem::zeroed();
    GetClientRect(hwnd, &mut rc);
    rc
}

/// 记下"这一帧已经画到哪儿了"。拖拽开始前调一次，避免第一帧脏区从原点算起。
unsafe fn mark_frame(hwnd: HWND, s: &mut Overlay) {
    s.prev_frame = frame_bbox(&s.cur, &client_rect(hwnd), s.scale);
}

/// 改完形状后**立刻**把这一块画完。
///
/// 这是快速拖拽出残影的根因修复：`SetWindowRgn` 立即改变窗口形状（DWM 下一帧就
/// 按新形状合成），而 `InvalidateRect` 只是把区域标脏、等消息队列空闲才发 WM_PAINT。
/// 快速拖拽时 WM_MOUSEMOVE 把队列塞满，WM_PAINT 一直饿着 —— 于是形状是新的、
/// 表面还是旧的。`UpdateWindow` 强制同步走一次 WM_PAINT，保证同帧内形状与内容一致。
unsafe fn repaint_now(hwnd: HWND, s: &mut Overlay) {
    let now = frame_bbox(&s.cur, &client_rect(hwnd), s.scale);
    let dirty = union(&s.prev_frame, &now, 4 * s.scale);
    s.prev_frame = now;
    InvalidateRect(hwnd, &dirty, 0);
    UpdateWindow(hwnd);
}

/// 强行把遮罩推到前台并拿到**键盘焦点**。
///
/// 热键唤醒时进程通常没有前台权限（按键的不是我们的窗口），只调
/// `SetForegroundWindow` 会被系统静默拒绝 —— 表现就是：遮罩盖满全屏、鼠标能拖，
/// 但 Enter / Esc 全落到上一个程序上。`AttachThreadInput` 到当前前台线程再抢
/// 是绕开前台锁的标准做法。
unsafe fn grab_keyboard(hwnd: HWND) {
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
}

unsafe fn state<'a>(hwnd: HWND) -> Option<&'a mut Overlay> {
    (GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Overlay).as_mut()
}

/// 把当前状态写进窗口标题，方便脚本/Spy++ 观察（窗口无标题栏，用户看不到）。
unsafe fn set_phase_title(hwnd: HWND, phase: &str) {
    let t = to_wide(&format!("MySnipTool Overlay [{phase}]"));
    SetWindowTextW(hwnd, t.as_ptr());
}

// ---------------------------------------------------------------------------
// 打开 / 关闭
// ---------------------------------------------------------------------------

pub unsafe fn open(mode: Mode) -> Option<HWND> {
    let hinstance = GetModuleHandleW(ptr::null());
    if hinstance.is_null() {
        return None;
    }

    let vx = GetSystemMetrics(SM_XVIRTUALSCREEN);
    let vy = GetSystemMetrics(SM_YVIRTUALSCREEN);
    let vw = GetSystemMetrics(SM_CXVIRTUALSCREEN);
    let vh = GetSystemMetrics(SM_CYVIRTUALSCREEN);
    if vw <= 0 || vh <= 0 {
        return None;
    }

    // 冻结模式：**必须在创建任何窗口之前**抓屏，否则抓到的会是我们自己的遮罩。
    let frozen = if mode == Mode::Frozen {
        let scr = RECT {
            left: vx,
            top: vy,
            right: vx + vw,
            bottom: vy + vh,
        };
        Some(Dib::capture(&scr)?)
    } else {
        None
    };

    // 底图窗口先建（z 序在下），遮罩后建（在上）
    let bg = if frozen.is_some() {
        Some(frozen_bg::create(vx, vy, vw, vh)?)
    } else {
        None
    };

    let class_name = to_wide(OVERLAY_CLASS);
    let title = to_wide("MySnipTool Overlay");
    let face = to_wide("Microsoft YaHei UI");

    let mut wc: WNDCLASSW = mem::zeroed();
    wc.style = CS_HREDRAW | CS_VREDRAW;
    wc.lpfnWndProc = Some(wnd_proc);
    wc.hInstance = hinstance;
    wc.hCursor = LoadCursorW(ptr::null_mut(), IDC_CROSS);
    wc.lpszClassName = class_name.as_ptr();
    RegisterClassW(&wc);

    let hwnd = CreateWindowExW(
        WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_LAYERED,
        class_name.as_ptr(),
        title.as_ptr(),
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
        if let Some(b) = bg {
            DestroyWindow(b);
        }
        return None;
    }

    let dpi = {
        let d = GetDpiForWindow(hwnd);
        if d == 0 {
            96
        } else {
            d as i32
        }
    };
    let scale = (dpi as f32 / 96.0).round() as i32;

    let st = Box::new(Overlay {
        mode,
        phase: Phase::Idle,
        drag: Drag::None,
        anchor: POINT { x: 0, y: 0 },
        start_rect: RECT {
            left: 0,
            top: 0,
            right: 0,
            bottom: 0,
        },
        start_pt: POINT { x: 0, y: 0 },
        cur: RECT {
            left: 0,
            top: 0,
            right: 0,
            bottom: 0,
        },
        prev_frame: RECT {
            left: 0,
            top: 0,
            right: 0,
            bottom: 0,
        },
        vx,
        vy,
        vw,
        vh,
        scale,
        bg,
        frozen,
        bar: None,
        catcher: None,
        hbr_mask: CreateSolidBrush(rgb(0, 0, 0)),
        hbr_accent: CreateSolidBrush(rgb(0, 174, 255)),
        hbr_ok: CreateSolidBrush(rgb(48, 209, 88)),
        hbr_handle: CreateSolidBrush(rgb(245, 245, 247)),
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
    });
    let raw = Box::into_raw(st);
    SetWindowLongPtrW(hwnd, GWLP_USERDATA, raw as isize);

    // 底图指针要等 Overlay 落到堆上才稳定
    if let Some(b) = bg {
        if let Some(f) = (*raw).frozen.as_ref() {
            frozen_bg::attach(b, f as *const Dib);
        }
        ShowWindow(b, SW_SHOWNOACTIVATE);
    }

    SetLayeredWindowAttributes(hwnd, 0, MASK_ALPHA, LWA_ALPHA);
    apply_region(hwnd, None);
    ShowWindow(hwnd, SW_SHOW);
    grab_keyboard(hwnd);
    set_phase_title(hwnd, "idle");
    UpdateWindow(hwnd);

    Some(hwnd)
}

unsafe fn apply_region(hwnd: HWND, sel: Option<&RECT>) {
    let mut rc: RECT = mem::zeroed();
    GetClientRect(hwnd, &mut rc);
    let rgn = CreateRectRgn(0, 0, rc.right, rc.bottom);
    if rgn.is_null() {
        return;
    }
    if let Some(s) = sel {
        let hole = CreateRectRgn(s.left, s.top, s.right, s.bottom);
        if !hole.is_null() {
            CombineRgn(rgn, rgn, hole, RGN_DIFF);
            DeleteObject(hole);
        }
    }
    SetWindowRgn(hwnd, rgn, 1);
}

// ---------------------------------------------------------------------------
// 绘制
// ---------------------------------------------------------------------------

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
    FillRect(hdc, &rc, s.hbr_mask);

    if let Some(sel) = s.sel() {
        let pending = s.phase == Phase::Pending;
        let brush = if pending { s.hbr_ok } else { s.hbr_accent };
        let pad = 2 * s.scale;

        let mut outer = RECT {
            left: sel.left - pad,
            top: sel.top - pad,
            right: sel.right + pad,
            bottom: sel.bottom + pad,
        };
        FrameRect(hdc, &outer, brush);
        outer.left += 1;
        outer.top += 1;
        outer.right -= 1;
        outer.bottom -= 1;
        FrameRect(hdc, &outer, brush);

        if pending {
            draw_handles(hdc, &sel, s.hbr_handle, 4 * s.scale);
        }
        draw_size_label(hdc, &sel, &rc, s.hfont, s.scale, pending);
    }

    EndPaint(hwnd, &ps);
}

unsafe fn draw_handles(
    hdc: windows_sys::Win32::Graphics::Gdi::HDC,
    sel: &RECT,
    brush: HBRUSH,
    half: i32,
) {
    let xs = [sel.left, (sel.left + sel.right) / 2, sel.right];
    let ys = [sel.top, (sel.top + sel.bottom) / 2, sel.bottom];

    for (i, y) in ys.iter().enumerate() {
        for (j, x) in xs.iter().enumerate() {
            if i == 1 && j == 1 {
                continue; // 正中间不画
            }
            let r = RECT {
                left: x - half,
                top: y - half,
                right: x + half,
                bottom: y + half,
            };
            FillRect(hdc, &r, brush);
        }
    }
}

unsafe fn draw_size_label(
    hdc: windows_sys::Win32::Graphics::Gdi::HDC,
    sel: &RECT,
    client: &RECT,
    hfont: HFONT,
    scale: i32,
    pending: bool,
) {
    let w = sel.right - sel.left;
    let h = sel.bottom - sel.top;
    let text = to_wide(&if pending {
        format!("{} × {}    拖内部移动 · 拖边角缩放 · Enter 确认", w, h)
    } else {
        format!("{} × {}", w, h)
    });

    let mut tr = label_rect(sel, client, scale);

    SelectObject(hdc, hfont);
    SetBkMode(hdc, TRANSPARENT as i32);
    SetTextColor(hdc, rgb(255, 255, 255));
    DrawTextW(
        hdc,
        text.as_ptr(),
        -1,
        &mut tr,
        DT_LEFT | DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX,
    );
}

// ---------------------------------------------------------------------------
// 状态迁移
// ---------------------------------------------------------------------------

/// 框选完成 → 待确认：开工具栏、标题置位。region 的洞保持开着。
unsafe fn enter_pending(hwnd: HWND) {
    let Some(s) = state(hwnd) else { return };
    if s.sel().is_none() {
        DestroyWindow(hwnd);
        return;
    }

    s.phase = Phase::Pending;
    set_phase_title(hwnd, "pending");

    let screen = s.to_screen(&s.cur);
    // 先建捕获窗、再建工具栏：工具栏可能翻到选区内部，必须压在捕获窗上面
    s.catcher = catcher::create(hwnd, s.vx, s.vy, &screen);
    s.bar = toolbar::open(hwnd, &screen);

    s.prev_frame = frame_bbox(&s.cur, &client_rect(hwnd), s.scale);
    InvalidateRect(hwnd, ptr::null(), 0);
    UpdateWindow(hwnd);

    // 工具栏和捕获窗都是 WS_EX_NOACTIVATE，理论上抢不走焦点；但这里再要一次
    // 成本极低，能兜住"某些环境下焦点被切走 → Enter/Esc 失灵"的情况。
    grab_keyboard(hwnd);
}

/// 落盘 + 写剪贴板 + 关窗。**不要求处在待确认态**：动态模式松手就直接走这里。
unsafe fn commit(hwnd: HWND) {
    let Some(s) = state(hwnd) else { return };
    if s.sel().is_none() {
        DestroyWindow(hwnd);
        return;
    }

    let sel = s.cur;
    // 冻结模式：直接从全屏底图里裁，不再碰屏幕（所以内容绝对静止）
    // 动态模式：此刻洞还开着，BitBlt 拿到的就是真实桌面
    let shot = match s.mode {
        Mode::Frozen => s.frozen.as_ref().and_then(|f| Dib::from_dc(f.dc(), &sel)),
        Mode::Dynamic => Dib::capture(&s.to_screen(&sel)),
    };

    let Some(d) = shot else {
        DestroyWindow(hwnd);
        hud::show(false, "截图失败", "无法读取屏幕区域");
        return;
    };
    let (w, h) = (d.w, d.h);

    // 先销毁遮罩再落盘：窗口不在了，用户马上能看到"截完了"，而编码在后台这点时间
    // 反正也感知不到；反过来则会让遮罩多闪一帧。
    DestroyWindow(hwnd);

    let rgb_buf = store::bgra_to_rgb(d.pixels());
    let saved = store::save_png(&rgb_buf, w, h);
    drop(rgb_buf);

    let path = match saved {
        Ok(p) => p,
        Err(e) => {
            hud::show(false, "保存失败", &e.to_string());
            return;
        }
    };

    // 缩略图在这一刻生成：全分辨率 DIB 还在手里，StretchBlt 一次只要几毫秒
    if let Some(stem) = path.file_stem().map(|s| s.to_string_lossy().to_string()) {
        store::save_thumb(&d, &stem);
    }

    let png = std::fs::read(&path).unwrap_or_default();
    let copied = clipboard::set_image(d.pixels(), w, h, &png);
    drop(png);

    if copied {
        hud::show(true, "已复制到剪贴板", &format!("{} × {}", w, h));
    } else {
        hud::show(false, "已保存，剪贴板被占用", &format!("{} × {}", w, h));
    }

    crate::trim_working_set();
}

// ---------------------------------------------------------------------------
// 消息
// ---------------------------------------------------------------------------

unsafe extern "system" fn wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_LBUTTONDOWN => {
            let Some(s) = state(hwnd) else { return 0 };
            let p = client_xy(lparam);

            match s.phase {
                Phase::Idle => {
                    s.anchor = p;
                    s.cur = RECT {
                        left: p.x,
                        top: p.y,
                        right: p.x,
                        bottom: p.y,
                    };
                    s.phase = Phase::Dragging;
                    s.drag = Drag::New;
                    mark_frame(hwnd, s);
                    set_phase_title(hwnd, "dragging");
                    SetCapture(hwnd);
                }
                Phase::Pending => match s.hit(p) {
                    Hit::Edge(h) => {
                        s.drag = Drag::Size(h);
                        s.start_rect = s.cur;
                        s.start_pt = p;
                        mark_frame(hwnd, s);
                        set_phase_title(hwnd, "resizing");
                        SetCapture(hwnd);
                    }
                    Hit::Inside => {
                        s.drag = Drag::Move;
                        s.start_rect = s.cur;
                        s.start_pt = p;
                        mark_frame(hwnd, s);
                        set_phase_title(hwnd, "moving");
                        SetCapture(hwnd);
                    }
                    Hit::Outside => {
                        DestroyWindow(hwnd);
                    }
                },
                Phase::Dragging => {}
            }
            0
        }

        WM_MOUSEMOVE => {
            let Some(s) = state(hwnd) else { return 0 };
            let raw = client_xy(lparam);
            // 夹在虚拟屏内：拖到屏幕外时 BitBlt 会读到越界区域，成图边缘会有黑边
            let b = s.bounds();
            let p = POINT {
                x: raw.x.clamp(b.left, b.right),
                y: raw.y.clamp(b.top, b.bottom),
            };
            let min = MIN_SIZE * s.scale;

            match s.drag {
                Drag::New => {
                    s.cur = rect_from(s.anchor, p);
                    let sel = s.sel();
                    apply_region(hwnd, sel.as_ref());
                    repaint_now(hwnd, s);
                }
                Drag::Move => {
                    let b = s.bounds();
                    s.cur = moved(&s.start_rect, p.x - s.start_pt.x, p.y - s.start_pt.y, &b);
                    let sel = s.sel();
                    apply_region(hwnd, sel.as_ref());
                    repaint_now(hwnd, s);
                    let scr = s.to_screen(&s.cur);
                    if let Some(c) = s.catcher {
                        catcher::relocate(c, &scr);
                    }
                    if let Some(bar) = s.bar {
                        toolbar::reposition(bar, &scr);
                    }
                }
                Drag::Size(h) => {
                    s.cur = resized(&s.start_rect, h, p, min);
                    let sel = s.sel();
                    apply_region(hwnd, sel.as_ref());
                    repaint_now(hwnd, s);
                    let scr = s.to_screen(&s.cur);
                    if let Some(c) = s.catcher {
                        catcher::relocate(c, &scr);
                    }
                    if let Some(bar) = s.bar {
                        toolbar::reposition(bar, &scr);
                    }
                }
                Drag::None => {}
            }
            0
        }

        WM_LBUTTONUP => {
            // 先把状态改完、把借用放掉，再决定要做什么：
            // commit 内部还会再取一次 state()，同时持有两个 &mut 是自找麻烦。
            let act = match state(hwnd) {
                None => AfterUp::Nothing,
                Some(s) => match s.drag {
                    Drag::New => {
                        s.drag = Drag::None;
                        ReleaseCapture();
                        s.cur = normalize(s.cur, MIN_SIZE * s.scale);
                        let sel = s.sel();
                        apply_region(hwnd, sel.as_ref());
                        if s.mode == Mode::Dynamic {
                            // 极速模式：松手即落盘，不进待确认、不弹工具栏
                            AfterUp::Commit
                        } else {
                            AfterUp::Pending
                        }
                    }
                    Drag::Move | Drag::Size(_) => {
                        s.drag = Drag::None;
                        ReleaseCapture();
                        set_phase_title(hwnd, "pending");
                        AfterUp::Nothing
                    }
                    Drag::None => AfterUp::Nothing,
                },
            };

            match act {
                AfterUp::Commit => commit(hwnd),
                AfterUp::Pending => enter_pending(hwnd),
                AfterUp::Nothing => {}
            }
            0
        }

        WM_RBUTTONDOWN => {
            DestroyWindow(hwnd);
            0
        }

        WM_KEYDOWN => {
            match wparam as u32 {
                v if v == VK_ESCAPE as u32 => {
                    DestroyWindow(hwnd);
                }
                v if v == VK_RETURN as u32 => commit(hwnd),
                _ => {}
            }
            0
        }

        WM_TOOLBAR => {
            if wparam == toolbar::ID_CONFIRM {
                commit(hwnd);
            } else if wparam == toolbar::ID_CANCEL {
                DestroyWindow(hwnd);
            }
            0
        }

        WM_SETCURSOR => {
            if let Some(s) = state(hwnd) {
                SetCursor(LoadCursorW(ptr::null_mut(), cursor_for(hwnd, s)));
                return 1;
            }
            0
        }

        WM_PAINT => {
            paint(hwnd);
            0
        }

        WM_ERASEBKGND => 1,

        WM_NCDESTROY => {
            ReleaseCapture();
            let p = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Overlay;
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
            if !p.is_null() {
                let b = Box::from_raw(p);
                // 顺序要紧：底图窗口先销毁（它的 wndproc 还引用着 DIB），再释放 DIB
                if let Some(bg) = b.bg {
                    DestroyWindow(bg);
                }
                if let Some(c) = b.catcher {
                    DestroyWindow(c);
                }
                if let Some(bar) = b.bar {
                    DestroyWindow(bar);
                }
                DeleteObject(b.hbr_mask);
                DeleteObject(b.hbr_accent);
                DeleteObject(b.hbr_ok);
                DeleteObject(b.hbr_handle);
                DeleteObject(b.hfont);
                drop(b);
            }
            crate::on_overlay_closed();
            0
        }

        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

/// 按当前鼠标位置和状态给出光标形状。
unsafe fn cursor_for(hwnd: HWND, s: &Overlay) -> windows_sys::core::PCWSTR {
    if s.phase != Phase::Pending {
        return IDC_CROSS;
    }
    let mut p = POINT { x: 0, y: 0 };
    GetCursorPos(&mut p);
    ScreenToClient(hwnd, &mut p);

    match s.hit(p) {
        Hit::Edge(Handle::NW) | Hit::Edge(Handle::SE) => IDC_SIZENWSE,
        Hit::Edge(Handle::NE) | Hit::Edge(Handle::SW) => IDC_SIZENESW,
        Hit::Edge(Handle::N) | Hit::Edge(Handle::S) => IDC_SIZENS,
        Hit::Edge(_) => IDC_SIZEWE,
        Hit::Inside => IDC_SIZEALL,
        Hit::Outside => IDC_ARROW,
    }
}
