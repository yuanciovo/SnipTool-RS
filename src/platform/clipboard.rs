//! 剪贴板：同时写 CF_DIB 和 CF_PNG。
//!
//! 为什么写两份（技术方案 §9 #4）：
//!   - CF_DIB 兼容性最好，微信/QQ/Word/画图都认；
//!   - 但部分应用对 32bpp BI_RGB 的 alpha 通道处理不一致，粘出来会发黑/透明；
//!   - 所以再写一份注册格式 "PNG"（Win10+ 的 Chrome/Office/新版画图都优先用它）。
//!
//! 关键约定：`SetClipboardData` 成功后内存归系统所有，**绝不能**再 GlobalFree，
//! 失败才由我们自己释放。

use std::ptr;
use std::time::Duration;

use windows_sys::Win32::Foundation::{GlobalFree, HANDLE};
use windows_sys::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, OpenClipboard, RegisterClipboardFormatW, SetClipboardData,
};
use windows_sys::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};

use crate::platform::win_util::to_wide;

const CF_DIB: u32 = 8;

unsafe fn alloc_global(bytes: &[u8]) -> Option<HANDLE> {
    let h = GlobalAlloc(GMEM_MOVEABLE, bytes.len());
    if h.is_null() {
        return None;
    }
    let p = GlobalLock(h);
    if p.is_null() {
        GlobalFree(h);
        return None;
    }
    ptr::copy_nonoverlapping(bytes.as_ptr(), p as *mut u8, bytes.len());
    GlobalUnlock(h);
    Some(h)
}

/// 构造 CF_DIB：40 字节 BITMAPINFOHEADER + bottom-up 32bpp 像素。
/// alpha 一律写 0xFF —— GDI 抓屏留下的 alpha 是 0，直接塞进去有些应用会当全透明。
fn build_dib(bgra_topdown: &[u8], w: i32, h: i32) -> Vec<u8> {
    let wi = w as usize;
    let hi = h as usize;
    let stride = wi * 4;
    let image_size = stride * hi;

    let mut v = Vec::with_capacity(40 + image_size);
    v.extend_from_slice(&40u32.to_le_bytes()); // biSize
    v.extend_from_slice(&w.to_le_bytes()); // biWidth
    v.extend_from_slice(&h.to_le_bytes()); // biHeight 正数 = bottom-up
    v.extend_from_slice(&1u16.to_le_bytes()); // biPlanes
    v.extend_from_slice(&32u16.to_le_bytes()); // biBitCount
    v.extend_from_slice(&0u32.to_le_bytes()); // biCompression = BI_RGB
    v.extend_from_slice(&(image_size as u32).to_le_bytes());
    v.extend_from_slice(&2835i32.to_le_bytes()); // 72 DPI
    v.extend_from_slice(&2835i32.to_le_bytes());
    v.extend_from_slice(&0u32.to_le_bytes());
    v.extend_from_slice(&0u32.to_le_bytes());

    for y in (0..hi).rev() {
        let row = &bgra_topdown[y * stride..(y + 1) * stride];
        let start = v.len();
        v.extend_from_slice(row);
        for x in 0..wi {
            v[start + x * 4 + 3] = 0xFF;
        }
    }
    v
}

/// 打开剪贴板并重试。别的程序（输入法、剪贴板管理器）经常短暂占用它。
unsafe fn open_with_retry() -> bool {
    for _ in 0..5 {
        if OpenClipboard(ptr::null_mut()) != 0 {
            return true;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    false
}

/// 把截图放进剪贴板。返回 CF_DIB 是否写成功（CF_PNG 属锦上添花，失败不算整体失败）。
pub unsafe fn set_image(bgra_topdown: &[u8], w: i32, h: i32, png: &[u8]) -> bool {
    let dib = build_dib(bgra_topdown, w, h);

    if !open_with_retry() {
        return false;
    }

    EmptyClipboard();

    let mut ok = false;
    if let Some(h) = alloc_global(&dib) {
        if SetClipboardData(CF_DIB, h).is_null() {
            GlobalFree(h);
        } else {
            ok = true;
        }
    }

    if !png.is_empty() {
        let fmt = RegisterClipboardFormatW(to_wide("PNG").as_ptr());
        if fmt != 0 {
            if let Some(h) = alloc_global(png) {
                if SetClipboardData(fmt, h).is_null() {
                    GlobalFree(h);
                }
            }
        }
    }

    CloseClipboard();
    ok
}
