//! 离线自检：拿真实截图验证「黄条判定 / 头像匹配」，不需要开游戏。
//!
//! 用法（release 或 debug 都可）：
//!   strinova-skill-cue.exe --selftest 截图1.png 截图2.png ...
//! 会把结果同时打印到控制台并写入 `自检报告.txt`。

use std::io::Write;
use std::path::{Path, PathBuf};

use crate::capture::Frame;
use crate::config::Config;
use crate::vision::{self, Template};

fn load(path: &Path) -> Result<Frame, String> {
    let img = image::open(path)
        .map_err(|e| format!("{}: {e}", path.display()))?
        .to_rgba8();
    Ok(Frame {
        w: img.width(),
        h: img.height(),
        rgba: img.into_raw(),
    })
}

fn crop(f: &Frame, x: i32, y: i32, w: u32, h: u32) -> Frame {
    let x0 = x.max(0) as u32;
    let y0 = y.max(0) as u32;
    let w = w.min(f.w.saturating_sub(x0)).max(1);
    let h = h.min(f.h.saturating_sub(y0)).max(1);
    let mut rgba = Vec::with_capacity((w * h * 4) as usize);
    for row in 0..h {
        let sy = y0 + row;
        if sy >= f.h {
            break;
        }
        let start = ((sy * f.w + x0) * 4) as usize;
        let end = start + (w * 4) as usize;
        if end <= f.rgba.len() {
            rgba.extend_from_slice(&f.rgba[start..end]);
        }
    }
    Frame { w, h, rgba }
}

pub fn run(images: &[PathBuf]) -> i32 {
    let mut out = String::new();
    let mut o = |s: String| {
        println!("{s}");
        out.push_str(&s);
        out.push('\n');
    };

    o("== 超弦体技能音效 · 离线自检 ==".into());
    let cfg = Config::load();
    o(format!(
        "配置: {} 个技能槽 · 头像阈值 {:.2} · 音量 {:.2}",
        cfg.actions.len(),
        cfg.avatar_threshold,
        cfg.volume
    ));

    // 头像模板库
    let mut templates: Vec<Template> = Vec::new();
    if let Ok(rd) = std::fs::read_dir(crate::config::avatars_dir()) {
        for e in rd.flatten() {
            let p = e.path();
            if p.extension().and_then(|s| s.to_str()).map(|s| s.eq_ignore_ascii_case("png")) == Some(true) {
                if let Ok(f) = load(&p) {
                    let id = p
                        .file_stem()
                        .and_then(|s| s.to_str())
                        .unwrap_or("?")
                        .to_string();
                    let name = cfg.character_name(&id);
                    templates.push(Template {
                        id,
                        name,
                        vec: vision::avatar_signature(&f, 0.10),
                    });
                }
            }
        }
    }
    o(format!("头像模板: {} 个", templates.len()));

    if images.is_empty() {
        o("\n没有传入截图。用法: --selftest <png> [png...]".into());
        let _ = std::fs::write("自检报告.txt", &out);
        return 2;
    }

    let mut failures = 0;
    for path in images {
        match load(path) {
            Ok(f) => {
                o(format!(
                    "\n--- {} ({}x{}) ---",
                    path.file_name().and_then(|s| s.to_str()).unwrap_or("?"),
                    f.w,
                    f.h
                ));
                for a in &cfg.actions {
                    let (x, y, w, h) = a.rect.to_px(f.w, f.h);
                    let sub = crop(&f, x, y, w, h);
                    let st = vision::analyze_bar(&sub);
                    let ready = st.ready(a.min_fill);
                    o(format!(
                        "  {:<6} 区域 {:<4}x{:<3} @{},{}  黄色 {:>5.1}% (阈值 {:>4.0}%)  亮度 {:.2}  饱和 {:.2}  => {}",
                        a.label,
                        w,
                        h,
                        x,
                        y,
                        st.fill * 100.0,
                        a.min_fill * 100.0,
                        st.mean_v,
                        st.mean_s,
                        if ready { "就绪" } else { "未就绪" }
                    ));
                }
                // 头像
                let (x, y, w, h) = cfg.avatar_rect.to_px(f.w, f.h);
                let sub = crop(&f, x, y, w, h);
                if !templates.is_empty() {
                    let sig = vision::avatar_signature(&sub, 0.10);
                    let mut scored: Vec<(String, f32)> = templates
                        .iter()
                        .map(|t| (t.name.clone(), vision::ncc(&sig, &t.vec)))
                        .collect();
                    scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
                    o(format!("  头像区域 {}x{} @{},{}  最佳匹配:", w, h, x, y));
                    for (name, s) in scored.iter().take(3) {
                        o(format!("    {name:<10} 相似度 {s:.3}{}", if *s >= cfg.avatar_threshold { "  ✔" } else { "" }));
                    }
                    if scored.first().map(|s| s.1 < cfg.avatar_threshold).unwrap_or(true) {
                        failures += 1;
                    }
                } else {
                    o(format!("  头像区域 {}x{} @{},{} （无模板，跳过匹配）", w, h, x, y));
                }
            }
            Err(e) => {
                o(format!("!! 读取失败: {e}"));
                failures += 1;
            }
        }
    }
    o(format!("\n完成，失败项 {failures}"));
    let report = "自检报告.txt";
    if let Ok(mut f) = std::fs::File::create(report) {
        let _ = f.write_all(out.as_bytes());
    }
    if failures == 0 {
        0
    } else {
        1
    }
}
