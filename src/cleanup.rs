//! 每日清理：删除超过 N 小时的旧截图。
//!
//! 铁律（技术方案 §5.3）：**只读目录项元数据，图像一个字节都不进内存。**
//! 所以用 `FindFirstFileW` 一次遍历拿到「名字 + 属性 + ftLastWriteTime」，
//! 不 CreateFile、不 ReadFile、不 mmap。峰值增量与文件数量无关。
//!
//! 安全护栏：
//!   - 只匹配 `snip_*.png` / `snip_*.tmp`（白名单），用户把目录配错也不会误删；
//!   - 跳过目录和重解析点（junction/symlink），防止把删除引到别处；
//!   - 单个文件删除失败只计数，不中断整体流程。

use std::path::{Path, PathBuf};

use windows_sys::Win32::Foundation::{FILETIME, INVALID_HANDLE_VALUE};
use windows_sys::Win32::Storage::FileSystem::{
    DeleteFileW, FindClose, FindFirstFileW, FindNextFileW, FILE_ATTRIBUTE_DIRECTORY,
    FILE_ATTRIBUTE_REPARSE_POINT, WIN32_FIND_DATAW,
};
use windows_sys::Win32::System::SystemInformation::GetSystemTimeAsFileTime;

/// 只清理自己写出来的文件名，任何时候都不放开。
const PATTERNS: [&str; 2] = ["snip_*.png", "snip_*.tmp"];

#[derive(Debug, Default, Clone, Copy)]
pub struct SweepStat {
    pub deleted: u32,
    pub kept: u32,
    pub failed: u32,
}

fn to_u64(ft: &FILETIME) -> u64 {
    ((ft.dwHighDateTime as u64) << 32) | ft.dwLowDateTime as u64
}

fn wide(path: &Path) -> Vec<u16> {
    let mut v: Vec<u16> = path.to_string_lossy().encode_utf16().collect();
    v.push(0);
    v
}

fn file_name_of(data: &WIN32_FIND_DATAW) -> String {
    let end = data
        .cFileName
        .iter()
        .position(|&c| c == 0)
        .unwrap_or(data.cFileName.len());
    String::from_utf16_lossy(&data.cFileName[..end])
}

pub unsafe fn sweep(dir: &Path, retention_secs: u64) -> SweepStat {
    let mut stat = SweepStat::default();

    let mut now_ft: FILETIME = std::mem::zeroed();
    GetSystemTimeAsFileTime(&mut now_ft);
    let now = to_u64(&now_ft);
    // FILETIME 单位是 100ns
    let max_age = retention_secs.saturating_mul(10_000_000);

    // 主目录 + thumbs 子目录一起扫，否则孤儿的缩略图永远不会被删
    let dirs = [dir.to_path_buf(), dir.join("thumbs")];

    for d in dirs.iter() {
        for pattern in PATTERNS {
            let mut spec: Vec<u16> = d.join(pattern).to_string_lossy().encode_utf16().collect();
            spec.push(0);

            let mut data: WIN32_FIND_DATAW = std::mem::zeroed();
            let h = FindFirstFileW(spec.as_ptr(), &mut data);
            if h == INVALID_HANDLE_VALUE {
                continue; // 目录不存在或没有匹配项
            }

            loop {
                let attrs = data.dwFileAttributes;
                let is_dir = attrs & FILE_ATTRIBUTE_DIRECTORY != 0;
                let is_link = attrs & FILE_ATTRIBUTE_REPARSE_POINT != 0;

                if !is_dir && !is_link {
                    let age = now.saturating_sub(to_u64(&data.ftLastWriteTime));
                    if age > max_age {
                        let path: PathBuf = d.join(file_name_of(&data));
                        if DeleteFileW(wide(&path).as_ptr()) != 0 {
                            stat.deleted += 1;
                        } else {
                            stat.failed += 1; // 被占用/只读 → 下轮再试
                        }
                    } else {
                        stat.kept += 1;
                    }
                }

                if FindNextFileW(h, &mut data) == 0 {
                    break;
                }
            }
            FindClose(h);
        }
    }

    stat
}
