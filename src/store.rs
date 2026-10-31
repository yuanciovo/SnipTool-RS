//! 临时目录、文件命名、PNG 落盘。
//!
//! 命名规则 `snip_YYYYMMDD_HHMMSS_mmm.png` 有两个用途：
//!   1. 清理时用 `FindFirstFileW("snip_*.png")` 就能天然做白名单，不会误删别人的文件；
//!   2. 文件名本身按时间可排序，历史面板（Spike 4）可以直接按名字倒序。

use std::fs;
use std::io;
use std::mem;
use std::path::PathBuf;

use windows_sys::Win32::Foundation::SYSTEMTIME;
use windows_sys::Win32::System::SystemInformation::GetLocalTime;

pub const DIR_NAME: &str = "MySnipTool";
pub const FILE_PREFIX: &str = "snip_";

pub fn temp_dir() -> PathBuf {
    let mut p = std::env::temp_dir();
    p.push(DIR_NAME);
    p
}

pub fn ensure_dir() -> io::Result<PathBuf> {
    let d = temp_dir();
    fs::create_dir_all(&d)?;
    Ok(d)
}

/// 本地时间戳。用 Win32 而不是引 chrono/time —— 只为格式化一个文件名不值得加依赖。
fn new_name() -> String {
    let mut st: SYSTEMTIME = unsafe { mem::zeroed() };
    unsafe { GetLocalTime(&mut st) };
    format!(
        "{}{:04}{:02}{:02}_{:02}{:02}{:02}_{:03}.png",
        FILE_PREFIX,
        st.wYear,
        st.wMonth,
        st.wDay,
        st.wHour,
        st.wMinute,
        st.wSecond,
        st.wMilliseconds
    )
}

/// DIB 的 BGRA(top-down) → PNG 用的 RGB。丢掉 alpha：截图不透明，
/// 24bpp 比 32bpp 的 PNG 小 25% 左右，编码也快一点。
pub fn bgra_to_rgb(bgra: &[u8]) -> Vec<u8> {
    let px = bgra.len() / 4;
    let mut out = Vec::with_capacity(px * 3);
    for i in 0..px {
        let o = i * 4;
        out.push(bgra[o + 2]); // R
        out.push(bgra[o + 1]); // G
        out.push(bgra[o]); // B
    }
    out
}

/// 先写 `.tmp` 再原子重命名。
/// 这样清理线程绝不会删到「写了一半」的 PNG，历史面板也不会列到它。
pub fn save_png(rgb: &[u8], w: i32, h: i32) -> io::Result<PathBuf> {
    let dir = ensure_dir()?;
    let name = new_name();
    let final_path = dir.join(&name);
    let tmp_path = dir.join(format!("{}.tmp", name));

    image::save_buffer_with_format(
        &tmp_path,
        rgb,
        w as u32,
        h as u32,
        image::ExtendedColorType::Rgb8,
        image::ImageFormat::Png,
    )
    .map_err(io::Error::other)?;

    fs::rename(&tmp_path, &final_path)?;
    Ok(final_path)
}

// ---------------------------------------------------------------------------
// 缩略图与历史列表（Spike 4）
// ---------------------------------------------------------------------------

use crate::platform::gdi::Dib;

pub const THUMB_W: i32 = 256;
pub const THUMB_H: i32 = 144;

pub fn thumbs_dir() -> PathBuf {
    let mut p = temp_dir();
    p.push("thumbs");
    p
}

/// 生成缩略图并落盘：`thumbs/<同名>.png`。
///
/// 关键：**在截图那一刻做**，此时全分辨率 DIB 本来就在手里，StretchBlt 一次
/// 只要几毫秒；等在面板里再缩放，就得先把原图整块解码（4K 就是 33MB）。
pub unsafe fn save_thumb(src: &Dib, stem: &str) -> Option<PathBuf> {
    let dir = thumbs_dir();
    fs::create_dir_all(&dir).ok()?;

    // 等比缩放到 256×144 的框内
    let sw = src.w.max(1);
    let sh = src.h.max(1);
    let k = (THUMB_W as f32 / sw as f32).min(THUMB_H as f32 / sh as f32);
    let tw = ((sw as f32 * k).round() as i32).max(1);
    let th = ((sh as f32 * k).round() as i32).max(1);

    let small = src.scaled(tw, th)?;
    let rgb = bgra_to_rgb(small.pixels());
    let path = dir.join(format!("{}.png", stem));

    image::save_buffer_with_format(
        &path,
        &rgb,
        tw as u32,
        th as u32,
        image::ExtendedColorType::Rgb8,
        image::ImageFormat::Png,
    )
    .ok()?;

    Some(path)
}

/// 历史列表里的一项。只读文件元数据 + PNG 头，不解码像素。
pub struct Entry {
    pub name: String,
    pub path: PathBuf,
    pub thumb: PathBuf,
    pub w: u32,
    pub h: u32,
    pub kb: u64,
}

impl Entry {
    /// `snip_20261031_123859_984.png` → `10-31 12:38:59`
    pub fn time_label(&self) -> String {
        let n = &self.name;
        if n.len() >= 21 && n.starts_with(FILE_PREFIX) {
            let d = &n[5..13];
            let t = &n[14..20];
            format!(
                "{}-{} {}:{}:{}",
                &d[4..6],
                &d[6..8],
                &t[0..2],
                &t[2..4],
                &t[4..6]
            )
        } else {
            n.clone()
        }
    }
}

/// 最近 n 张。按文件名倒序即可（文件名本身就是时间戳）。
pub fn list_recent(n: usize) -> Vec<Entry> {
    let dir = temp_dir();
    let Ok(rd) = fs::read_dir(&dir) else {
        return Vec::new();
    };

    let mut names: Vec<String> = rd
        .filter_map(|e| e.ok())
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|n| n.starts_with(FILE_PREFIX) && n.ends_with(".png"))
        .collect();
    names.sort();
    names.reverse();
    names.truncate(n);

    let tdir = thumbs_dir();
    let mut out = Vec::with_capacity(names.len());
    for name in names {
        let path = dir.join(&name);
        let kb = fs::metadata(&path).map(|m| m.len() / 1024).unwrap_or(0);
        // image_dimensions 只读 PNG 头，不解码像素
        let (w, h) = image::image_dimensions(&path).unwrap_or((0, 0));
        let stem = name.trim_end_matches(".png");
        out.push(Entry {
            thumb: tdir.join(format!("{}.png", stem)),
            name,
            path,
            w,
            h,
            kb,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &str) -> Entry {
        Entry {
            name: name.to_string(),
            path: PathBuf::new(),
            thumb: PathBuf::new(),
            w: 0,
            h: 0,
            kb: 0,
        }
    }

    #[test]
    fn time_label_from_filename() {
        assert_eq!(
            entry("snip_20261031_123859_984.png").time_label(),
            "10-31 12:38:59"
        );
        assert_eq!(
            entry("snip_20260101_000000_001.png").time_label(),
            "01-01 00:00:00"
        );
    }

    #[test]
    fn time_label_falls_back_to_name() {
        assert_eq!(entry("weird.png").time_label(), "weird.png");
    }

    #[test]
    fn bgra_to_rgb_swaps_channels() {
        // 一个像素：B=3 G=2 R=1 A=255 → RGB 应是 1,2,3
        let rgb = bgra_to_rgb(&[3, 2, 1, 255, 30, 20, 10, 255]);
        assert_eq!(rgb, vec![1, 2, 3, 10, 20, 30]);
    }
}
