//! MySnipTool · 轻量 Windows 截图工具
//!
//! 常驻形态：无主窗口、无弹窗，只有托盘图标 + 三个全局热键。
//!
//! 模块划分（按 docs/技术方案.md §8）：
//!   - `platform/`  Win32 封装，unsafe 全部关在这里，对外只暴露安全接口
//!   - `capture/`   截图与遮罩
//!   - `ui/`        历史面板、设置窗口
//!   - `store`      临时目录与 PNG 落盘
//!   - `cleanup`    定时清理
//!   - `config`     配置持久化

#![cfg_attr(not(test), windows_subsystem = "windows")]

mod capture;
mod cleanup;
mod config;
mod platform;
mod store;
mod ui;

use std::mem;
use std::ptr;

use windows_sys::Win32::Foundation::{GetLastError, HINSTANCE, HWND, LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Threading::{GetCurrentProcess, SetProcessWorkingSetSize};
use windows_sys::Win32::UI::HiDpi::{
    SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
};
use windows_sys::Win32::UI::WindowsAndMessaging::*;

use platform::{autostart, hotkey, tray, win_util};

const APP_NAME: &str = "MySnipTool";
const WINDOW_CLASS: &str = "MySnipTool.TrayHost";
const WINDOW_TITLE: &str = "MySnipTool Tray Host";

pub(crate) const IDM_CAPTURE: usize = 1001;
pub(crate) const IDM_EXIT: usize = 1002;
pub(crate) const IDM_CAPTURE_FROZEN: usize = 1003;
pub(crate) const IDM_HISTORY: usize = 1004;
pub(crate) const IDM_OPEN_LATEST: usize = 1005;
pub(crate) const IDM_OPEN_FOLDER: usize = 1006;
pub(crate) const IDM_SETTINGS: usize = 1007;

const TIMER_CLEANUP: usize = 1;

pub(crate) struct App {
    /// 托盘宿主窗口。历史面板 / 设置窗口用它做 owner。
    pub main_hwnd: HWND,
    pub config: config::Config,
    pub hotkeys: hotkey::Status,
    pub overlay: Option<HWND>,
    pub history: Option<HWND>,
    pub settings: Option<HWND>,
}

// 单线程 Win32 消息循环：APP 只在 wnd_proc / 菜单 / 热键分支里访问，不存在并发。
// 用 static mut 而不是 Mutex，是为了避免每条消息都过一遍锁——这里没有第二个线程。
#[allow(static_mut_refs)]
static mut APP: Option<App> = None;

#[allow(static_mut_refs)]
pub(crate) fn app() -> &'static mut App {
    unsafe { APP.as_mut().expect("APP 未初始化") }
}

// ---------------------------------------------------------------------------
// 给 capture/overlay 用的回调
// ---------------------------------------------------------------------------

pub(crate) fn on_overlay_closed() {
    app().overlay = None;
}

pub(crate) fn on_history_closed() {
    app().history = None;
}

pub(crate) fn on_settings_closed() {
    app().settings = None;
}

/// 打开历史面板。已经开着就先关掉（当快捷键是开关用）。
fn toggle_history() {
    let a = app();
    if let Some(h) = a.history {
        unsafe { DestroyWindow(h) };
        a.history = None;
        return;
    }
    let hwnd = unsafe { ui::history::open() };
    a.history = hwnd;
}

/// 打开设置窗口。已经开着就把它提到前面，不重复开第二个。
fn toggle_settings() {
    let a = app();
    if let Some(h) = a.settings {
        unsafe { ui::settings::bring_to_front(h) };
        return;
    }
    let cfg = a.config.clone();
    a.settings = unsafe { ui::settings::open(&cfg) };
}

/// 保存并应用一份新配置：先注销旧热键、落盘、改自启、重注册、刷新提示与定时器。
///
/// 一次性全做完而不是"存了等重启"：用户点保存就该立刻能用新快捷键。
/// 注册失败的组合不会静默吞掉，返回值交给设置窗口显示。
pub(crate) fn apply_config(cfg: config::Config) -> hotkey::Status {
    unsafe {
        let hwnd = app().main_hwnd;
        hotkey::unregister(hwnd);
        let st = hotkey::register(hwnd, &cfg);
        let _ = config::save(&cfg);
        autostart::set_enabled(cfg.autostart);

        let a = app();
        a.config = cfg;
        a.hotkeys = st;

        refresh_tray_tip();
        reset_cleanup_timer();
        // 保留时长可能刚被改小，立刻扫一次，别让用户等下一个周期
        run_cleanup();
        st
    }
}

unsafe fn refresh_tray_tip() {
    let a = app();
    let suffix = if a.hotkeys.all_ok() {
        ""
    } else {
        "（部分热键未注册）"
    };
    let tip = format!("MySnipTool · {} 截图{}", a.config.quick.label(), suffix);
    tray::set_tip(a.main_hwnd, &tip);
}

