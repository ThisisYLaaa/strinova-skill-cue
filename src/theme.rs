//! Win11 风格主题：配色、字体（微软雅黑）、控件微调、通用小部件。

use std::sync::Arc;

use eframe::egui::{
    self, Color32, CornerRadius, FontFamily, FontId, Response, RichText, Sense, Stroke, TextStyle,
    Ui, Vec2,
};

#[derive(Clone, Copy)]
pub struct Palette {
    pub dark: bool,
    pub bg: Color32,
    pub card: Color32,
    pub card_hover: Color32,
    pub field_bg: Color32,
    pub stroke: Color32,
    pub text: Color32,
    pub text_dim: Color32,
    pub accent: Color32,
    pub accent_hover: Color32,
    pub accent_soft: Color32,
    pub on_accent: Color32,
    pub ok: Color32,
    pub warn: Color32,
    pub err: Color32,
    pub track: Color32,
}

impl Palette {
    pub fn light() -> Self {
        Self {
            dark: false,
            bg: Color32::from_rgb(0xF3, 0xF3, 0xF3),
            card: Color32::from_rgb(0xFF, 0xFF, 0xFF),
            card_hover: Color32::from_rgb(0xF7, 0xF7, 0xF7),
            field_bg: Color32::from_rgb(0xFF, 0xFF, 0xFF),
            stroke: Color32::from_rgb(0xE1, 0xE1, 0xE1),
            text: Color32::from_rgb(0x1B, 0x1B, 0x1B),
            text_dim: Color32::from_rgb(0x5F, 0x5F, 0x5F),
            accent: Color32::from_rgb(0x0F, 0x6C, 0xBD),
            accent_hover: Color32::from_rgb(0x1A, 0x7F, 0xD0),
            accent_soft: Color32::from_rgb(0xE5, 0xF1, 0xFB),
            on_accent: Color32::from_rgb(0xFF, 0xFF, 0xFF),
            ok: Color32::from_rgb(0x0F, 0x7B, 0x0F),
            warn: Color32::from_rgb(0x9D, 0x5D, 0x00),
            err: Color32::from_rgb(0xC4, 0x2B, 0x1C),
            track: Color32::from_rgb(0xE6, 0xE6, 0xE6),
        }
    }

    pub fn dark() -> Self {
        Self {
            dark: true,
            bg: Color32::from_rgb(0x20, 0x20, 0x20),
            card: Color32::from_rgb(0x2B, 0x2B, 0x2B),
            card_hover: Color32::from_rgb(0x33, 0x33, 0x33),
            field_bg: Color32::from_rgb(0x1B, 0x1B, 0x1B),
            stroke: Color32::from_rgb(0x3A, 0x3A, 0x3A),
            text: Color32::from_rgb(0xF2, 0xF2, 0xF2),
            text_dim: Color32::from_rgb(0xA6, 0xA6, 0xA6),
            accent: Color32::from_rgb(0x4C, 0xC2, 0xFF),
            accent_hover: Color32::from_rgb(0x6A, 0xCE, 0xFF),
            accent_soft: Color32::from_rgb(0x24, 0x3B, 0x4A),
            on_accent: Color32::from_rgb(0x00, 0x21, 0x2E),
            ok: Color32::from_rgb(0x6C, 0xCB, 0x5F),
            warn: Color32::from_rgb(0xE8, 0xC2, 0x4A),
            err: Color32::from_rgb(0xFF, 0x99, 0xA4),
            track: Color32::from_rgb(0x3A, 0x3A, 0x3A),
        }
    }
}

/// 跟随系统深浅色（读注册表，失败按浅色）
pub fn system_is_dark() -> bool {
    #[cfg(windows)]
    unsafe {
        use windows::core::w;
        use windows::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_DWORD};
        let mut val: u32 = 1;
        let mut size = std::mem::size_of::<u32>() as u32;
        let ok = RegGetValueW(
            HKEY_CURRENT_USER,
            w!("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize"),
            w!("AppsUseLightTheme"),
            RRF_RT_REG_DWORD,
            None,
            Some(&mut val as *mut u32 as *mut std::ffi::c_void),
            Some(&mut size),
        );
        if ok.is_err() {
            return false;
        }
        val == 0
    }
    #[cfg(not(windows))]
    {
        false
    }
}

