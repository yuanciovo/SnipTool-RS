//! 托盘图标：增删、右键菜单、气泡提示。

use std::mem::{self, size_of};
use std::ptr;

use windows_sys::Win32::Foundation::{HWND, POINT};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Registry::{
    RegCloseKey, RegOpenKeyExW, RegQueryValueExW, HKEY, HKEY_CURRENT_USER, KEY_QUERY_VALUE,
};
use windows_sys::Win32::UI::Shell::{
    Shell_NotifyIconW, NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NIM_MODIFY,
    NOTIFYICONDATAW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::*;

use super::win_util::to_wide;
use crate::{
    IDM_CAPTURE, IDM_CAPTURE_FROZEN, IDM_EXIT, IDM_HISTORY, IDM_OPEN_FOLDER, IDM_OPEN_LATEST,
    IDM_SETTINGS,
};

pub const TRAY_ICON_ID: u32 = 1;

// 资源 id，对应 assets/app.rc
const IDI_TRAY_DARK: usize = 2;
const IDI_TRAY_LIGHT: usize = 3;

/// 自己 LoadImage 出来的图标要 DestroyIcon；LoadIcon 的共享图标不能删。
static mut TRAY_ICON: HICON = ptr::null_mut();

/// 任务栏是浅色吗？（决定用深色还是白色字形）
///
/// Win11 深色任务栏上放深色图标等于隐形，所以托盘必须准备两套单色版，
/// 启动时按系统主题挑一个。读不到就按浅色处理（深色字形，通用性最好）。
unsafe fn taskbar_is_light() -> bool {
    const PATH: &str = r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize";
    let mut key: HKEY = ptr::null_mut();
    if RegOpenKeyExW(
        HKEY_CURRENT_USER,
        to_wide(PATH).as_ptr(),
        0,
        KEY_QUERY_VALUE,
        &mut key,
    ) != 0
    {
        return true;
    }
    let mut value: u32 = 1;
    let mut cb: u32 = 4;
    let status = RegQueryValueExW(
        key,
        to_wide("SystemUsesLightTheme").as_ptr(),
        ptr::null(),
        ptr::null_mut(),
        &mut value as *mut u32 as *mut u8,
        &mut cb,
    );
    RegCloseKey(key);
    status != 0 || value != 0
}

unsafe fn load_tray_icon() -> HICON {
    let id = if taskbar_is_light() {
        IDI_TRAY_DARK
    } else {
        IDI_TRAY_LIGHT
    };
    let cx = GetSystemMetrics(SM_CXSMICON);
    let h = LoadImageW(
        GetModuleHandleW(ptr::null()),
        id as *const u16,
        IMAGE_ICON,
        cx,
        cx,
        LR_DEFAULTCOLOR,
    ) as HICON;
    if !h.is_null() {
        return h;
    }
    // 资源没编进去（缺 SDK）时退回系统图标，至少还能用
    LoadIconW(ptr::null_mut(), IDI_APPLICATION)
}
/// 托盘回调消息，wnd_proc 里匹配用。
pub const WM_TRAY_CALLBACK: u32 = WM_APP + 1;

fn base_nid(hwnd: HWND) -> NOTIFYICONDATAW {
    let mut nid: NOTIFYICONDATAW = unsafe { mem::zeroed() };
    nid.cbSize = size_of::<NOTIFYICONDATAW>() as u32;
    nid.hWnd = hwnd;
    nid.uID = TRAY_ICON_ID;
    nid
}

pub unsafe fn add(hwnd: HWND) -> bool {
    let mut nid = base_nid(hwnd);
    nid.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
    nid.uCallbackMessage = WM_TRAY_CALLBACK;
    let icon = load_tray_icon();
    nid.hIcon = icon;

    let tip = to_wide(&format!(
        "MySnipTool · {} 截图",
        crate::app().config.quick.label()
    ));
    let n = tip.len().min(nid.szTip.len());
    nid.szTip[..n].copy_from_slice(&tip[..n]);

    let ok = Shell_NotifyIconW(NIM_ADD, &nid) != 0;
    if ok {
        TRAY_ICON = icon;
    }
    ok
}

pub unsafe fn del(hwnd: HWND) {
    let nid = base_nid(hwnd);
    Shell_NotifyIconW(NIM_DELETE, &nid);
    if !TRAY_ICON.is_null() {
        DestroyIcon(TRAY_ICON);
        TRAY_ICON = ptr::null_mut();
    }
}

/// 改托盘提示文字（热键注册失败时用，替代以前的气泡弹窗）。
pub unsafe fn set_tip(hwnd: HWND, text: &str) {
    let mut nid = base_nid(hwnd);
    nid.uFlags = NIF_TIP;
    let tip = to_wide(text);
    let n = tip.len().min(nid.szTip.len());
    nid.szTip[..n].copy_from_slice(&tip[..n]);
    Shell_NotifyIconW(NIM_MODIFY, &nid);
}

pub unsafe fn menu(hwnd: HWND) {
    let m = CreatePopupMenu();
    if m.is_null() {
        return;
    }
    let cfg = crate::app().config.clone();
    let hot = crate::app().hotkeys;

    let quick = to_wide(&format!("截图（动态）    {}", cfg.quick.label()));
    AppendMenuW(m, MF_STRING, IDM_CAPTURE, quick.as_ptr());

    let frozen = to_wide(&format!("截图（冻结）    {}", cfg.frozen.label()));
    AppendMenuW(m, MF_STRING, IDM_CAPTURE_FROZEN, frozen.as_ptr());

    AppendMenuW(m, MF_SEPARATOR, 0, ptr::null());

    let hist = to_wide(&format!("历史面板    {}", cfg.history.label()));
    AppendMenuW(m, MF_STRING, IDM_HISTORY, hist.as_ptr());

    let latest = to_wide("打开最新截图");
    AppendMenuW(m, MF_STRING, IDM_OPEN_LATEST, latest.as_ptr());

    let folder = to_wide("打开临时文件夹");
    AppendMenuW(m, MF_STRING, IDM_OPEN_FOLDER, folder.as_ptr());

    AppendMenuW(m, MF_SEPARATOR, 0, ptr::null());

    let settings = to_wide("设置…");
    AppendMenuW(m, MF_STRING, IDM_SETTINGS, settings.as_ptr());

    let exit_label = to_wide("退出(E)");
    AppendMenuW(m, MF_STRING, IDM_EXIT, exit_label.as_ptr());

    if !hot.all_ok() {
        let mut names: Vec<String> = Vec::new();
        if !hot.quick {
            names.push(cfg.quick.label());
        }
        if !hot.frozen {
            names.push(cfg.frozen.label());
        }
        if !hot.history {
            names.push(cfg.history.label());
        }
        let warn = to_wide(&format!(
            "（{} 未注册，可能被占用 → 设置里改绑）",
            names.join(" / ")
        ));
        AppendMenuW(m, MF_STRING | MF_DISABLED | MF_GRAYED, 0, warn.as_ptr());
    }

    let mut pt = POINT { x: 0, y: 0 };
    GetCursorPos(&mut pt);

    // 必须成对出现，否则菜单点空白处不消失（技术方案 §9 #3）。
    SetForegroundWindow(hwnd);
    TrackPopupMenu(
        m,
        TPM_RIGHTBUTTON | TPM_BOTTOMALIGN,
        pt.x,
        pt.y,
        0,
        hwnd,
        ptr::null(),
    );
    PostMessageW(hwnd, WM_NULL, 0, 0);

    DestroyMenu(m);
}