/// 清理周期取保留时长的 1/4，夹在 5 分钟到 1 小时之间。
///
/// 固定 24h 已经不合适了：保留时长可以调到 1 小时，一天扫一次等于让文件多活一天。
/// 扫描本身只读目录项元数据（几十微秒），频率高一点没有代价。
fn cleanup_interval_ms(hours: u32) -> u32 {
    let quarter = hours as u64 * 900_000;
    quarter.clamp(300_000, 3_600_000) as u32
}

unsafe fn reset_cleanup_timer() {
    let a = app();
    let hwnd = a.main_hwnd;
    let ms = cleanup_interval_ms(a.config.retention_hours);
    KillTimer(hwnd, TIMER_CLEANUP);
    SetTimer(hwnd, TIMER_CLEANUP, ms, None);
}

/// 截图/清理的结果反馈统一走迷你 HUD（不再用系统托盘气泡）。
pub(crate) fn toast(title: &str, body: &str) {
    unsafe { capture::hud::show(true, title, body) }
}

pub(crate) fn toast_err(title: &str, body: &str) {
    unsafe { capture::hud::show(false, title, body) }
}

/// 把工作集里的冷页面 page 出去，让任务管理器上的常驻内存回到基线。
///
/// **这不是"释放内存"**：页只是被移到 standby list，下次访问会软缺页回来（代价很低）。
/// 但本工具的用户指标就是任务管理器里的数字，而 PNG 编码器分配的堆块释放后
/// 会一直留在工作集里（实测 8 次连拍稳定多占 8MB），不裁剪的话 Spike 4 加面板必爆线。
///
/// 只在一次截图完整走完后调一次，不能放在热路径上。
pub(crate) fn trim_working_set() {
    unsafe {
        SetProcessWorkingSetSize(GetCurrentProcess(), usize::MAX, usize::MAX);
    }
}

/// 清理过期截图。启动补跑一次，之后按保留时长的 1/4 周期跑（见 WM_TIMER）。
/// 只遍历目录项元数据，图像一个字节都不读进内存。
fn run_cleanup() {
    let dir = store::temp_dir();
    if !dir.is_dir() {
        return;
    }
    let secs = app().config.retention_hours as u64 * 3600;
    let stat = unsafe { cleanup::sweep(&dir, secs) };
    // 默认静默。只有删不掉时才打扰用户（多半是文件被别的程序占着）。
    if stat.failed > 0 {
        toast_err(
            &format!("清理了 {} 个过期截图", stat.deleted),
            &format!("{} 个文件被占用，下次再试", stat.failed),
        );
    }
}

/// 打开框选遮罩。已在框选中则忽略（防连按热键）。
fn open_capture(mode: capture::overlay::Mode) {
    let a = app();
    if a.overlay.is_some() {
        return;
    }
    let hwnd = unsafe { capture::overlay::open(mode) };
    a.overlay = hwnd;
}

// ---------------------------------------------------------------------------
// 窗口
// ---------------------------------------------------------------------------

unsafe extern "system" fn wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        tray::WM_TRAY_CALLBACK => {
            if matches!(lparam as u32, WM_RBUTTONUP | WM_CONTEXTMENU) {
                tray::menu(hwnd);
            }
            0
        }

        WM_COMMAND => {
            match wparam & 0xFFFF {
                IDM_CAPTURE => open_capture(capture::overlay::Mode::Dynamic),
                IDM_CAPTURE_FROZEN => open_capture(capture::overlay::Mode::Frozen),
                IDM_HISTORY => toggle_history(),
                IDM_OPEN_LATEST => {
                    if let Some(e) = store::list_recent(1).into_iter().next() {
                        unsafe { platform::shell::open_path(&e.path) };
                    }
                }
                IDM_OPEN_FOLDER => {
                    let d = store::ensure_dir().unwrap_or_else(|_| store::temp_dir());
                    unsafe { platform::shell::open_path(&d) };
                }
                IDM_SETTINGS => toggle_settings(),
                IDM_EXIT => {
                    DestroyWindow(hwnd);
                }
                _ => {}
            }
            0
        }

        WM_HOTKEY => {
            match wparam as i32 {
                hotkey::HK_QUICK => open_capture(capture::overlay::Mode::Dynamic),
                hotkey::HK_FROZEN => open_capture(capture::overlay::Mode::Frozen),
                hotkey::HK_HISTORY => toggle_history(),
                _ => {}
            }
            0
        }

        WM_TIMER => {
            if wparam == TIMER_CLEANUP {
                run_cleanup();
            }
            0
        }

        WM_QUERYENDSESSION => 1,

        WM_DESTROY => {
            hotkey::unregister(hwnd);
            tray::del(hwnd);
            PostQuitMessage(0);
            0
        }

        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

