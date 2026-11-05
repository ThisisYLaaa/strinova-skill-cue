//! 界面自截图：让程序把自己窗口的几个页面各存一张 PNG。
//!
//! 用途：开发/回归时检查排版、字体、配色，不需要人工盯屏幕。
//! 用法： `--snapshot-ui <输出目录>`

use std::ffi::c_void;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use windows::core::PCWSTR;
use windows::Win32::Foundation::{HWND, RECT};
use windows::Win32::Graphics::Dwm::{
    DwmGetWindowAttribute, DWMWA_EXTENDED_FRAME_BOUNDS,
};
use windows::Win32::UI::WindowsAndMessaging::{
    BringWindowToTop, FindWindowW, SetForegroundWindow, SetWindowPos, HWND_TOPMOST,
    SWP_NOMOVE, SWP_NOSIZE, SWP_SHOWWINDOW,
};

use crate::capture::Capturer;

pub const PAGES: [&str; 5] = [
    "1-dashboard",
    "2-sounds",
    "3-characters",
    "4-detection",
    "5-about",
];

// 窗口放置（与 main 里保持一致，单位是点）
pub const WIN_X: f32 = 60.0;
pub const WIN_Y: f32 = 60.0;
pub const WIN_W: f32 = 1160.0;
pub const WIN_H: f32 = 800.0;

pub struct Job {
    pub dir: PathBuf,
    pub page: usize,
    shots: usize,
    failed: usize,
    hwnd: isize,
    at: Option<Instant>,
    topmost: bool,
}

/// 找到主窗口（按标题精确匹配）
pub fn main_hwnd() -> Option<HWND> {
    let title = format!("{} · {}", crate::config::APP_NAME, crate::config::APP_NAME_EN);
    let wide: Vec<u16> = title.encode_utf16().chain(std::iter::once(0)).collect();
    unsafe {
        match FindWindowW(None, PCWSTR(wide.as_ptr())) {
            Ok(h) if !h.0.is_null() => Some(h),
            _ => None,
        }
    }
}

/// Win11 标题栏跟随深浅色（DWMWA_USE_IMMERSIVE_DARK_MODE = 20）+ 圆角
pub fn set_dark_titlebar(dark: bool) -> bool {
    use windows::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWINDOWATTRIBUTE};
    let Some(hwnd) = main_hwnd() else {
        return false;
    };
    let value: i32 = if dark { 1 } else { 0 };
    unsafe {
        let dark_ok = DwmSetWindowAttribute(
            hwnd,
            DWMWINDOWATTRIBUTE(20),
            &value as *const i32 as *const c_void,
            std::mem::size_of::<i32>() as u32,
        )
        .is_ok();
        // DWMWA_WINDOW_CORNER_PREFERENCE(33) = DWMWCP_ROUND(2)
        let round: i32 = 2;
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWINDOWATTRIBUTE(33),
            &round as *const i32 as *const c_void,
            std::mem::size_of::<i32>() as u32,
        );
        dark_ok
    }
}

impl Job {
    pub fn new(dir: PathBuf) -> Self {
        let _ = std::fs::create_dir_all(&dir);
        Self {
            dir,
            page: 0,
            shots: 0,
            failed: 0,
            hwnd: 0,
            at: None,
            topmost: false,
        }
    }

    fn find_window(&mut self) {
        if let Some(h) = main_hwnd() {
            self.hwnd = h.0 as isize;
            unsafe {
                let _ = SetWindowPos(
                    h,
                    Some(HWND_TOPMOST),
                    0,
                    0,
                    0,
                    0,
                    SWP_NOMOVE | SWP_NOSIZE | SWP_SHOWWINDOW,
                );
                let _ = BringWindowToTop(h);
                let _ = SetForegroundWindow(h);
            }
            self.topmost = true;
        }
    }

    /// 每帧调用；返回 true 表示全部截完可以退出
    pub fn step(&mut self) -> bool {
        if self.at.is_none() {
            self.find_window();
            self.at = Some(Instant::now());
            return false;
        }
        // 等窗口稳定（字体上屏、DWM 合成）
        if self.at.map(|t| t.elapsed() < Duration::from_millis(420)).unwrap_or(false) {
            return false;
        }
        let ok = self.shoot();
        if ok {
            self.shots += 1;
        } else {
            self.failed += 1;
        }
        self.page += 1;
        self.at = Some(Instant::now());
        if self.page >= PAGES.len() {
            self.finish();
            return true;
        }
        false
    }

    fn shoot(&mut self) -> bool {
        let Some((x, y, w, h)) = self.bounds() else {
            return false;
        };
        let name = PAGES[self.page.min(PAGES.len() - 1)];
        let path = self.dir.join(format!("ui-{name}.png"));
        let mut cap = match Capturer::new() {
            Some(c) => c,
            None => return false,
        };
        let Some(f) = cap.grab(x, y, w, h) else {
            return false;
        };
        image::save_buffer(&path, &f.rgba, f.w, f.h, image::ExtendedColorType::Rgba8).is_ok()
    }

    fn bounds(&self) -> Option<(i32, i32, u32, u32)> {
        if self.hwnd == 0 {
            return None;
        }
        let hwnd = HWND(self.hwnd as *mut c_void);
        unsafe {
            let mut r = RECT::default();
            let ok = DwmGetWindowAttribute(
                hwnd,
                DWMWA_EXTENDED_FRAME_BOUNDS,
                &mut r as *mut RECT as *mut c_void,
                std::mem::size_of::<RECT>() as u32,
            );
            if ok.is_err() {
                use windows::Win32::UI::WindowsAndMessaging::GetWindowRect;
                let _ = GetWindowRect(hwnd, &mut r);
            }
            let w = (r.right - r.left).max(16) as u32;
            let h = (r.bottom - r.top).max(16) as u32;
            Some((r.left, r.top, w, h))
        }
    }

    fn finish(&mut self) {
        let msg = format!(
            "界面截图完成: {} 张，失败 {}",
            self.shots, self.failed
        );
        let _ = std::fs::write(self.dir.join("snapshot-log.txt"), msg);
        if self.topmost && self.hwnd != 0 {
            unsafe {
                let hwnd = HWND(self.hwnd as *mut c_void);
                let _ = SetWindowPos(
                    hwnd,
                    Some(HWND(-1 as isize as *mut c_void)), // 保留置顶，反正马上退出
                    0,
                    0,
                    0,
                    0,
                    SWP_NOMOVE | SWP_NOSIZE,
                );
            }
        }
    }
}
