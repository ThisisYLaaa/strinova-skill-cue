//! 图像判定：黄条就绪检测 + 角色头像模板匹配。
//!
//! 两者都只依赖截图（不注入游戏、不读内存）。

use crate::capture::Frame;

/// 单帧黄条统计
#[derive(Clone, Copy, Debug, Default)]
pub struct BarStats {
    /// 判定区域内「黄色」像素占比 0..1
    pub fill: f32,
    /// 平均亮度 0..1
    pub mean_v: f32,
    /// 平均饱和度 0..1
    pub mean_s: f32,
    pub pixels: u32,
}

impl BarStats {
    pub fn ready(&self, min_fill: f32) -> bool {
        self.pixels > 0 && self.fill >= min_fill
    }
}

/// 把 RGB 转成 HSV（h: 0..360）
#[inline]
fn rgb_to_hsv(r: u8, g: u8, b: u8) -> (f32, f32, f32) {
    let rf = r as f32 / 255.0;
    let gf = g as f32 / 255.0;
    let bf = b as f32 / 255.0;
    let mx = rf.max(gf).max(bf);
    let mn = rf.min(gf).min(bf);
    let d = mx - mn;
    let v = mx;
    let s = if mx > 0.0 { d / mx } else { 0.0 };
    if d <= f32::EPSILON {
        return (0.0, s, v);
    }
    let mut h = if (mx - rf).abs() < f32::EPSILON {
        60.0 * ((gf - bf) / d)
    } else if (mx - gf).abs() < f32::EPSILON {
        60.0 * ((bf - rf) / d) + 120.0
    } else {
        60.0 * ((rf - gf) / d) + 240.0
    };
    if h < 0.0 {
        h += 360.0;
    }
    (h, s, v)
}

/// 卡拉彼丘技能条那种「亮黄」：色相 32°~72°，饱和度 ≥0.30，亮度 ≥0.40
#[inline]
fn is_bar_yellow(h: f32, s: f32, v: f32) -> bool {
    (32.0..=72.0).contains(&h) && s >= 0.30 && v >= 0.40
}

/// 统计一块截图里的黄色占比
pub fn analyze_bar(frame: &Frame) -> BarStats {
    let n = (frame.w * frame.h) as usize;
    if n == 0 {
        return BarStats::default();
    }
    let mut yellow = 0u32;
    let mut sum_v = 0.0f32;
    let mut sum_s = 0.0f32;
    for px in frame.rgba.chunks_exact(4) {
        let (h, s, v) = rgb_to_hsv(px[0], px[1], px[2]);
        if is_bar_yellow(h, s, v) {
            yellow += 1;
        }
        sum_v += v;
        sum_s += s;
    }
    BarStats {
        fill: yellow as f32 / n as f32,
        mean_v: sum_v / n as f32,
        mean_s: sum_s / n as f32,
        pixels: n as u32,
    }
}

/// 头像模板：24x24 归一化灰度向量
pub const AVATAR_GRID: usize = 24;

#[derive(Clone, Debug)]
pub struct Template {
    pub id: String,
    pub name: String,
    pub vec: Vec<f32>,
}