unsafe fn register_class(hinstance: HINSTANCE, class_name: &[u16]) -> Result<(), String> {
    let mut wc: WNDCLASSW = mem::zeroed();
    wc.style = CS_DBLCLKS;
    wc.lpfnWndProc = Some(wnd_proc);
    wc.hInstance = hinstance;
    wc.lpszClassName = class_name.as_ptr();

    if RegisterClassW(&wc) == 0 {
        return Err(format!(
            "RegisterClassW 失败，GetLastError={}",
            GetLastError()
        ));
    }
    Ok(())
}

/// 永不显示的宿主窗口（不用 HWND_MESSAGE，见技术方案 §9 #3）。
unsafe fn create_host(
    hinstance: HINSTANCE,
    class_name: &[u16],
    title: &[u16],
) -> Result<HWND, String> {
    let hwnd = CreateWindowExW(
        0,
        class_name.as_ptr(),
        title.as_ptr(),
        0,
        0,
        0,
        0,
        0,
        ptr::null_mut(),
        ptr::null_mut(),
        hinstance,
        ptr::null(),
    );
    if hwnd.is_null() {
        return Err(format!(
            "CreateWindowExW 失败，GetLastError={}",
            GetLastError()
        ));
    }
    Ok(hwnd)
}

// ---------------------------------------------------------------------------
// 入口
// ---------------------------------------------------------------------------

fn run() -> Result<(), String> {
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);

        let class_name = win_util::to_wide(WINDOW_CLASS);
        let title = win_util::to_wide(WINDOW_TITLE);

        if !FindWindowW(class_name.as_ptr(), ptr::null()).is_null() {
            return Ok(()); // 已有实例
        }

        let hinstance = GetModuleHandleW(ptr::null());
        if hinstance.is_null() {
            return Err(format!(
                "GetModuleHandleW 失败，GetLastError={}",
                GetLastError()
            ));
        }

        register_class(hinstance, &class_name)?;
        let hwnd = create_host(hinstance, &class_name, &title)?;

        let (cfg, existed) = config::load();

        APP = Some(App {
            main_hwnd: hwnd,
            config: cfg.clone(),
            hotkeys: hotkey::Status::default(),
            overlay: None,
            history: None,
            settings: None,
        });

        if !tray::add(hwnd) {
            DestroyWindow(hwnd);
            return Err("Shell_NotifyIconW(NIM_ADD) 失败：托盘区不可用".into());
        }

        let hot = hotkey::register(hwnd, &cfg);
        app().hotkeys = hot;

        // 开机自启每次启动都对齐一次：用户可能换了位置/重装，Run 项就失效了。
        // 重复写同一个值是幂等的；用户在「任务管理器→启动」里禁用由 StartupApproved
        // 掌管，我们不动那个键，所以不会和用户打架。
        autostart::set_enabled(cfg.autostart);
        refresh_tray_tip();

        // 首次运行把默认配置落盘，用户才有地方改（不然得自己去翻 %APPDATA%）
        if !existed {
            let _ = config::save(&cfg);
        }

        // 诊断用：把热键状态写进窗口标题（窗口从不显示，用户看不到）。
        // 冒烟脚本读它就知道三个热键各注册成功没，不必真按键。
        let diag = format!(
            "MySnipTool Tray Host [quick={} frozen={} history={}]",
            if hot.quick { "ok" } else { "fail" },
            if hot.frozen { "ok" } else { "fail" },
            if hot.history { "ok" } else { "fail" }
        );
        SetWindowTextW(hwnd, win_util::to_wide(&diag).as_ptr());

        // 开机补跑一次（关机期间没人清理），之后按保留时长的 1/4 周期跑
        run_cleanup();
        reset_cleanup_timer();

        // 启动完就把冷页换出：托盘常驻程序接下来长时间空闲，
        // 没必要把那十几 MB 的 DLL 代码页一直压在任务管理器里。
        // 注意这不是"释放"——提交内存不变，页进了 standby，用时软缺页回来。
        trim_working_set();

        let mut msg: MSG = mem::zeroed();
        while GetMessageW(&mut msg, ptr::null_mut(), 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }

        hotkey::unregister(hwnd);
        tray::del(hwnd);
        Ok(())
    }
}

fn main() {
    std::panic::set_hook(Box::new(|info| unsafe {
        win_util::error_box(
            "MySnipTool · panic",
            &format!("程序发生 panic，请把这段文字发给我：\n\n{info}"),
        );
    }));

    if let Err(e) = run() {
        unsafe { win_util::error_box(&format!("{APP_NAME} 启动失败"), &e) };
        std::process::exit(1);
    }
}