/// 安装中文字体（微软雅黑优先）。用 ab_glyph 预校验，避免 egui 内部 panic。
pub fn install_fonts(ctx: &egui::Context) {
    const CANDIDATES: &[&str] = &[
        r"C:\Windows\Fonts\msyh.ttc",
        r"C:\Windows\Fonts\msyh.ttf",
        r"C:\Windows\Fonts\msyhl.ttc",
        r"C:\Windows\Fonts\simhei.ttf",
        r"C:\Windows\Fonts\Deng.ttf",
        r"C:\Windows\Fonts\simsun.ttc",
    ];
    let mut defs = egui::FontDefinitions::default();
    for path in CANDIDATES {
        let Ok(bytes) = std::fs::read(path) else {
            continue;
        };
        if ab_glyph::FontVec::try_from_vec_and_index(bytes.clone(), 0).is_err() {
            continue;
        }
        defs.font_data
            .insert("cjk".to_owned(), Arc::new(egui::FontData::from_owned(bytes)));
        for family in [FontFamily::Proportional, FontFamily::Monospace] {
            defs.families
                .entry(family)
                .or_default()
                .insert(0, "cjk".to_owned());
        }
        break;
    }
    ctx.set_fonts(defs);
}

pub fn apply_style(ctx: &egui::Context, p: &Palette) {
    let mut style = (*ctx.style()).clone();
    let mut v = if p.dark { egui::Visuals::dark() } else { egui::Visuals::light() };

    v.panel_fill = p.bg;
    v.window_fill = p.bg;
    v.extreme_bg_color = p.field_bg;
    v.faint_bg_color = p.card_hover;
    v.window_stroke = Stroke::new(1.0_f32, p.stroke);
    v.window_corner_radius = CornerRadius::same(10);
    v.menu_corner_radius = CornerRadius::same(8);
    v.window_shadow = egui::epaint::Shadow {
        offset: [0, 6],
        blur: 18,
        spread: 0,
        color: Color32::from_black_alpha(if p.dark { 120 } else { 40 }),
    };
    v.popup_shadow = v.window_shadow;
    v.selection.bg_fill = p.accent.gamma_multiply(0.30);
    v.selection.stroke = Stroke::new(1.0_f32, p.accent);
    v.hyperlink_color = p.accent;
    v.override_text_color = Some(p.text);

    for w in [
        &mut v.widgets.noninteractive,
        &mut v.widgets.inactive,
        &mut v.widgets.hovered,
        &mut v.widgets.active,
        &mut v.widgets.open,
    ] {
        w.corner_radius = CornerRadius::same(6);
    }
    v.widgets.noninteractive.bg_fill = p.card;
    v.widgets.noninteractive.weak_bg_fill = p.card;
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0_f32, p.stroke);
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0_f32, p.text);

    v.widgets.inactive.bg_fill = p.card;
    v.widgets.inactive.weak_bg_fill = p.card_hover;
    v.widgets.inactive.bg_stroke = Stroke::new(1.0_f32, p.stroke);
    v.widgets.inactive.fg_stroke = Stroke::new(1.0_f32, p.text);

    v.widgets.hovered.bg_fill = p.card_hover;
    v.widgets.hovered.weak_bg_fill = p.card_hover;
    v.widgets.hovered.bg_stroke = Stroke::new(1.0_f32, p.stroke);
    v.widgets.hovered.fg_stroke = Stroke::new(1.0_f32, p.text);

    v.widgets.active.bg_fill = p.accent_soft;
    v.widgets.active.weak_bg_fill = p.accent_soft;
    v.widgets.active.bg_stroke = Stroke::new(1.0_f32, p.accent);
    v.widgets.active.fg_stroke = Stroke::new(1.0_f32, p.text);

    style.visuals = v;
    style.spacing.item_spacing = Vec2::new(8.0, 8.0);
    style.spacing.button_padding = Vec2::new(12.0, 5.0);
    style.spacing.interact_size.y = 26.0;
    style.spacing.scroll.bar_width = 10.0;
    style.text_styles = [
        (TextStyle::Heading, FontId::new(19.0, FontFamily::Proportional)),
        (TextStyle::Body, FontId::new(14.0, FontFamily::Proportional)),
        (TextStyle::Button, FontId::new(14.0, FontFamily::Proportional)),
        (TextStyle::Small, FontId::new(12.0, FontFamily::Proportional)),
        (TextStyle::Monospace, FontId::new(13.0, FontFamily::Monospace)),
    ]
    .into();
    ctx.set_style(style);
}

// ---------------------------------------------------------------- 小部件

pub fn lerp_color(a: Color32, b: Color32, t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    let f = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgba_unmultiplied(
        f(a.r(), b.r()),
        f(a.g(), b.g()),
        f(a.b(), b.b()),
        f(a.a(), b.a()),
    )
}

/// 卡片容器
pub fn card<R>(ui: &mut Ui, p: &Palette, add: impl FnOnce(&mut Ui) -> R) -> R {
    let frame = egui::Frame::new()
        .fill(p.card)
        .stroke(Stroke::new(1.0_f32, p.stroke))
        .corner_radius(CornerRadius::same(10))
        .inner_margin(egui::Margin::symmetric(16, 14));
    frame.show(ui, |ui| add(ui)).inner
}