/// 把截图（取中间 inner 比例的区域）压成 AVATAR_GRID^2 的零均值单位向量
pub fn avatar_signature(frame: &Frame, inner: f32) -> Vec<f32> {
    let g = AVATAR_GRID;
    let mut out = vec![0.0f32; g * g];
    if frame.w == 0 || frame.h == 0 {
        return out;
    }
    let inset = inner.clamp(0.0, 0.45);
    let x0 = (frame.w as f32 * inset) as u32;
    let y0 = (frame.h as f32 * inset) as u32;
    let x1 = frame.w - x0;
    let y1 = frame.h - y0;
    let bw = ((x1 - x0) as f32 / g as f32).max(1.0);
    let bh = ((y1 - y0) as f32 / g as f32).max(1.0);

    for gy in 0..g {
        for gx in 0..g {
            let sx0 = x0 as f32 + gx as f32 * bw;
            let sy0 = y0 as f32 + gy as f32 * bh;
            let sx1 = (sx0 + bw).min(x1 as f32);
            let sy1 = (sy0 + bh).min(y1 as f32);
            let mut acc = 0.0f32;
            let mut cnt = 0.0f32;
            let mut y = sy0;
            while y < sy1 {
                let mut x = sx0;
                while x < sx1 {
                    let i = ((y as u32 * frame.w + x as u32) * 4) as usize;
                    if i + 2 < frame.rgba.len() {
                        let r = frame.rgba[i] as f32;
                        let gg = frame.rgba[i + 1] as f32;
                        let b = frame.rgba[i + 2] as f32;
                        acc += 0.299 * r + 0.587 * gg + 0.114 * b;
                        cnt += 1.0;
                    }
                    x += 1.0;
                }
                y += 1.0;
            }
            out[gy * g + gx] = if cnt > 0.0 { acc / cnt } else { 0.0 };
        }
    }
    normalize_vec(&mut out);
    out
}

fn normalize_vec(v: &mut [f32]) {
    let n = v.len() as f32;
    let mean = v.iter().sum::<f32>() / n.max(1.0);
    for x in v.iter_mut() {
        *x -= mean;
    }
    let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 1e-6 {
        for x in v.iter_mut() {
            *x /= norm;
        }
    }
}

/// 归一化互相关：1.0 = 完全一致
pub fn ncc(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    a.iter().zip(b.iter()).map(|(x, y)| x * y).sum()
}

/// 在模板库里找最佳匹配。返回 (模板, 相似度)
pub fn best_match<'a>(sig: &[f32], templates: &'a [Template]) -> Option<(&'a Template, f32)> {
    let mut best: Option<(&Template, f32)> = None;
    for t in templates {
        let s = ncc(sig, &t.vec);
        if best.map(|(_, bs)| s > bs).unwrap_or(true) {
            best = Some((t, s));
        }
    }
    best
}

/// 区域尺寸是否合理（防止配置写坏导致狂读像素）
pub fn sane_rect(w: u32, h: u32) -> bool {
    w > 1 && h > 1 && w <= 2048 && h <= 2048
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame_from(w: u32, h: u32, rgb: [u8; 3]) -> Frame {
        let mut rgba = Vec::with_capacity((w * h * 4) as usize);
        for _ in 0..(w * h) {
            rgba.extend_from_slice(&[rgb[0], rgb[1], rgb[2], 255]);
        }
        Frame { w, h, rgba }
    }

    #[test]
    fn yellow_bar_is_full() {
        // #F5C542 就是游戏里的亮黄
        let f = frame_from(100, 10, [245, 197, 66]);
        let s = analyze_bar(&f);
        assert!(s.fill > 0.99, "fill={}", s.fill);
        assert!(s.ready(0.8));
    }

    #[test]
    fn grey_bar_is_not_full() {
        let f = frame_from(100, 10, [110, 125, 145]);
        let s = analyze_bar(&f);
        assert!(s.fill < 0.01, "fill={}", s.fill);
        assert!(!s.ready(0.2));
    }

    #[test]
    fn identical_avatar_scores_one() {
        let mut rgba = Vec::new();
        for y in 0..40u32 {
            for x in 0..40u32 {
                let v = ((x * 7 + y * 13) % 255) as u8;
                rgba.extend_from_slice(&[v, v / 2, 255 - v, 255]);
            }
        }
        let f = Frame { w: 40, h: 40, rgba };
        let sig = avatar_signature(&f, 0.1);
        let t = Template { id: "a".into(), name: "A".into(), vec: sig.clone() };
        assert!((ncc(&sig, &t.vec) - 1.0).abs() < 1e-4);
    }
}
