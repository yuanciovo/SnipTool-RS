//! 全局热键：组合的定义、解析、显示、注册。
//!
//! 组合用 `Combo { mods, vk }` 表示，`mods` 是 `MOD_*` 位（不含 `MOD_NOREPEAT`，
//! 注册时再补），这样配置文件和界面用的是同一个值，不用来回转换。
//!
//! 三条铁律：
//!   1. **必须至少含一个修饰键**。否则一个裸字母会把全局输入吃掉。
//!   2. 三个热键分别注册，**一个失败不影响其它两个**（真实世界里 Alt+A 很容易被微信/QQ 占）。
//!   3. 失败不弹窗、不抢占，把结果交给上层去改 tooltip / 菜单灰字。

use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyState, RegisterHotKey, UnregisterHotKey, MOD_ALT, MOD_CONTROL, MOD_NOREPEAT, MOD_SHIFT,
    MOD_WIN, VK_BACK, VK_CONTROL, VK_DELETE, VK_DOWN, VK_END, VK_ESCAPE, VK_F1, VK_HOME, VK_INSERT,
    VK_LEFT, VK_LMENU, VK_LSHIFT, VK_LWIN, VK_MENU, VK_NEXT, VK_PRIOR, VK_RCONTROL, VK_RETURN,
    VK_RIGHT, VK_RMENU, VK_RSHIFT, VK_RWIN, VK_SHIFT, VK_SPACE, VK_TAB, VK_UP,
};

pub const HK_QUICK: i32 = 1;
pub const HK_FROZEN: i32 = 2;
pub const HK_HISTORY: i32 = 3;

/// 一个热键组合。
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Combo {
    /// `MOD_CONTROL | MOD_ALT | ...`，不含 `MOD_NOREPEAT`
    pub mods: u32,
    pub vk: u32,
}

impl Combo {
    pub const fn new(mods: u32, vk: u32) -> Self {
        Self { mods, vk }
    }

    /// `Ctrl+Alt+A` → Combo。任何一处不合法就返回 None（配置写错了不静默采用）。
    pub fn parse(s: &str) -> Option<Self> {
        let mut mods = 0u32;
        let mut vk: Option<u32> = None;

        for raw in s.split('+') {
            let t = raw.trim();
            if t.is_empty() {
                continue;
            }
            let lower = t.to_ascii_lowercase();
            match lower.as_str() {
                "ctrl" | "control" => mods |= MOD_CONTROL,
                "alt" => mods |= MOD_ALT,
                "shift" => mods |= MOD_SHIFT,
                "win" | "super" | "meta" => mods |= MOD_WIN,
                _ => {
                    if vk.is_some() {
                        return None; // 两个键？不合法
                    }
                    vk = Some(key_from_name(&lower)?);
                }
            }
        }

        let vk = vk?;
        if mods == 0 {
            return None; // 没有修饰键的组合一律不接受
        }
        Some(Self { mods, vk })
    }

    /// 规范写法：Ctrl → Alt → Shift → Win → 键名。
    pub fn label(&self) -> String {
        let mut s = String::with_capacity(24);
        if self.mods & MOD_CONTROL != 0 {
            s.push_str("Ctrl+");
        }
        if self.mods & MOD_ALT != 0 {
            s.push_str("Alt+");
        }
        if self.mods & MOD_SHIFT != 0 {
            s.push_str("Shift+");
        }
        if self.mods & MOD_WIN != 0 {
            s.push_str("Win+");
        }
        s.push_str(&key_name(self.vk));
        s
    }
}

