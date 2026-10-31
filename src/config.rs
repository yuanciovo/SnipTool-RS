//! 配置持久化。
//!
//! 文件位置：`%APPDATA%\MySnipTool\config.ini`
//!
//! 为什么不是 JSON：只有 5 个字段。手写一个 JSON 解析器要处理转义/嵌套，引 serde 又要
//! 多 100KB 和一堆编译时间 —— 都不值。`key=value` 每行一条，用户拿记事本就能改，
//! 解析器也只有 20 行，且**任何一行写错都只回退那一项**，不会让程序起不来。

use std::fs;
use std::path::PathBuf;

use crate::platform::hotkey::Combo;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{MOD_ALT, MOD_CONTROL};

#[derive(Clone)]
pub struct Config {
    pub quick: Combo,
    pub frozen: Combo,
    pub history: Combo,
    /// 截图保留时长（小时），1..=720
    pub retention_hours: u32,
    /// 开机自启（写 HKCU\...\Run）
    pub autostart: bool,
}

pub const DEFAULT_QUICK: Combo = Combo::new(MOD_ALT, 'A' as u32);
pub const DEFAULT_FROZEN: Combo = Combo::new(MOD_CONTROL | MOD_ALT, 'A' as u32);
pub const DEFAULT_HISTORY: Combo = Combo::new(MOD_ALT, 'V' as u32);

impl Default for Config {
    fn default() -> Self {
        Self {
            quick: DEFAULT_QUICK,
            frozen: DEFAULT_FROZEN,
            history: DEFAULT_HISTORY,
            retention_hours: 24,
            autostart: true,
        }
    }
}

pub fn config_path() -> PathBuf {
    let base = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    base.join("MySnipTool").join("config.ini")
}

/// 返回 `(配置, 文件是否已存在)`。文件不存在时给默认值，由调用方决定要不要落盘。
pub fn load() -> (Config, bool) {
    let path = config_path();
    let Ok(text) = fs::read_to_string(&path) else {
        return (Config::default(), false);
    };
    (parse(&text), true)
}

/// 纯函数：文本 → 配置。**任何一行写错只回退那一项**，不会让程序起不来。
pub fn parse(text: &str) -> Config {
    let mut c = Config::default();
    // 文件带 UTF-8 BOM（见 save），读回来先剥掉，否则第一行会被当成未知键
    let text = text.trim_start_matches('\u{feff}');
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        let Some((k, v)) = line.split_once('=') else {
            continue;
        };
        let key = k.trim().to_ascii_lowercase();
        let val = v.trim();
        match key.as_str() {
            "quick" => {
                if let Some(x) = Combo::parse(val) {
                    c.quick = x;
                }
            }
            "frozen" => {
                if let Some(x) = Combo::parse(val) {
                    c.frozen = x;
                }
            }
            "history" => {
                if let Some(x) = Combo::parse(val) {
                    c.history = x;
                }
            }
            "retention_hours" => {
                if let Ok(n) = val.parse::<u32>() {
                    c.retention_hours = n.clamp(1, 720);
                }
            }
            "autostart" => {
                c.autostart = matches!(
                    val.to_ascii_lowercase().as_str(),
                    "1" | "true" | "yes" | "on"
                )
            }
            _ => {}
        }
    }
    c
}

/// 先写 `.tmp` 再原子替换：写到一半断电也不会留下半个配置文件。
pub fn save(c: &Config) -> std::io::Result<()> {
    let path = config_path();
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let body = render(c);

    let tmp = path.with_extension("ini.tmp");
    fs::write(&tmp, body.as_bytes())?;
    fs::rename(&tmp, &path)?;
    Ok(())
}

/// 纯函数：配置 → 文本。
pub fn render(c: &Config) -> String {
    format!(
        "\u{feff}# MySnipTool 配置\n\
         # 热键写法：Ctrl+Alt+A / Alt+V / F9 …… 必须至少含一个修饰键（Ctrl/Alt/Shift/Win）\n\
         # 改完保存，从托盘菜单「设置」里点保存会立即生效；手改文件则重启程序生效\n\
         quick={}\n\
         frozen={}\n\
         history={}\n\
         retention_hours={}\n\
         autostart={}\n",
        c.quick.label(),
        c.frozen.label(),
        c.history.label(),
        c.retention_hours,
        if c.autostart { 1 } else { 0 },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::MOD_SHIFT;

    #[test]
    fn render_then_parse_round_trips() {
        let c = Config {
            quick: Combo::new(MOD_CONTROL | MOD_ALT, 'Q' as u32),
            history: Combo::new(MOD_ALT | MOD_SHIFT, 'V' as u32),
            retention_hours: 72,
            autostart: false,
            ..Config::default()
        };

        let back = parse(&render(&c));
        assert_eq!(back.quick.label(), "Ctrl+Alt+Q");
        assert_eq!(back.history.label(), "Alt+Shift+V");
        assert_eq!(back.retention_hours, 72);
        assert!(!back.autostart);
    }

    #[test]
    fn broken_lines_fall_back_one_by_one() {
        // 热键写错 → 只回退那一项，其余照常
        let c = parse("quick=not-a-hotkey\nfrozen=Ctrl+Shift+F9\nretention_hours=abc\n");
        assert_eq!(c.quick.label(), DEFAULT_QUICK.label());
        assert_eq!(c.frozen.label(), "Ctrl+Shift+F9");
        assert_eq!(c.retention_hours, 24);
    }

    #[test]
    fn retention_is_clamped() {
        assert_eq!(parse("retention_hours=0").retention_hours, 1);
        assert_eq!(parse("retention_hours=99999").retention_hours, 720);
    }

    #[test]
    fn bom_comments_and_unknown_keys_are_ignored() {
        let c = parse(
            "\u{feff}# MySnipTool\n; another style\nnonsense\nfoo=bar\nfrozen=Ctrl+Shift+F9\n",
        );
        assert_eq!(c.frozen.label(), "Ctrl+Shift+F9");
        assert_eq!(c.quick.label(), DEFAULT_QUICK.label());
    }
}
