//! 全局低级键盘钩子（WH_KEYBOARD_LL）。
//!
//! 只读监听，不拦截、不注入、不修改按键事件 —— 钩子函数始终把事件原样透传。
//! 回调里只做「塞进 channel」一件事，保证不会拖慢系统输入。

use std::collections::HashSet;
use std::sync::mpsc::Sender;
use std::sync::OnceLock;
use std::time::Instant;

use windows::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, GetMessageW, SetWindowsHookExW, UnhookWindowsHookEx, KBDLLHOOKSTRUCT, MSG,
    WH_KEYBOARD_LL, WM_KEYDOWN, WM_KEYUP, WM_SYSKEYDOWN, WM_SYSKEYUP,
};

#[derive(Clone, Copy, Debug)]
pub struct VkEvent {
    pub vk: u32,
    pub down: bool,
    pub at: Instant,
}

static TX: OnceLock<Sender<VkEvent>> = OnceLock::new();

unsafe extern "system" fn hook_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code >= 0 {
        let msg = wparam.0 as u32;
        let down = msg == WM_KEYDOWN || msg == WM_SYSKEYDOWN;
        let up = msg == WM_KEYUP || msg == WM_SYSKEYUP;
        if down || up {
            let kb = &*(lparam.0 as *const KBDLLHOOKSTRUCT);
            // 忽略软件注入的按键（连点器 / 自动化），只认真实硬件输入
            let injected = (kb.flags.0 & 0x10) != 0;
            if !injected {
                if let Some(tx) = TX.get() {
                    let _ = tx.send(VkEvent {
                        vk: kb.vkCode,
                        down,
                        at: Instant::now(),
                    });
                }
            }
        }
    }
    CallNextHookEx(None, code, wparam, lparam)
}

/// 在独立线程里安装钩子并跑消息循环
pub fn spawn(tx: Sender<VkEvent>) -> bool {
    if TX.set(tx).is_err() {
        return false;
    }
    std::thread::Builder::new()
        .name("kbd-hook".into())
        .spawn(|| unsafe {
            let hook = match SetWindowsHookExW(WH_KEYBOARD_LL, Some(hook_proc), None, 0) {
                Ok(h) => h,
                Err(e) => {
                    eprintln!("安装键盘钩子失败: {e}");
                    return;
                }
            };
            let mut msg = MSG::default();
            while GetMessageW(&mut msg, None, 0, 0).as_bool() {
                // 不需要处理，消息循环只是为了让钩子活着
            }
            let _ = UnhookWindowsHookEx(hook);
        })
        .is_ok()
}

/// 按键名 -> 虚拟键码
pub fn vk_from_name(name: &str) -> Option<u32> {
    let n = name.trim().to_uppercase();
    if n.len() == 1 {
        let c = n.chars().next().unwrap() as u32;
        if (0x30..=0x39).contains(&c) || (0x41..=0x5A).contains(&c) {
            return Some(c);
        }
    }
    if let Some(rest) = n.strip_prefix('F') {
        if let Ok(i) = rest.parse::<u32>() {
            if (1..=24).contains(&i) {
                return Some(0x70 + i - 1);
            }
        }
    }
    Some(match n.as_str() {
        "SPACE" | "空格" => 0x20,
        "TAB" => 0x09,
        "ENTER" | "回车" => 0x0D,
        "SHIFT" => 0xA0,
        "CTRL" | "CONTROL" => 0xA2,
        "ALT" => 0xA4,
        "ESC" | "ESCAPE" => 0x1B,
        "CAPS" | "CAPSLOCK" => 0x14,
        "BACKSPACE" => 0x08,
        "UP" | "ARROWUP" => 0x26,
        "DOWN" | "ARROWDOWN" => 0x28,
        "LEFT" | "ARROWLEFT" => 0x25,
        "RIGHT" | "ARROWRIGHT" => 0x27,
        _ => return None,
    })
}

/// 虚拟键码 -> 展示名
pub fn vk_name(vk: u32) -> String {
    if (0x30..=0x39).contains(&vk) || (0x41..=0x5A).contains(&vk) {
        return char::from_u32(vk).map(|c| c.to_string()).unwrap_or_default();
    }
    if (0x70..=0x87).contains(&vk) {
        return format!("F{}", vk - 0x70 + 1);
    }
    match vk {
        0x20 => "Space".into(),
        0x09 => "Tab".into(),
        0x0D => "Enter".into(),
        0xA0 => "Shift".into(),
        0xA2 => "Ctrl".into(),
        0xA4 => "Alt".into(),
        0x1B => "Esc".into(),
        0x14 => "CapsLock".into(),
        0x08 => "Backspace".into(),
        other => format!("VK{other:02X}"),
    }
}

/// 进程内按键去重（自动重复的按下只算一次）
#[derive(Default)]
pub struct PressFilter {
    down: HashSet<u32>,
}

impl PressFilter {
    /// 返回 true 表示这是一次新的按下
    pub fn press(&mut self, vk: u32) -> bool {
        self.down.insert(vk)
    }
    pub fn release(&mut self, vk: u32) {
        self.down.remove(&vk);
    }
    pub fn clear(&mut self) {
        self.down.clear();
    }
}
