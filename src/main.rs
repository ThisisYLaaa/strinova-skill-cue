#![cfg_attr(all(not(debug_assertions), windows), windows_subsystem = "windows")]

mod audio;
mod capture;
mod config;
mod engine;
mod hook;
mod selftest;
mod snapshot;
mod theme;
mod ui;
mod vision;

use std::path::PathBuf;

use config::Config;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let wants_selftest = args
        .iter()
        .any(|a| a.eq_ignore_ascii_case("--selftest") || a.eq_ignore_ascii_case("selftest"));
    let wants_help = args
        .iter()
        .any(|a| a.eq_ignore_ascii_case("--help") || a.eq_ignore_ascii_case("-h"));

    if wants_selftest || wants_help {
        attach_console();
        if wants_help {
            print_help();
            return;
        }
        config::ensure_dirs();
        let images: Vec<PathBuf> = args
            .iter()
            .filter(|a| !a.starts_with('-'))
            .map(PathBuf::from)
            .collect();
        let code = selftest::run(&images);
        std::process::exit(code);
    }

    config::ensure_dirs();
    let mut cfg = Config::load();
    let first_run = !cfg.onboarded;
    let seeded = config::seed_characters(&mut cfg);
    if first_run {
        config::install_demo_sounds(&mut cfg);
    }
    if seeded || first_run {
        cfg.onboarded = true;
        cfg.normalize();
        cfg.save();
    }

    set_dpi_aware();
    let icon = make_icon();

    // --snapshot-ui <目录>：自动截图各页面后退出（开发自检用）
    let snapshot_dir = args
        .iter()
        .position(|a| a == "--snapshot-ui")
        .and_then(|i| args.get(i + 1))
        .map(PathBuf::from);

    let mut viewport = eframe::egui::ViewportBuilder::default()
        .with_inner_size([snapshot::WIN_W, snapshot::WIN_H])
        .with_min_inner_size([940.0, 620.0])
        .with_decorations(false)
        .with_title(format!("{} · {}", config::APP_NAME, config::APP_NAME_EN))
        .with_icon(icon);
    if snapshot_dir.is_some() {
        viewport = viewport.with_position([snapshot::WIN_X, snapshot::WIN_Y]);
    }
    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };

    if let Err(e) = eframe::run_native(
        "strinova-skill-cue",
        options,
        Box::new(move |cc| {
            let app = ui::App::new(cc, cfg);
            Ok(Box::new(match snapshot_dir {
                Some(d) => app.with_snapshot(d),
                None => app,
            }))
        }),
    ) {
        eprintln!("启动失败: {e}");
        attach_console();
        print_help();
    }
}

fn print_help() {
    println!("{} ({})", config::APP_NAME, config::APP_NAME_EN);
    println!("作者: {}", config::AUTHOR);
    println!("版本: {}", config::APP_VERSION);
    println!();
    println!("直接运行: 打开图形界面");
    println!("离线自检: --selftest <截图1> [截图2 ...]");
    println!("  用真实截图验证黄条判定与头像匹配，结果写入 自检报告.txt");
}

#[cfg(windows)]
fn attach_console() {
    unsafe {
        use windows::Win32::System::Console::{AllocConsole, ATTACH_PARENT_PROCESS};
        if let Err(_) = windows::Win32::System::Console::AttachConsole(ATTACH_PARENT_PROCESS) {
            let _ = AllocConsole();
        }
    }
}

#[cfg(not(windows))]
fn attach_console() {}

#[cfg(windows)]
fn set_dpi_aware() {
    unsafe {
        use windows::Win32::UI::HiDpi::{
            SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
        };
        // 失败无所谓：winit 自己也会设
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }
}

#[cfg(not(windows))]
fn set_dpi_aware() {}

/// 程序图标：圆角方块 + 四根音量条（运行时生成，不额外带资源文件）
fn make_icon() -> eframe::egui::IconData {
    const S: usize = 64;
    let mut rgba = vec![0u8; S * S * 4];
    let accent = [0x0F, 0x6C, 0xBD];
    let bars = [(14usize, 20usize), (26, 38), (38, 30), (50, 14)];
    for y in 0..S {
        for x in 0..S {
            let i = (y * S + x) * 4;
            // 圆角 14px
            let r = 14i32;
            let dx = if (x as i32) < r {
                r - x as i32
            } else if (x as i32) > S as i32 - r {
                x as i32 - (S as i32 - r)
            } else {
                0
            };
            let dy = if (y as i32) < r {
                r - y as i32
            } else if (y as i32) > S as i32 - r {
                y as i32 - (S as i32 - r)
            } else {
                0
            };
            let inside = dx == 0 || dy == 0 || (dx * dx + dy * dy) <= r * r;
            let mut c = [0u8, 0, 0, 0];
            if inside {
                c = [accent[0], accent[1], accent[2], 255];
                for (bx, bh) in bars {
                    let half = bh as i32 / 2;
                    let cy = S as i32 / 2;
                    if (x as i32) >= bx as i32 - 1
                        && (x as i32) <= bx as i32 + 3
                        && (y as i32) >= cy - half
                        && (y as i32) <= cy + half
                    {
                        c = [255, 255, 255, 255];
                    }
                }
            }
            rgba[i..i + 4].copy_from_slice(&c);
        }
    }
    eframe::egui::IconData {
        rgba,
        width: S as u32,
        height: S as u32,
    }
}
