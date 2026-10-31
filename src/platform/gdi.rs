//! GDI 资源的 RAII 封装。
//!
//! windows-sys 是裸 C 绑定，没有 Drop 保护 —— 句柄泄漏必须靠这里兜住。
//! Spike 2 只有 Dib 一个资源；Spike 3 起会补 Pen/Font/DC 的 wrapper。

use std::mem::{self, size_of};
use std::ptr;

use windows_sys::Win32::Foundation::RECT;
use windows_sys::Win32::Graphics::Gdi::{
    BitBlt, CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, GetDC, ReleaseDC,
    SelectObject, SetBrushOrgEx, SetStretchBltMode, StretchBlt, BITMAPINFO, BITMAPINFOHEADER,
    BI_RGB, CAPTUREBLT, DIB_RGB_COLORS, HALFTONE, HBITMAP, HDC, HGDIOBJ, SRCCOPY,
};

/// 一块 32bpp top-down 的设备无关位图，附带一个已选入位图的内存 DC。
///
/// 生命周期即内存峰值：`drop` 之后 GDI 对象数必须回到基线。
pub struct Dib {
    hdc: HDC,
    hbm: HBITMAP,
    old: HGDIOBJ,
    bits: *mut u8,
    pub w: i32,
    pub h: i32,
}

impl Dib {
    /// 从屏幕 DC 抓取一块矩形（屏幕坐标）。
    pub unsafe fn capture(rect: &RECT) -> Option<Self> {
        let w = rect.right - rect.left;
        let h = rect.bottom - rect.top;
        if w <= 0 || h <= 0 {
            return None;
        }

        let screen = GetDC(ptr::null_mut());
        if screen.is_null() {
            return None;
        }

        let hdc = CreateCompatibleDC(screen);
        if hdc.is_null() {
            ReleaseDC(ptr::null_mut(), screen);
            return None;
        }

        let mut bi: BITMAPINFO = mem::zeroed();
        bi.bmiHeader.biSize = size_of::<BITMAPINFOHEADER>() as u32;
        bi.bmiHeader.biWidth = w;
        bi.bmiHeader.biHeight = -h; // 负数 = top-down，省掉写文件时的行翻转
        bi.bmiHeader.biPlanes = 1;
        bi.bmiHeader.biBitCount = 32;
        bi.bmiHeader.biCompression = BI_RGB;

        let mut bits: *mut core::ffi::c_void = ptr::null_mut();
        let hbm = CreateDIBSection(hdc, &bi, DIB_RGB_COLORS, &mut bits, ptr::null_mut(), 0);
        if hbm.is_null() || bits.is_null() {
            DeleteDC(hdc);
            ReleaseDC(ptr::null_mut(), screen);
            return None;
        }

        let old = SelectObject(hdc, hbm);
        // CAPTUREBLT：连分层窗口一起抓，否则镂空区域会拿到过期内容
        BitBlt(
            hdc,
            0,
            0,
            w,
            h,
            screen,
            rect.left,
            rect.top,
            SRCCOPY | CAPTUREBLT,
        );
        ReleaseDC(ptr::null_mut(), screen);

        Some(Dib {
            hdc,
            hbm,
            old,
            bits: bits as *mut u8,
            w,
            h,
        })
    }

    /// 把 RGB 打包数据填进一块 32bpp DIB（历史面板解码原图后用）。
    pub unsafe fn from_rgb(rgb: &[u8], w: i32, h: i32) -> Option<Self> {
        if w <= 0 || h <= 0 || rgb.len() < (w as usize) * (h as usize) * 3 {
            return None;
        }
        let screen = GetDC(ptr::null_mut());
        let hdc = CreateCompatibleDC(screen);
        if hdc.is_null() {
            ReleaseDC(ptr::null_mut(), screen);
            return None;
        }
        let mut bi: BITMAPINFO = mem::zeroed();
        bi.bmiHeader.biSize = size_of::<BITMAPINFOHEADER>() as u32;
        bi.bmiHeader.biWidth = w;
        bi.bmiHeader.biHeight = -h;
        bi.bmiHeader.biPlanes = 1;
        bi.bmiHeader.biBitCount = 32;
        bi.bmiHeader.biCompression = BI_RGB;

        let mut bits: *mut core::ffi::c_void = ptr::null_mut();
        let hbm = CreateDIBSection(hdc, &bi, DIB_RGB_COLORS, &mut bits, ptr::null_mut(), 0);
        if hbm.is_null() || bits.is_null() {
            DeleteDC(hdc);
            ReleaseDC(ptr::null_mut(), screen);
            return None;
        }
        let old = SelectObject(hdc, hbm);

        let dst = bits as *mut u8;
        let n = (w as usize) * (h as usize);
        for i in 0..n {
            *dst.add(i * 4) = rgb[i * 3 + 2];
            *dst.add(i * 4 + 1) = rgb[i * 3 + 1];
            *dst.add(i * 4 + 2) = rgb[i * 3];
            *dst.add(i * 4 + 3) = 0xFF;
        }
        ReleaseDC(ptr::null_mut(), screen);

        Some(Dib {
            hdc,
            hbm,
            old,
            bits: dst,
            w,
            h,
        })
    }