fn key_from_name(lower: &str) -> Option<u32> {
    let named = match lower {
        "space" => Some(VK_SPACE),
        "tab" => Some(VK_TAB),
        "enter" | "return" => Some(VK_RETURN),
        "esc" | "escape" => Some(VK_ESCAPE),
        "backspace" => Some(VK_BACK),
        "insert" | "ins" => Some(VK_INSERT),
        "delete" | "del" => Some(VK_DELETE),
        "home" => Some(VK_HOME),
        "end" => Some(VK_END),
        "pgup" | "pageup" => Some(VK_PRIOR),
        "pgdn" | "pagedown" => Some(VK_NEXT),
        "up" => Some(VK_UP),
        "down" => Some(VK_DOWN),
        "left" => Some(VK_LEFT),
        "right" => Some(VK_RIGHT),
        _ => None,
    };
    if let Some(vk) = named {
        return Some(vk as u32);
    }

    // F1..F24 是连续的
    if let Some(rest) = lower.strip_prefix('f') {
        if let Ok(n) = rest.parse::<u32>() {
            if (1..=24).contains(&n) {
                return Some(VK_F1 as u32 + n - 1);
            }
        }
    }

    // 单字符：字母 / 数字 / 极少数符号（VK 值与 ASCII 大写一致）
    let mut it = lower.chars();
    let c = it.next()?;
    if it.next().is_some() {
        return None;
    }
    let up = c.to_ascii_uppercase();
    if up.is_ascii_alphanumeric() {
        return Some(up as u32);
    }
    match c {
        '`' | '~' => Some(0xC0),
        '-' | '_' => Some(0xBD),
        '=' | '+' => Some(0xBB),
        '[' | '{' => Some(0xDB),
        ']' | '}' => Some(0xDD),
        ';' | ':' => Some(0xBA),
        '\'' | '"' => Some(0xDE),
        ',' | '<' => Some(0xBC),
        '.' | '>' => Some(0xBE),
        '/' | '?' => Some(0xBF),
        '\\' | '|' => Some(0xDC),
        _ => None,
    }
}

fn key_name(vk: u32) -> String {
    let named = match vk as u16 {
        v if v == VK_SPACE => Some("Space"),
        v if v == VK_TAB => Some("Tab"),
        v if v == VK_RETURN => Some("Enter"),
        v if v == VK_ESCAPE => Some("Esc"),
        v if v == VK_BACK => Some("Backspace"),
        v if v == VK_INSERT => Some("Insert"),
        v if v == VK_DELETE => Some("Delete"),
        v if v == VK_HOME => Some("Home"),
        v if v == VK_END => Some("End"),
        v if v == VK_PRIOR => Some("PgUp"),
        v if v == VK_NEXT => Some("PgDn"),
        v if v == VK_UP => Some("Up"),
        v if v == VK_DOWN => Some("Down"),
        v if v == VK_LEFT => Some("Left"),
        v if v == VK_RIGHT => Some("Right"),
        _ => None,
    };
    if let Some(n) = named {
        return n.to_string();
    }

    let f1 = VK_F1 as u32;
    if (f1..f1 + 24).contains(&vk) {
        return format!("F{}", vk - f1 + 1);
    }

    if (0x30..=0x39).contains(&vk) || (0x41..=0x5A).contains(&vk) {
        return char::from_u32(vk)
            .map(|c| c.to_string())
            .unwrap_or_default();
    }

    let sym = match vk {
        0xC0 => "`",
        0xBD => "-",
        0xBB => "=",
        0xDB => "[",
        0xDD => "]",
        0xBA => ";",
        0xDE => "'",
        0xBC => ",",
        0xBE => ".",
        0xBF => "/",
        0xDC => "\\",
        _ => return format!("VK{:02X}", vk),
    };
    sym.to_string()
}

/// 只按下了修饰键本身？这类按键不构成组合，要等真正的那个键。
pub fn is_modifier(vk: u32) -> bool {
    matches!(
        vk as u16,
        v if v == VK_SHIFT
            || v == VK_CONTROL
            || v == VK_MENU
            || v == VK_LSHIFT
            || v == VK_RSHIFT
            || v == VK_RCONTROL
            || v == VK_LMENU
            || v == VK_RMENU
            || v == VK_LWIN
            || v == VK_RWIN
    )
}

/// Shift 是否按住。Tab 走反序时用。
pub fn shift_down() -> bool {
    unsafe { GetKeyState(VK_SHIFT as i32) < 0 }
}