pub fn section(ui: &mut Ui, p: &Palette, title: &str, sub: Option<&str>) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(title).size(15.0).strong().color(p.text));
        if let Some(s) = sub {
            ui.label(RichText::new(s).size(12.0).color(p.text_dim));
        }
    });
    ui.add_space(4.0);
}

/// 状态徽标
pub fn badge(ui: &mut Ui, p: &Palette, text: &str, color: Color32) {
    let galley = ui.painter().layout_no_wrap(
        text.to_owned(),
        FontId::new(12.0, FontFamily::Proportional),
        color,
    );
    let size = galley.size() + Vec2::new(16.0, 6.0);
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    ui.painter()
        .rect_filled(rect, CornerRadius::same(5), color.gamma_multiply(0.14));
    ui.painter().galley(
        rect.center() - galley.size() * 0.5,
        galley,
        color,
    );
}

/// Win11 风格开关
pub fn toggle(ui: &mut Ui, p: &Palette, on: &mut bool) -> Response {
    let desired = Vec2::new(42.0, 22.0);
    let (rect, mut resp) = ui.allocate_exact_size(desired, Sense::click());
    if resp.clicked() {
        *on = !*on;
        resp.mark_changed();
    }
    let t = ui.ctx().animate_bool_responsive(resp.id, *on);
    let off = if p.dark {
        Color32::from_rgb(0x4A, 0x4A, 0x4A)
    } else {
        Color32::from_rgb(0xC9, 0xC9, 0xC9)
    };
    let bg = lerp_color(off, p.accent, t);
    let r = rect.height() * 0.5;
    ui.painter().rect_filled(rect, CornerRadius::same(r as u8), bg);
    let knob_r = r - 3.5;
    let cx = egui::lerp((rect.left() + r)..=(rect.right() - r), t);
    let knob = if *on { p.on_accent } else { Color32::WHITE };
    ui.painter()
        .circle_filled(egui::pos2(cx, rect.center().y), knob_r, knob);
    if resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    resp
}

/// 带阈值刻度的进度条
pub fn fill_bar(ui: &mut Ui, p: &Palette, fill: f32, threshold: f32, height: f32) {
    let width = ui.available_width().max(40.0);
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, height), Sense::hover());
    let r = CornerRadius::same((height * 0.5) as u8);
    ui.painter().rect_filled(rect, r, p.track);
    let w = (rect.width() * fill.clamp(0.0, 1.0)).max(0.0);
    if w > 1.0 {
        let c = if fill >= threshold { p.ok } else { p.accent };
        let fg = egui::Rect::from_min_size(rect.min, Vec2::new(w, rect.height()));
        ui.painter().rect_filled(fg, r, c);
    }
    // 阈值线
    let tx = rect.left() + rect.width() * threshold.clamp(0.0, 1.0);
    ui.painter().line_segment(
        [egui::pos2(tx, rect.top() - 2.0), egui::pos2(tx, rect.bottom() + 2.0)],
        Stroke::new(1.5, p.text_dim),
    );
}

/// 键位徽章（方块里一个字母）
pub fn key_cap(ui: &mut Ui, p: &Palette, key: &str) {
    let galley = ui.painter().layout_no_wrap(
        key.to_owned(),
        FontId::new(12.0, FontFamily::Proportional),
        p.text,
    );
    let size = Vec2::new(galley.size().x.max(10.0) + 12.0, 20.0);
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    ui.painter()
        .rect_filled(rect, CornerRadius::same(5), p.card_hover);
    ui.painter()
        .rect_stroke(rect, CornerRadius::same(5), Stroke::new(1.0_f32, p.stroke), egui::StrokeKind::Inside);
    ui.painter()
        .galley(rect.center() - galley.size() * 0.5, galley, p.text);
}

pub fn dim(ui: &mut Ui, p: &Palette, text: impl Into<String>) {
    ui.label(RichText::new(text.into()).size(12.0).color(p.text_dim));
}

/// 主按钮
pub fn primary_button(ui: &mut Ui, p: &Palette, text: &str) -> Response {
    let btn = egui::Button::new(RichText::new(text).size(13.0).color(p.on_accent))
        .fill(p.accent)
        .corner_radius(CornerRadius::same(6))
        .stroke(Stroke::NONE);
    let r = ui.add(btn);
    if r.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    r
}

/// 次级按钮
pub fn ghost_button(ui: &mut Ui, p: &Palette, text: &str) -> Response {
    let btn = egui::Button::new(RichText::new(text).size(13.0).color(p.text))
        .fill(p.card_hover)
        .corner_radius(CornerRadius::same(6))
        .stroke(Stroke::new(1.0_f32, p.stroke));
    let r = ui.add(btn);
    if r.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    r
}