    /// 等比缩放到 w×h（缩略图用，HALFTONE 让缩小不糊）。
    pub unsafe fn scaled(&self, w: i32, h: i32) -> Option<Self> {
        if w <= 0 || h <= 0 {
            return None;
        }
        let hdc = CreateCompatibleDC(self.hdc);
        if hdc.is_null() {
            return None;
        }
        let mut bi: BITMAPINFO = mem::zeroed();
        bi.bmiHeader.biSize = size_of::<BITMAPINFOHEADER>() as u32;
        bi.bmiHeader.biWidth = w;
        bi.bmiHeader.biHeight = -h;
        bi.bmiHeader.biPlanes = 1;
        bi.bmiHeader.biBitCount = 32;
        bi.bmiHeader.biCompression = BI_RGB;

        let mut bits: *mut core::ffi::c_void = ptr::null_mut();
        let hbm = CreateDIBSection(hdc, &bi, DIB_RGB_COLORS, &mut bits, ptr::null_mut(), 0);
        if hbm.is_null() || bits.is_null() {
            DeleteDC(hdc);
            return None;
        }
        let old = SelectObject(hdc, hbm);
        SetStretchBltMode(hdc, HALFTONE);
        SetBrushOrgEx(hdc, 0, 0, ptr::null_mut());
        StretchBlt(hdc, 0, 0, w, h, self.hdc, 0, 0, self.w, self.h, SRCCOPY);

        Some(Dib {
            hdc,
            hbm,
            old,
            bits: bits as *mut u8,
            w,
            h,
        })
    }

    /// 从另一个 DC 的指定矩形拷贝出一块新位图（冻结模式下抓选区用）。
    pub unsafe fn from_dc(src: HDC, rect: &RECT) -> Option<Self> {
        let w = rect.right - rect.left;
        let h = rect.bottom - rect.top;
        if w <= 0 || h <= 0 {
            return None;
        }
        let hdc = CreateCompatibleDC(src);
        if hdc.is_null() {
            return None;
        }
        let mut bi: BITMAPINFO = mem::zeroed();
        bi.bmiHeader.biSize = size_of::<BITMAPINFOHEADER>() as u32;
        bi.bmiHeader.biWidth = w;
        bi.bmiHeader.biHeight = -h;
        bi.bmiHeader.biPlanes = 1;
        bi.bmiHeader.biBitCount = 32;
        bi.bmiHeader.biCompression = BI_RGB;

        let mut bits: *mut core::ffi::c_void = ptr::null_mut();
        let hbm = CreateDIBSection(hdc, &bi, DIB_RGB_COLORS, &mut bits, ptr::null_mut(), 0);
        if hbm.is_null() || bits.is_null() {
            DeleteDC(hdc);
            return None;
        }
        let old = SelectObject(hdc, hbm);
        BitBlt(hdc, 0, 0, w, h, src, rect.left, rect.top, SRCCOPY);

        Some(Dib {
            hdc,
            hbm,
            old,
            bits: bits as *mut u8,
            w,
            h,
        })
    }

    /// 已选入位图的内存 DC，供 BitBlt 贴回屏幕。
    pub fn dc(&self) -> HDC {
        self.hdc
    }

    /// BGRA 像素，按行连续，top-down。
    pub fn pixels(&self) -> &[u8] {
        unsafe { std::slice::from_raw_parts(self.bits, (self.w as usize) * (self.h as usize) * 4) }
    }
}

impl Drop for Dib {
    fn drop(&mut self) {
        unsafe {
            SelectObject(self.hdc, self.old);
            DeleteObject(self.hbm);
            DeleteDC(self.hdc);
        }
    }
}