/// 此刻按住哪些修饰键（给录制控件用）。
pub fn mods_now() -> u32 {
    let mut m = 0u32;
    unsafe {
        if GetKeyState(VK_CONTROL as i32) < 0 {
            m |= MOD_CONTROL;
        }
        if GetKeyState(VK_MENU as i32) < 0 {
            m |= MOD_ALT;
        }
        if GetKeyState(VK_SHIFT as i32) < 0 {
            m |= MOD_SHIFT;
        }
        if GetKeyState(VK_LWIN as i32) < 0 || GetKeyState(VK_RWIN as i32) < 0 {
            m |= MOD_WIN;
        }
    }
    m
}

#[derive(Clone, Copy, Default)]
pub struct Status {
    pub quick: bool,
    pub frozen: bool,
    pub history: bool,
}

impl Status {
    pub fn all_ok(&self) -> bool {
        self.quick && self.frozen && self.history
    }
}

/// 按配置注册三个热键。失败返回 false —— 该组合大概率被别的程序占用。
unsafe fn one(hwnd: HWND, id: i32, c: Combo) -> bool {
    RegisterHotKey(hwnd, id, c.mods | MOD_NOREPEAT, c.vk) != 0
}

pub unsafe fn register(hwnd: HWND, cfg: &crate::config::Config) -> Status {
    Status {
        quick: one(hwnd, HK_QUICK, cfg.quick),
        frozen: one(hwnd, HK_FROZEN, cfg.frozen),
        history: one(hwnd, HK_HISTORY, cfg.history),
    }
}

pub unsafe fn unregister(hwnd: HWND) {
    UnregisterHotKey(hwnd, HK_QUICK);
    UnregisterHotKey(hwnd, HK_FROZEN);
    UnregisterHotKey(hwnd, HK_HISTORY);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn label_round_trips() {
        for s in [
            "Alt+A",
            "Alt+V",
            "Ctrl+Alt+A",
            "Ctrl+Shift+F9",
            "Ctrl+Alt+Shift+Win+F12",
            "Win+Space",
            "Alt+PgDn",
        ] {
            let combo = Combo::parse(s).unwrap_or_else(|| panic!("parse {s}"));
            assert_eq!(combo.label(), s, "round trip {s}");
        }
    }

    #[test]
    fn modifier_order_is_canonical() {
        // 用户写成什么顺序，输出都按 Ctrl→Alt→Shift→Win 归一
        assert_eq!(Combo::parse("Alt+Ctrl+A").unwrap().label(), "Ctrl+Alt+A");
        assert_eq!(
            Combo::parse("Shift+Ctrl+Alt+F9").unwrap().label(),
            "Ctrl+Alt+Shift+F9"
        );
        assert_eq!(Combo::parse("alt+v").unwrap().label(), "Alt+V");
    }

    #[test]
    fn rejects_invalid_combos() {
        assert!(Combo::parse("A").is_none(), "裸键不接受");
        assert!(Combo::parse("Ctrl+").is_none());
        assert!(Combo::parse("Ctrl+Alt").is_none());
        assert!(Combo::parse("Ctrl+A+B").is_none());
        assert!(Combo::parse("").is_none());
        assert!(Combo::parse("Alt+Nope").is_none());
    }

    #[test]
    fn fkeys_map_to_contiguous_vks() {
        assert_eq!(Combo::parse("Ctrl+F1").unwrap().vk, VK_F1 as u32);
        assert_eq!(Combo::parse("Ctrl+F9").unwrap().vk, VK_F1 as u32 + 8);
        assert_eq!(Combo::parse("Ctrl+F24").unwrap().vk, VK_F1 as u32 + 23);
        assert!(Combo::parse("Ctrl+F25").is_none());
    }

    #[test]
    fn modifier_keys_are_not_combos() {
        assert!(is_modifier(VK_CONTROL as u32));
        assert!(is_modifier(VK_LWIN as u32));
        assert!(!is_modifier('A' as u32));
    }
}
