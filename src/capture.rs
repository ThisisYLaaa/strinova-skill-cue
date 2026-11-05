//! Windows 截屏：GDI BitBlt。只抓很小的 HUD 区域，单次 1~3 ms。

use std::ffi::c_void;

use windows::Win32::Graphics::Gdi::{
    BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject, GetDC, GetDIBits,
    ReleaseDC, SelectObject, BITMAPINFO, BITMAPINFOHEADER, DIB_RGB_COLORS, HBITMAP, HDC, HGDIOBJ,
    ROP_CODE,
};
use windows::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SM_CXSCREEN, SM_CYSCREEN};

/// SRCCOPY | CAPTUREBLT：普通拷贝 + 连分层窗口一起抓
const ROP: u32 = 0x00CC_0020 | 0x4000_0000;

#[derive(Clone, Debug, Default)]
pub struct Frame {
    pub w: u32,
    pub h: u32,
    /// RGBA8 行优先
    pub rgba: Vec<u8>,
}

pub fn screen_size() -> (u32, u32) {
    unsafe {
        let w = GetSystemMetrics(SM_CXSCREEN).max(1) as u32;
        let h = GetSystemMetrics(SM_CYSCREEN).max(1) as u32;
        (w, h)
    }
}

pub struct Capturer {
    mem: HDC,
    bmp: HBITMAP,
    width: i32,
    height: i32,
    buf: Vec<u8>,
}

// 只在 engine 工作线程里使用
unsafe impl Send for Capturer {}

impl Capturer {
    pub fn new() -> Option<Self> {
        unsafe {
            let screen = GetDC(None);
            if screen.0.is_null() {
                return None;
            }
            let mem = CreateCompatibleDC(Some(screen));
            ReleaseDC(None, screen);
            if mem.0.is_null() {
                return None;
            }
            Some(Self {
                mem,
                bmp: HBITMAP::default(),
                width: 0,
                height: 0,
                buf: Vec::new(),
            })
        }
    }

    fn ensure(&mut self, w: i32, h: i32) -> bool {
        if !self.bmp.0.is_null() && self.width >= w && self.height >= h {
            return true;
        }
        let nw = w.max(self.width).max(16);
        let nh = h.max(self.height).max(16);
        unsafe {
            let screen = GetDC(None);
            if screen.0.is_null() {
                return false;
            }
            let bmp = CreateCompatibleBitmap(screen, nw, nh);
            ReleaseDC(None, screen);
            if bmp.0.is_null() {
                return false;
            }
            SelectObject(self.mem, HGDIOBJ(bmp.0));
            if !self.bmp.0.is_null() {
                let _ = DeleteObject(HGDIOBJ(self.bmp.0));
            }
            self.bmp = bmp;
            self.width = nw;
            self.height = nh;
            true
        }
    }

    /// 抓屏幕上一块矩形（物理像素）
    pub fn grab(&mut self, x: i32, y: i32, w: u32, h: u32) -> Option<Frame> {
        if w == 0 || h == 0 || w > 4096 || h > 4096 {
            return None;
        }
        if !self.ensure(w as i32, h as i32) {
            return None;
        }
        unsafe {
            let screen = GetDC(None);
            if screen.0.is_null() {
                return None;
            }
            let copied = BitBlt(
                self.mem,
                0,
                0,
                w as i32,
                h as i32,
                Some(screen),
                x,
                y,
                ROP_CODE(ROP),
            )
            .is_ok();
            ReleaseDC(None, screen);
            if !copied {
                return None;
            }

            let mut bi = BITMAPINFO::default();
            bi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
            bi.bmiHeader.biWidth = w as i32;
            bi.bmiHeader.biHeight = -(h as i32); // 顶向下
            bi.bmiHeader.biPlanes = 1;
            bi.bmiHeader.biBitCount = 32;
            bi.bmiHeader.biCompression = 0u32; // BI_RGB
            let need = (w as usize) * (h as usize) * 4;
            if self.buf.len() < need {
                self.buf.resize(need, 0);
            }
            let got = GetDIBits(
                self.mem,
                self.bmp,
                0u32,
                h,
                Some(self.buf.as_mut_ptr() as *mut c_void),
                &mut bi,
                DIB_RGB_COLORS,
            );
            if got == 0 {
                return None;
            }
            let mut rgba = vec![0u8; need];
            for i in (0..need).step_by(4) {
                rgba[i] = self.buf[i + 2];
                rgba[i + 1] = self.buf[i + 1];
                rgba[i + 2] = self.buf[i];
                rgba[i + 3] = 255;
            }
            Some(Frame { w, h, rgba })
        }
    }
}

impl Drop for Capturer {
    fn drop(&mut self) {
        unsafe {
            if !self.bmp.0.is_null() {
                let _ = DeleteObject(HGDIOBJ(self.bmp.0));
            }
            if !self.mem.0.is_null() {
                let _ = DeleteDC(self.mem);
            }
        }
    }
}
