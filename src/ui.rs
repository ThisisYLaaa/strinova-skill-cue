//! 界面：Win11 风格（侧边导航 + 卡片 + 开关 + 实时预览）。

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use eframe::egui::{self, Color32, CornerRadius, RichText, Stroke, Ui, Vec2};

use crate::config::{
    self, Config, RectF, ThemeMode, APP_NAME, APP_NAME_EN, APP_VERSION, AUTHOR, AUTHOR_URL,
};
use crate::engine::{Cmd, Engine, LogKind, State};
use crate::theme::{self, Palette};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Page {
    Dashboard,
    Sounds,
    Characters,
    Detection,
    About,
}

impl Page {
    fn title(self) -> &'static str {
        match self {
            Page::Dashboard => "仪表盘",
            Page::Sounds => "音效",
            Page::Characters => "角色",
            Page::Detection => "检测",
            Page::About => "关于",
        }
    }
}

enum Req {
    PickGlobal(String),
    ClearGlobal(String),
    PickChar(String, String),
    ClearChar(String, String),
    Test(String),
    CaptureAvatar(String),
    ImportAvatar(String),
    StartBind(String),
    AutoCalibrate,
    ResetTemplates,
    StartEngine,
    StopEngine,
}

pub struct App {
    engine: Engine,
    page: Page,
    cfg: Config,
    dark: bool,
    textures: HashMap<String, (u64, egui::TextureHandle)>,
    binding: Option<String>,
    editing_char: Option<String>,
    char_filter: String,
    dirty: bool,
    last_change: Instant,
    last_save: Instant,
    toast: Option<(String, Instant)>,
    log_all: bool,
    st: State,
    snapshot: Option<crate::snapshot::Job>,
    titlebar_done: bool,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, cfg: Config) -> Self {
        theme::install_fonts(&cc.egui_ctx);
        let dark = match cfg.theme {
            ThemeMode::Dark => true,
            ThemeMode::Light => false,
            ThemeMode::System => theme::system_is_dark(),
        };
        let p = if dark { Palette::dark() } else { Palette::light() };
        theme::apply_style(&cc.egui_ctx, &p);

        let engine = Engine::start(cfg.clone());
        let editing_char = cfg.characters.first().map(|c| c.id.clone());
        Self {
            engine,
            page: Page::Dashboard,
            cfg,
            dark,
            textures: HashMap::new(),
            binding: None,
            editing_char,
            char_filter: String::new(),
            dirty: false,
            last_change: Instant::now(),
            last_save: Instant::now() - Duration::from_secs(10),
            toast: None,
            log_all: false,
            st: State::default(),
            snapshot: None,
            titlebar_done: false,
        }
    }

    /// 打开「界面自截图」：按页轮换，每页存一张 PNG，截完自动退出
    pub fn with_snapshot(mut self, dir: std::path::PathBuf) -> Self {
        self.snapshot = Some(crate::snapshot::Job::new(dir));
        self
    }

    fn palette(&self) -> Palette {
        if self.dark {
            Palette::dark()
        } else {
            Palette::light()
        }
    }

    fn touch(&mut self) {
        self.dirty = true;
        self.last_change = Instant::now();
    }

    fn toast(&mut self, msg: impl Into<String>) {
        self.toast = Some((msg.into(), Instant::now()));
    }

    fn flush_config(&mut self) {
        if !self.dirty || self.last_change.elapsed() < Duration::from_millis(200) {
            return;
        }
        self.dirty = false;
        let cfg = self.cfg.clone();
        self.engine.update_config(|c| *c = cfg.clone());
        self.cfg.save();
        self.last_save = Instant::now();
    }

    fn handle(&mut self, req: Req) {
        match req {
            Req::PickGlobal(id) => {
                if let Some(p) = pick_sound() {
                    let path = p.to_string_lossy().to_string();
                    if let Some(a) = self.cfg.action_mut(&id) {
                        a.sound = Some(path.clone());
                    }
                    self.touch();
                    self.engine.send(Cmd::ReloadSounds);
                    self.toast(format!("已设置音效: {}", crate::engine::short_path(&path)));
                    let _ = self.engine.send(Cmd::RefreshNow);
                }
            }
            Req::ClearGlobal(id) => {
                if let Some(a) = self.cfg.action_mut(&id) {
                    a.sound = None;
                }
                self.touch();
                self.engine.send(Cmd::ReloadSounds);
            }
            Req::PickChar(cid, aid) => {
                if let Some(p) = pick_sound() {
                    let path = p.to_string_lossy().to_string();
                    self.cfg.set_char_sound(&cid, &aid, Some(&path));
                    self.touch();
                    self.engine.send(Cmd::ReloadSounds);
                    self.toast(format!("已设置 {} 音效", crate::engine::short_path(&path)));
                }
            }
            Req::ClearChar(cid, aid) => {
                self.cfg.set_char_sound(&cid, &aid, None);
                self.touch();
                self.engine.send(Cmd::ReloadSounds);
            }
            Req::Test(path) => {
                self.engine.send(Cmd::PreviewSound(path));
            }
            Req::CaptureAvatar(cid) => {
                self.engine.send(Cmd::CaptureAvatar(cid));
                self.toast("已采集当前头像作为模板");
            }
            Req::ImportAvatar(cid) => {
                if let Some(path) = rfd::FileDialog::new()
                    .set_title("选择角色头像图片")
                    .add_filter("图片", &["png", "jpg", "jpeg", "webp", "bmp"])
                    .pick_file()
                {
                    match image::open(&path) {
                        Ok(img) => {
                            config::ensure_dirs();
                            let dst = config::avatars_dir().join(format!("{cid}.png"));
                            let rgba = img.to_rgba8();
                            match image::save_buffer(
                                &dst,
                                rgba.as_raw(),
                                rgba.width(),
                                rgba.height(),
                                image::ExtendedColorType::Rgba8,
                            ) {
                                Ok(_) => {
                                    self.engine.send(Cmd::ReloadTemplates);
                                    self.toast("已导入头像模板");
                                }
                                Err(e) => self.toast(format!("保存失败: {e}")),
                            }
                        }
                        Err(e) => self.toast(format!("图片读取失败: {e}")),
                    }
                }
            }
            Req::StartBind(id) => {
                self.binding = Some(id);
            }
            Req::AutoCalibrate => {
                self.engine.send(Cmd::AutoCalibrate);
                self.toast("已开始自动校准（请确保三个技能都攒满）");
            }
            Req::ResetTemplates => {
                self.engine.send(Cmd::ResetTemplates);
                self.toast("已清空头像模板");
            }
            Req::StartEngine => {
                self.cfg.enabled = true;
                self.touch();
                let _ = self.engine.send(Cmd::RefreshNow);
            }
            Req::StopEngine => {
                self.cfg.enabled = false;
                self.touch();
                self.engine.send(Cmd::StopAll);
            }
        }
    }

    fn update_textures(&mut self, ctx: &egui::Context) {
        let ids: Vec<String> = self.st.previews.keys().cloned().collect();
        for id in ids {
            let Some(pv) = self.st.previews.get(&id).cloned() else {
                continue;
            };
            if pv.w == 0 || pv.h == 0 || pv.rgba.len() < (pv.w * pv.h * 4) as usize {
                continue;
            }
            let stale = self
                .textures
                .get(&id)
                .map(|(v, _)| *v != pv.version)
                .unwrap_or(true);
            if stale {
                let img = egui::ColorImage::from_rgba_unmultiplied(
                    [pv.w as usize, pv.h as usize],
                    &pv.rgba,
                );
                let tex = ctx.load_texture(format!("pv-{id}"), img, egui::TextureOptions::NEAREST);
                self.textures.insert(id, (pv.version, tex));
            }
        }
    }

    // ------------------------------------------------------------ 标题栏

    fn title_bar(&mut self, ctx: &egui::Context, p: &Palette) {
        let bar_fill = if p.dark {
            Color32::from_rgb(0x27, 0x27, 0x27)
        } else {
            Color32::from_rgb(0xEC, 0xEC, 0xEC)
        };
        egui::TopBottomPanel::top("titlebar")
            .exact_height(40.0)
            .frame(egui::Frame::new().fill(bar_fill))
            .show(ctx, |ui| {
                let full = ui.max_rect();
                let btn_w = 46.0;
                let drag_area = egui::Rect::from_min_max(
                    full.min,
                    egui::pos2(full.right() - btn_w * 3.0, full.bottom()),
                );
                // 图标 + 标题
                let icon_rect = egui::Rect::from_min_size(
                    egui::pos2(full.left() + 14.0, full.center().y - 9.0),
                    Vec2::splat(18.0),
                );
                ui.painter()
                    .rect_filled(icon_rect, CornerRadius::same(5), p.accent);
                let c = icon_rect.center();
                for (i, h) in [5.0f32, 9.0, 7.0, 3.5].iter().enumerate() {
                    let x = c.x - 5.5 + i as f32 * 3.6;
                    ui.painter().rect_filled(
                        egui::Rect::from_center_size(egui::pos2(x, c.y), Vec2::new(2.0, *h)),
                        CornerRadius::same(1),
                        p.on_accent,
                    );
                }
                ui.painter().text(
                    egui::pos2(icon_rect.right() + 9.0, full.center().y),
                    egui::Align2::LEFT_CENTER,
                    format!("{} · {}", APP_NAME, APP_NAME_EN),
                    egui::FontId::proportional(12.5),
                    p.text_dim,
                );

                // 拖动 / 双击最大化
                let resp = ui.interact(
                    drag_area,
                    egui::Id::new("titlebar-drag"),
                    egui::Sense::click_and_drag(),
                );
                if resp.drag_started() {
                    ctx.send_viewport_cmd(egui::ViewportCommand::StartDrag);
                }
                if resp.double_clicked() {
                    let max = ctx.input(|i| i.viewport().maximized.unwrap_or(false));
                    ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(!max));
                }

                // 三个窗口按钮
                let kinds = [
                    egui::ViewportCommand::Minimized(true),
                    egui::ViewportCommand::Maximized(
                        !ctx.input(|i| i.viewport().maximized.unwrap_or(false)),
                    ),
                    egui::ViewportCommand::Close,
                ];
                for (i, cmd) in kinds.into_iter().enumerate() {
                    let rect = egui::Rect::from_min_size(
                        egui::pos2(full.right() - btn_w * (3.0 - i as f32), full.top()),
                        Vec2::new(btn_w, full.height()),
                    );
                    let r = ui.interact(rect, egui::Id::new(("cap", i)), egui::Sense::click());
                    let is_close = i == 2;
                    if r.hovered() {
                        let hover = if is_close {
                            Color32::from_rgb(0xC4, 0x2B, 0x1C)
                        } else {
                            p.card_hover
                        };
                        ui.painter().rect_filled(rect, CornerRadius::ZERO, hover);
                        ctx.set_cursor_icon(egui::CursorIcon::PointingHand);
                    }
                    let glyph = if is_close {
                        Color32::from_rgb(0xFF, 0xFF, 0xFF)
                    } else {
                        p.text
                    };
                    let cc = rect.center();
                    let s = 4.5;
                    match i {
                        0 => {
                            ui.painter().line_segment(
                                [egui::pos2(cc.x - s, cc.y), egui::pos2(cc.x + s, cc.y)],
                                Stroke::new(1.0_f32, glyph),
                            );
                        }
                        1 => {
                            ui.painter().rect_stroke(
                                egui::Rect::from_center_size(cc, Vec2::splat(s * 1.8)),
                                CornerRadius::same(1),
                                Stroke::new(1.0_f32, glyph),
                                egui::StrokeKind::Inside,
                            );
                        }
                        _ => {
                            ui.painter().line_segment(
                                [egui::pos2(cc.x - s, cc.y - s), egui::pos2(cc.x + s, cc.y + s)],
                                Stroke::new(1.2_f32, glyph),
                            );
                            ui.painter().line_segment(
                                [egui::pos2(cc.x - s, cc.y + s), egui::pos2(cc.x + s, cc.y - s)],
                                Stroke::new(1.2_f32, glyph),
                            );
                        }
                    }
                    if r.clicked() {
                        ctx.send_viewport_cmd(cmd);
                    }
                }
            });
    }

    // ------------------------------------------------------------ 侧栏

    fn sidebar(&mut self, ctx: &egui::Context, reqs: &mut Vec<Req>) {
        let p = self.palette();
        egui::SidePanel::left("nav")
            .exact_width(214.0)
            .resizable(false)
            .frame(
                egui::Frame::new()
                    .fill(if p.dark {
                        Color32::from_rgb(0x27, 0x27, 0x27)
                    } else {
                        Color32::from_rgb(0xEB, 0xEB, 0xEB)
                    })
                    .inner_margin(egui::Margin::symmetric(12, 14)),
            )
            .show(ctx, |ui| {
                // 品牌
                ui.horizontal(|ui| {
                    let (rect, _) = ui.allocate_exact_size(Vec2::splat(34.0), egui::Sense::hover());
                    ui.painter()
                        .rect_filled(rect, CornerRadius::same(9), p.accent);
                    let c = rect.center();
                    for (i, h) in [8.0f32, 15.0, 11.0, 6.0].iter().enumerate() {
                        let x = c.x - 10.0 + i as f32 * 6.5;
                        let bh = *h;
                        ui.painter().rect_filled(
                            egui::Rect::from_center_size(
                                egui::pos2(x, c.y),
                                Vec2::new(3.5, bh),
                            ),
                            CornerRadius::same(2),
                            p.on_accent,
                        );
                    }
                    ui.add_space(2.0);
                    ui.vertical(|ui| {
                        ui.label(RichText::new(APP_NAME).size(15.0).strong().color(p.text));
                        ui.label(RichText::new(APP_NAME_EN).size(10.5).color(p.text_dim));
                    });
                });
                ui.add_space(14.0);

                for page in [
                    Page::Dashboard,
                    Page::Sounds,
                    Page::Characters,
                    Page::Detection,
                    Page::About,
                ] {
                    let selected = self.page == page;
                    let h = 34.0;
                    let (rect, resp) = ui.allocate_exact_size(
                        Vec2::new(ui.available_width(), h),
                        egui::Sense::click(),
                    );
                    if resp.hovered() {
                        ui.painter()
                            .rect_filled(rect, CornerRadius::same(7), p.card_hover);
                        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                    }
                    if selected {
                        ui.painter()
                            .rect_filled(rect, CornerRadius::same(7), p.accent_soft);
                        let bar = egui::Rect::from_min_size(
                            egui::pos2(rect.left(), rect.center().y - 8.0),
                            Vec2::new(3.0, 16.0),
                        );
                        ui.painter().rect_filled(bar, CornerRadius::same(2), p.accent);
                    }
                    let color = if selected { p.accent } else { p.text };
                    ui.painter().text(
                        egui::pos2(rect.left() + 16.0, rect.center().y),
                        egui::Align2::LEFT_CENTER,
                        page.title(),
                        egui::FontId::proportional(13.5),
                        color,
                    );
                    if resp.clicked() {
                        self.page = page;
                    }
                }

                ui.with_layout(egui::Layout::bottom_up(egui::Align::LEFT), |ui| {
                    ui.label(
                        RichText::new(format!("v{APP_VERSION} · {AUTHOR}"))
                            .size(10.5)
                            .color(p.text_dim),
                    );
                    ui.add_space(4.0);
                    // 运行状态
                    let running = self.cfg.enabled;
                    let (rect, resp) =
                        ui.allocate_exact_size(Vec2::new(ui.available_width(), 40.0), egui::Sense::click());
                    ui.painter().rect_filled(
                        rect,
                        CornerRadius::same(8),
                        if running {
                            p.ok.gamma_multiply(if p.dark { 0.18 } else { 0.12 })
                        } else {
                            p.card_hover
                        },
                    );
                    let dot = egui::pos2(rect.left() + 16.0, rect.center().y);
                    ui.painter().circle_filled(
                        dot,
                        4.5,
                        if running { p.ok } else { p.text_dim },
                    );
                    ui.painter().text(
                        egui::pos2(dot.x + 12.0, dot.y - 6.0),
                        egui::Align2::LEFT_CENTER,
                        if running { "运行中" } else { "已暂停" },
                        egui::FontId::proportional(13.0),
                        p.text,
                    );
                    ui.painter().text(
                        egui::pos2(dot.x + 12.0, dot.y + 8.0),
                        egui::Align2::LEFT_CENTER,
                        if running { "点击暂停" } else { "点击启动" },
                        egui::FontId::proportional(10.5),
                        p.text_dim,
                    );
                    if resp.clicked() {
                        reqs.push(if running { Req::StopEngine } else { Req::StartEngine });
                    }
                    ui.add_space(6.0);
                });
            });
    }

    // ------------------------------------------------------------ 页面

    fn header(&self, ui: &mut Ui, p: &Palette, title: &str, desc: &str) {
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.label(RichText::new(title).size(22.0).strong().color(p.text));
                ui.label(RichText::new(desc).size(12.0).color(p.text_dim));
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                button_texture(ui, p, &self.st, &self.textures, "avatar", 34.0);
                ui.vertical(|ui| {
                    let name = self
                        .st
                        .current_char
                        .as_deref()
                        .map(|c| self.cfg.character_name(c))
                        .unwrap_or_else(|| "未识别角色".into());
                    ui.label(RichText::new(name).size(13.0).strong().color(p.text));
                    theme::dim(
                        ui,
                        p,
                        if self.st.current_char.is_some() {
                            format!("相似度 {:.0}% · {}", self.st.char_score * 100.0, self.st.char_at)
                        } else {
                            "等待识别…".into()
                        },
                    );
                });
            });
        });
        ui.add_space(12.0);
    }

    fn page_dashboard(&mut self, ui: &mut Ui, p: &Palette, reqs: &mut Vec<Req>) {
        let st = self.st.clone();
        self.header(ui, p, "仪表盘", "按键触发判定与音效状态一览");

        // 顶部状态条
        theme::card(ui, p, |ui| {
            ui.horizontal(|ui| {
                let mut on = self.cfg.enabled;
                if theme::toggle(ui, p, &mut on).changed() {
                    reqs.push(if on { Req::StartEngine } else { Req::StopEngine });
                }
                ui.add_space(6.0);
                ui.vertical(|ui| {
                    ui.label(
                        RichText::new(if self.cfg.enabled { "监听已开启" } else { "监听已暂停" })
                            .size(14.0)
                            .strong()
                            .color(if self.cfg.enabled { p.ok } else { p.text_dim }),
                    );
                    theme::dim(
                        ui,
                        p,
                        format!(
                            "{} · 黄条判定 · {} 个角色音效覆盖",
                            if st.capture_ok { "截图正常" } else { "截图不可用" },
                            self.cfg.char_sounds.len()
                        ),
                    );
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if theme::ghost_button(ui, p, "清空正在播放").clicked() {
                        self.engine.send(Cmd::StopAll);
                    }
                    theme::badge(
                        ui,
                        p,
                        if st.audio_ok { "音频就绪" } else { "音频异常" },
                        if st.audio_ok { p.ok } else { p.err },
                    );
                });
            });
        });
        ui.add_space(10.0);

        // 三个技能卡
        let actions = self.cfg.actions.clone();
        ui.columns(3, |cols| {
            for (i, a) in actions.iter().enumerate() {
                let ui = &mut cols[i.min(2)];
                let ast = st.actions.get(&a.id).cloned().unwrap_or_default();
                theme::card(ui, p, |ui| {
                    ui.horizontal(|ui| {
                        theme::key_cap(ui, p, &a.key);
                        ui.label(RichText::new(&a.label).size(14.0).strong().color(p.text));
                        ui.with_layout(
                            egui::Layout::right_to_left(egui::Align::Center),
                            |ui| {
                                if !a.enabled {
                                    theme::badge(ui, p, "已禁用", p.text_dim);
                                } else if ast.ready {
                                    theme::badge(ui, p, "就绪", p.ok);
                                } else {
                                    theme::badge(ui, p, "充能中", p.warn);
                                }
                            },
                        );
                    });
                    ui.add_space(8.0);
                    theme::fill_bar(ui, p, ast.fill, a.min_fill, 8.0);
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        theme::dim(ui, p, format!("黄条 {:.0}%", ast.fill * 100.0));
                        theme::dim(ui, p, "·");
                        theme::dim(ui, p, format!("阈值 {:.0}%", a.min_fill * 100.0));
                        if a.rect2.is_some() {
                            theme::dim(ui, p, "·");
                            theme::dim(ui, p, format!("备用 {:.0}%", ast.fill2 * 100.0));
                        }
                    });
                    ui.add_space(2.0);
                    let sound = self
                        .cfg
                        .resolve_sound(st.current_char.as_deref(), &a.id)
                        .map(|s| crate::engine::short_path(&s))
                        .unwrap_or_else(|| "未配置音效".into());
                    ui.label(
                        RichText::new(sound)
                            .size(11.5)
                            .color(if self
                                .cfg
                                .resolve_sound(st.current_char.as_deref(), &a.id)
                                .is_some()
                            {
                                p.text_dim
                            } else {
                                p.err
                            }),
                    );
                    ui.add_space(4.0);
                    theme::dim(
                        ui,
                        p,
                        format!(
                            "触发 {} 次 · 上次 {}",
                            ast.fired,
                            if ast.last_at.is_empty() { "-".into() } else { ast.last_at.clone() }
                        ),
                    );
                });
            }
        });
        ui.add_space(6.0);

        // 预览 + 日志
        ui.columns(2, |cols| {
            {
                let ui = &mut cols[0];
                theme::card(ui, p, |ui| {
                    theme::section(ui, p, "实时预览", Some("每 60ms 刷新"));
                    ui.add_space(6.0);
                    for a in &self.cfg.actions {
                        ui.horizontal(|ui| {
                            ui.allocate_ui(Vec2::new(64.0, 18.0), |ui| {
                                ui.label(RichText::new(&a.label).size(12.0).color(p.text_dim));
                            });
                            button_texture(
                                ui,
                                p,
                                &st,
                                &self.textures,
                                &a.id,
                                22.0,
                            );
                        });
                    }
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        ui.allocate_ui(Vec2::new(64.0, 18.0), |ui| {
                            ui.label(RichText::new("头像").size(12.0).color(p.text_dim));
                        });
                        button_texture(ui, p, &st, &self.textures, "avatar", 22.0);
                    });
                });
            }
            {
                let ui = &mut cols[1];
                theme::card(ui, p, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("最近事件").size(15.0).strong().color(p.text));
                        ui.with_layout(
                            egui::Layout::right_to_left(egui::Align::Center),
                            |ui| {
                                let mut all = self.log_all;
                                if ui
                                    .add(egui::Button::new(
                                        RichText::new(if all { "收起" } else { "全部" }).size(11.5),
                                    ))
                                    .clicked()
                                {
                                    all = !all;
                                    self.log_all = all;
                                }
                            },
                        );
                    });
                    ui.add_space(4.0);
                    let n = if self.log_all { st.events.len() } else { 6 };
                    egui::ScrollArea::vertical()
                        .max_height(if self.log_all { 260.0 } else { 150.0 })
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            for e in st.events.iter().rev().take(n) {
                                let c = match e.kind {
                                    LogKind::Trigger => p.ok,
                                    LogKind::Skip => p.text_dim,
                                    LogKind::Warn => p.warn,
                                    LogKind::Info => p.accent,
                                };
                                ui.horizontal(|ui| {
                                    ui.label(
                                        RichText::new(&e.time)
                                            .size(11.0)
                                            .color(p.text_dim)
                                            .monospace(),
                                    );
                                    ui.label(RichText::new(&e.text).size(11.5).color(c));
                                });
                            }
                            if st.events.is_empty() {
                                theme::dim(ui, p, "还没有事件。按一下技能键试试。");
                            }
                        });
                });
            }
        });
        ui.add_space(6.0);
        theme::card(ui, p, |ui| {
            ui.horizontal(|ui| {
                theme::badge(
                    ui,
                    p,
                    if st.audio_ok { "音频引擎" } else { "音频引擎异常" },
                    if st.audio_ok { p.ok } else { p.err },
                );
                theme::dim(ui, p, st.audio_device.clone());
            });
            if let Some(err) = &st.audio_err {
                ui.label(RichText::new(err).size(11.5).color(p.err));
            }
            ui.add_space(2.0);
            ui.horizontal(|ui| {
                theme::dim(
                    ui,
                    p,
                    format!(
                        "触发 {} 次 · 最近延迟 {:.1} ms · 模板 {} 个 · 屏幕 {}x{}",
                        st.triggers,
                        st.last_latency_ms,
                        st.templates,
                        st.screen.0,
                        st.screen.1
                    ),
                );
            });
        });
    }

    fn page_sounds(&mut self, ui: &mut Ui, p: &Palette, reqs: &mut Vec<Req>) {
        self.header(ui, p, "全局音效", "未给具体角色单独配置时，使用这里的音效");
        theme::card(ui, p, |ui| {
            theme::section(ui, p, "技能音效", Some("按键就绪即播放，从 0:00 开始"));
            ui.add_space(8.0);
            let actions = self.cfg.actions.clone();
            for a in &actions {
                ui.horizontal(|ui| {
                    ui.allocate_ui(Vec2::new(96.0, 24.0), |ui| {
                        ui.horizontal(|ui| {
                            theme::key_cap(ui, p, &a.key);
                            ui.label(RichText::new(&a.label).size(13.0).color(p.text));
                        });
                    });
                    let name = a
                        .sound
                        .as_deref()
                        .map(crate::engine::short_path)
                        .unwrap_or_else(|| "未配置".into());
                    let exists = a
                        .sound
                        .as_deref()
                        .map(|s| std::path::Path::new(s).exists())
                        .unwrap_or(false);
                    ui.label(
                        RichText::new(name)
                            .size(12.5)
                            .color(if exists { p.text } else { p.text_dim }),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if theme::ghost_button(ui, p, "选择…").clicked() {
                            reqs.push(Req::PickGlobal(a.id.clone()));
                        }
                        if a.sound.is_some() {
                            if theme::ghost_button(ui, p, "试听").clicked() {
                                if let Some(s) = &a.sound {
                                    reqs.push(Req::Test(s.clone()));
                                }
                            }
                            if theme::ghost_button(ui, p, "清除").clicked() {
                                reqs.push(Req::ClearGlobal(a.id.clone()));
                            }
                        }
                    });
                });
                ui.add_space(4.0);
            }
        });
        ui.add_space(10.0);
        theme::card(ui, p, |ui| {
            theme::section(ui, p, "音量与播放", None);
            ui.add_space(6.0);
            let mut vol = self.cfg.volume;
            if ui
                .add(egui::Slider::new(&mut vol, 0.0..=1.5).text("音量"))
                .changed()
            {
                self.cfg.volume = vol;
                self.engine.send(Cmd::SetVolume(vol));
                self.touch();
            }
            let mut cooldown = self.cfg.cooldown_ms as i32;
            if ui
                .add(
                    egui::Slider::new(&mut cooldown, 0..=1500)
                        .text("触发冷却 (ms)")
                        .suffix(" ms"),
                )
                .changed()
            {
                self.cfg.cooldown_ms = cooldown.max(0) as u64;
                self.touch();
            }
            theme::dim(
                ui,
                p,
                "支持 mp3 / wav / m4a / flac / ogg / aac，文件在配置时即预解码，触发瞬间直接送声卡。",
            );
        });
    }

    fn page_characters(&mut self, ui: &mut Ui, p: &Palette, reqs: &mut Vec<Req>) {
        self.header(ui, p, "角色音效", "为每个超弦体单独指定音效（优先于全局）");
        ui.horizontal_top(|ui| {
            // 左：角色列表
            ui.allocate_ui_with_layout(
                Vec2::new(300.0, ui.available_height()),
                egui::Layout::top_down(egui::Align::LEFT),
                |ui| {
                theme::card(ui, p, |ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut self.char_filter)
                            .hint_text("搜索角色…")
                            .desired_width(f32::INFINITY),
                    );
                    ui.add_space(6.0);
                    egui::ScrollArea::vertical()
                        .auto_shrink([false, false])
                        .max_height(ui.available_height() - 10.0)
                        .show(ui, |ui| {
                            let filter = self.char_filter.to_lowercase();
                            let list: Vec<(String, String)> = self
                                .cfg
                                .characters
                                .iter()
                                .filter(|c| {
                                    filter.is_empty()
                                        || c.name.to_lowercase().contains(&filter)
                                        || c.id.to_lowercase().contains(&filter)
                                })
                                .map(|c| (c.id.clone(), c.name.clone()))
                                .collect();
                            if list.is_empty() {
                                theme::dim(ui, p, "没有角色。可以到「检测」页采集头像。");
                            }
                            for (id, name) in list {
                                let selected =
                                    self.editing_char.as_deref() == Some(id.as_str());
                                let has_tpl = self
                                    .st
                                    .previews
                                    .contains_key("avatar"); // 仅用于提示
                                let overrides = self
                                    .cfg
                                    .char_sounds
                                    .get(&id)
                                    .map(|m| m.len())
                                    .unwrap_or(0);
                                let label = if overrides > 0 {
                                    format!("{name}  ·{overrides}")
                                } else {
                                    name.clone()
                                };
                                if ui
                                    .add(
                                        egui::Button::selectable(selected, label)
                                            .min_size(Vec2::new(ui.available_width(), 26.0)),
                                    )
                                    .clicked()
                                {
                                    self.editing_char = Some(id.clone());
                                }
                                let _ = has_tpl;
                            }
                        });
                });
            });
            ui.add_space(8.0);
            // 右：选中角色
            ui.allocate_ui_with_layout(
                Vec2::new(ui.available_width(), ui.available_height()),
                egui::Layout::top_down(egui::Align::LEFT),
                |ui| {
                    self.char_detail(ui, p, reqs);
                },
            );
        });
    }

    fn char_detail(&mut self, ui: &mut Ui, p: &Palette, reqs: &mut Vec<Req>) {
        let Some(cid) = self.editing_char.clone() else {
            theme::card(ui, p, |ui| theme::dim(ui, p, "左侧选择一个角色"));
            return;
        };
        let name = self.cfg.character_name(&cid);
        let actions = self.cfg.actions.clone();
        theme::card(ui, p, |ui| {
            ui.horizontal(|ui| {
                let tpl = config::avatars_dir().join(format!("{cid}.png")).exists();
                theme::badge(
                    ui,
                    p,
                    if tpl { "已有头像模板" } else { "缺头像模板" },
                    if tpl { p.ok } else { p.warn },
                );
                ui.label(RichText::new(&name).size(16.0).strong().color(p.text));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if theme::primary_button(ui, p, "采集当前头像").clicked() {
                        reqs.push(Req::CaptureAvatar(cid.clone()));
                    }
                    if theme::ghost_button(ui, p, "从图片导入…").clicked() {
                        reqs.push(Req::ImportAvatar(cid.clone()));
                    }
                });
            });
            ui.add_space(8.0);
            let stale = self
                .textures
                .get("avatar")
                .map(|_| ())
                .is_some();
            if stale {
                button_texture(ui, p, &self.st, &self.textures, "avatar", 48.0);
            } else {
                theme::dim(ui, p, "在游戏里显示该角色后点「采集当前头像」。");
            }
        });
        ui.add_space(8.0);
        theme::card(ui, p, |ui| {
            theme::section(ui, p, "技能音效", Some("留空＝跟随全局"));
            ui.add_space(8.0);
            for a in &actions {
                ui.horizontal(|ui| {
                    ui.allocate_ui(Vec2::new(110.0, 24.0), |ui| {
                        ui.horizontal(|ui| {
                            theme::key_cap(ui, p, &a.key);
                            ui.label(RichText::new(&a.label).size(13.0).color(p.text));
                        });
                    });
                    let own = self
                        .cfg
                        .char_sound(&cid, &a.id)
                        .map(|s| s.to_string());
                    let global = a.sound.clone();
                    let shown = own
                        .clone()
                        .or_else(|| global.clone())
                        .map(|s| crate::engine::short_path(&s))
                        .unwrap_or_else(|| "未配置".into());
                    let is_own = own.is_some();
                    ui.label(
                        RichText::new(shown)
                            .size(12.5)
                            .color(if is_own { p.accent } else { p.text_dim }),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if theme::ghost_button(ui, p, "选择…").clicked() {
                            reqs.push(Req::PickChar(cid.clone(), a.id.clone()));
                        }
                        if let Some(own) = own.clone() {
                            if theme::ghost_button(ui, p, "试听").clicked() {
                                reqs.push(Req::Test(own.clone()));
                            }
                            if theme::ghost_button(ui, p, "跟随全局").clicked() {
                                reqs.push(Req::ClearChar(cid.clone(), a.id.clone()));
                            }
                        }
                    });
                });
                ui.add_space(2.0);
            }
        });
    }

    fn page_detection(&mut self, ui: &mut Ui, p: &Palette, reqs: &mut Vec<Req>) {
        self.header(ui, p, "检测设置", "截图区域、判定阈值与按键绑定");
        let (sw, sh) = self.st.screen;
        let (sw, sh) = (sw.max(1), sh.max(1));

        theme::card(ui, p, |ui| {
            theme::section(ui, p, "角色头像识别", Some("定时截取左下角头像并与模板比对"));
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                let mut auto = self.cfg.auto_detect_avatar;
                if theme::toggle(ui, p, &mut auto).changed() {
                    self.cfg.auto_detect_avatar = auto;
                    self.touch();
                }
                ui.label(RichText::new("自动识别角色").size(13.0).color(p.text));
                ui.add_space(16.0);
                theme::dim(
                    ui,
                    p,
                    format!("模板 {} 个 · 上次 {}", self.st.templates, self.st.char_at),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if theme::ghost_button(ui, p, "清空模板").clicked() {
                        reqs.push(Req::ResetTemplates);
                    }
                    if theme::ghost_button(ui, p, "立即识别").clicked() {
                        self.engine.send(Cmd::RefreshNow);
                    }
                });
            });
            ui.add_space(6.0);
            let mut secs = (self.cfg.avatar_interval_ms / 1000).max(1) as i32;
            ui.horizontal(|ui| {
                ui.allocate_ui(Vec2::new(110.0, 20.0), |ui| {
                    ui.label(RichText::new("识别间隔").size(12.5).color(p.text_dim));
                });
                if ui
                    .add(
                        egui::Slider::new(&mut secs, 1..=120)
                            .suffix(" s")
                            .trailing_fill(true),
                    )
                    .changed()
                {
                    self.cfg.avatar_interval_ms = (secs as u64) * 1000;
                    self.touch();
                }
            });
            let mut thr = self.cfg.avatar_threshold;
            ui.horizontal(|ui| {
                ui.allocate_ui(Vec2::new(110.0, 20.0), |ui| {
                    ui.label(RichText::new("匹配阈值").size(12.5).color(p.text_dim));
                });
                if ui
                    .add(egui::Slider::new(&mut thr, 0.5..=0.98).fixed_decimals(2))
                    .changed()
                {
                    self.cfg.avatar_threshold = thr;
                    self.touch();
                }
            });
            ui.horizontal(|ui| {
                ui.allocate_ui(Vec2::new(110.0, 20.0), |ui| {
                    ui.label(RichText::new("锁定角色").size(12.5).color(p.text_dim));
                });
                let cur = self
                    .cfg
                    .locked_character
                    .clone()
                    .map(|id| self.cfg.character_name(&id))
                    .unwrap_or_else(|| "自动".into());
                egui::ComboBox::from_id_salt("lock-char")
                    .selected_text(cur)
                    .show_ui(ui, |ui| {
                        if ui
                            .selectable_label(self.cfg.locked_character.is_none(), "自动")
                            .clicked()
                        {
                            self.cfg.locked_character = None;
                            self.touch();
                        }
                        let list = self.cfg.characters.clone();
                        for c in list {
                            if ui
                                .selectable_label(
                                    self.cfg.locked_character.as_deref() == Some(c.id.as_str()),
                                    &c.name,
                                )
                                .clicked()
                            {
                                self.cfg.locked_character = Some(c.id.clone());
                                self.touch();
                            }
                        }
                    });
            });
            ui.add_space(6.0);
            rect_editor(ui, p, "头像区域", &mut self.cfg.avatar_rect, sw, sh);
            ui.add_space(4.0);
            if self.st.previews.contains_key("avatar") {
                button_texture(ui, p, &self.st, &self.textures, "avatar", 44.0);
            }
        });

        ui.add_space(10.0);
        let mut actions = self.cfg.actions.clone();
        theme::card(ui, p, |ui| {
            ui.horizontal(|ui| {
                theme::section(ui, p, "技能黄条判定", Some("按下按键后检查黄色进度条是否充满"));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if theme::primary_button(ui, p, "自动校准区域").clicked() {
                        reqs.push(Req::AutoCalibrate);
                    }
                });
            });
            ui.add_space(10.0);
            for a in &mut actions {
                ui.push_id(&a.id, |ui| {
                    egui::Frame::new()
                        .fill(if p.dark {
                            Color32::from_rgb(0x24, 0x24, 0x24)
                        } else {
                            Color32::from_rgb(0xFA, 0xFA, 0xFA)
                        })
                        .corner_radius(CornerRadius::same(8))
                        .stroke(Stroke::new(1.0, p.stroke))
                        .inner_margin(egui::Margin::symmetric(12, 10))
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                theme::key_cap(ui, p, &a.key);
                                ui.label(
                                    RichText::new(&a.label).size(13.5).strong().color(p.text),
                                );
                                let mut on = a.enabled;
                                if theme::toggle(ui, p, &mut on).changed() {
                                    a.enabled = on;
                                    self.touch();
                                }
                                let mut check = a.check_bar;
                                if ui
                                    .add(egui::Checkbox::new(&mut check, "做黄条判定"))
                                    .changed()
                                {
                                    a.check_bar = check;
                                    self.touch();
                                }
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        let binding = self.binding.as_deref() == Some(a.id.as_str());
                                        let text = if binding {
                                            "请按键…".to_string()
                                        } else {
                                            format!("改键 ({})", a.key)
                                        };
                                        if theme::ghost_button(ui, p, &text).clicked() {
                                            reqs.push(Req::StartBind(a.id.clone()));
                                        }
                                    },
                                );
                            });
                            ui.add_space(6.0);
                            ui.horizontal(|ui| {
                                ui.allocate_ui(Vec2::new(80.0, 20.0), |ui| {
                                    theme::dim(ui, p, "黄色阈值");
                                });
                                let mut mf = a.min_fill * 100.0;
                                if ui
                                    .add(
                                        egui::Slider::new(&mut mf, 2.0..=95.0)
                                            .suffix(" %")
                                            .fixed_decimals(0),
                                    )
                                    .changed()
                                {
                                    a.min_fill = (mf / 100.0).clamp(0.02, 0.95);
                                    self.touch();
                                }
                            });
                            ui.add_space(4.0);
                            rect_editor(ui, p, "黄条区域", &mut a.rect, sw, sh);
                            ui.add_space(4.0);
                            // 备用判定区域：不同角色的「就绪」表现可能不一样
                            let mut has_alt = a.rect2.is_some();
                            if ui
                                .add(egui::Checkbox::new(&mut has_alt, "启用备用判定区域"))
                                .changed()
                            {
                                a.rect2 = if has_alt {
                                    a.min_fill2 = 0.06;
                                    Some(RectF::from_px(
                                        (a.rect.x * sw as f32) as i32,
                                        ((a.rect.y - a.rect.h * 2.0) * sh as f32).max(0.0) as i32,
                                        (a.rect.w * sw as f32).max(8.0) as u32,
                                        (a.rect.h * sh as f32 * 2.0).max(8.0) as u32,
                                        sw,
                                        sh,
                                    ))
                                } else {
                                    None
                                };
                                self.touch();
                            }
                            if a.rect2.is_some() {
                                if let Some(r2) = a.rect2.as_mut() {
                                    rect_editor(ui, p, "备用区域", r2, sw, sh);
                                }
                                ui.horizontal(|ui| {
                                    ui.allocate_ui(Vec2::new(80.0, 20.0), |ui| {
                                        theme::dim(ui, p, "备用阈值");
                                    });
                                    let mut mf2 = a.min_fill2 * 100.0;
                                    if ui
                                        .add(
                                            egui::Slider::new(&mut mf2, 0.5..=95.0)
                                                .suffix(" %")
                                                .fixed_decimals(1),
                                        )
                                        .changed()
                                    {
                                        a.min_fill2 = (mf2 / 100.0).clamp(0.005, 0.95);
                                        self.touch();
                                    }
                                });
                                if self.st.previews.contains_key(&format!("{}-alt", a.id)) {
                                    ui.horizontal(|ui| {
                                        ui.allocate_ui(Vec2::new(80.0, 20.0), |ui| {
                                            theme::dim(ui, p, "备用画面");
                                        });
                                        button_texture(
                                            ui,
                                            p,
                                            &self.st,
                                            &self.textures,
                                            &format!("{}-alt", a.id),
                                            26.0,
                                        );
                                    });
                                }
                            }
                            ui.add_space(6.0);
                            ui.horizontal(|ui| {
                                if self.st.previews.contains_key(&a.id) {
                                    button_texture(ui, p, &self.st, &self.textures, &a.id, 26.0);
                                }
                                let ast = self.st.actions.get(&a.id).cloned().unwrap_or_default();
                                let mut txt = format!(
                                    "当前黄色 {:.1}% {}",
                                    ast.fill * 100.0,
                                    if ast.ready && !ast.alt { "（就绪）" } else { "" }
                                );
                                if a.rect2.is_some() {
                                    txt.push_str(&format!(
                                        " · 备用 {:.1}%{}",
                                        ast.fill2 * 100.0,
                                        if ast.alt { "（就绪）" } else { "" }
                                    ));
                                }
                                theme::dim(ui, p, txt);
                            });
                        });
                });
                ui.add_space(8.0);
            }
        });
        self.cfg.actions = actions;

        ui.add_space(10.0);
        theme::card(ui, p, |ui| {
            theme::section(ui, p, "高级", None);
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                let mut v = self.cfg.ignore_when_focused;
                if theme::toggle(ui, p, &mut v).changed() {
                    self.cfg.ignore_when_focused = v;
                    self.touch();
                }
                ui.label(RichText::new("本工具窗口在前台时不触发").size(12.5).color(p.text));
            });
            ui.horizontal(|ui| {
                let mut v = self.cfg.preview_enabled;
                if theme::toggle(ui, p, &mut v).changed() {
                    self.cfg.preview_enabled = v;
                    self.touch();
                }
                ui.label(RichText::new("开启实时预览截图").size(12.5).color(p.text));
            });
            ui.horizontal(|ui| {
                let mut v = self.cfg.play_fallback;
                if theme::toggle(ui, p, &mut v).changed() {
                    self.cfg.play_fallback = v;
                    self.touch();
                }
                ui.label(RichText::new("截图失败时也播放").size(12.5).color(p.text));
            });
            ui.add_space(4.0);
            theme::dim(
                ui,
                p,
                format!(
                    "配置文件: {}",
                    config::config_path().to_string_lossy()
                ),
            );
        });
    }

    fn page_about(&mut self, ui: &mut Ui, p: &Palette) {
        self.header(ui, p, "关于", "Screenshot-driven skill audio cue");
        theme::card(ui, p, |ui| {
            ui.horizontal(|ui| {
                let (rect, _) = ui.allocate_exact_size(Vec2::splat(56.0), egui::Sense::hover());
                ui.painter()
                    .rect_filled(rect, CornerRadius::same(14), p.accent);
                let c = rect.center();
                for (i, h) in [12.0f32, 24.0, 18.0, 9.0].iter().enumerate() {
                    let x = c.x - 16.0 + i as f32 * 11.0;
                    ui.painter().rect_filled(
                        egui::Rect::from_center_size(egui::pos2(x, c.y), Vec2::new(6.0, *h)),
                        CornerRadius::same(3),
                        p.on_accent,
                    );
                }
                ui.add_space(6.0);
                ui.vertical(|ui| {
                    ui.label(RichText::new(APP_NAME).size(20.0).strong().color(p.text));
                    ui.label(
                        RichText::new(format!("{APP_NAME_EN} · v{APP_VERSION}"))
                            .size(12.0)
                            .color(p.text_dim),
                    );
                });
            });
            ui.add_space(12.0);
            ui.separator();
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                ui.allocate_ui(Vec2::new(72.0, 20.0), |ui| {
                    ui.label(RichText::new("作者").size(13.0).color(p.text_dim));
                });
                ui.label(RichText::new(AUTHOR).size(13.0).strong().color(p.text));
                ui.add_space(8.0);
                ui.hyperlink_to("哔哩哔哩主页", AUTHOR_URL);
            });
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.allocate_ui(Vec2::new(72.0, 20.0), |ui| {
                    ui.label(RichText::new("开源").size(13.0).color(p.text_dim));
                });
                ui.hyperlink_to(
                    "github.com/ThisisYLaaa/strinova-skill-cue",
                    "https://github.com/ThisisYLaaa/strinova-skill-cue",
                );
            });
            ui.add_space(14.0);
            theme::section(ui, p, "工作方式", None);
            ui.add_space(4.0);
            for line in [
                "· 全局监听技能/大招键（默认 Q / X），不注入、不改键、不读写游戏内存。",
                "· 按键瞬间截取技能条区域，统计黄色占比，达到阈值才认为「放出来了」。",
                "· 判定通过立刻播放音效，音频在保存配置时就已解码进内存，从 0:00 起播。",
                "· 每隔 N 秒截一次左下角头像，与角色模板做归一化互相关比对，识别当前超弦体。",
            ] {
                ui.label(RichText::new(line).size(12.5).color(p.text_dim));
            }
            ui.add_space(10.0);
            theme::section(ui, p, "注意", None);
            ui.add_space(4.0);
            for line in [
                "· 游戏请使用「无边框窗口」或「窗口化」，独占全屏下 GDI 截屏可能拿不到画面。",
                "· 本工具只读屏幕，不修改游戏任何数据；是否使用请自行判断，风险自负。",
                "· 首次使用请到「检测」页确认三个技能条区域对齐，或点「自动校准区域」。",
            ] {
                ui.label(RichText::new(line).size(12.5).color(p.text_dim));
            }
            ui.add_space(10.0);
            theme::dim(ui, p, "MIT License · 用 Rust + egui 写的小工具");
        });
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // 同步状态
        self.st = self.engine.snapshot();
        if let Ok(mut s) = self.engine.state.lock() {
            s.focused = ctx.input(|i| i.focused);
        }
        // 深浅色跟随
        if self.cfg.theme == ThemeMode::System {
            let d = theme::system_is_dark();
            if d != self.dark {
                self.dark = d;
                theme::apply_style(ctx, &self.palette());
                self.titlebar_done = false;
            }
        }
        if !self.titlebar_done {
            self.titlebar_done = crate::snapshot::set_dark_titlebar(self.dark);
        }
        self.update_textures(ctx);

        // 键位绑定
        if let Some(id) = self.binding.clone() {
            let picked = ctx.input(|i| {
                i.events.iter().find_map(|e| match e {
                    egui::Event::Key {
                        key,
                        pressed: true,
                        ..
                    } => Some(*key),
                    _ => None,
                })
            });
            if let Some(k) = picked {
                if k == egui::Key::Escape {
                    self.binding = None;
                } else {
                    let name = k.name().to_uppercase();
                    if let Some(a) = self.cfg.action_mut(&id) {
                        a.key = name.clone();
                    }
                    self.binding = None;
                    self.touch();
                    self.toast(format!("已绑定按键 {name}"));
                }
            }
        }

        let p = self.palette();
        let mut reqs: Vec<Req> = Vec::new();
        self.title_bar(ctx, &p);

        // 界面自截图模式：切页 + 截屏 + 收工
        if let Some(job) = self.snapshot.as_mut() {
            self.page = match job.page {
                0 => Page::Dashboard,
                1 => Page::Sounds,
                2 => Page::Characters,
                3 => Page::Detection,
                _ => Page::About,
            };
            let done = job.step();
            if done {
                self.snapshot = None;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            } else {
                ctx.request_repaint_after(Duration::from_millis(16));
            }
        }

        self.sidebar(ctx, &mut reqs);

        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(p.bg)
                    .inner_margin(egui::Margin::symmetric(22, 18)),
            )
            .show(ctx, |ui| {
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| match self.page {
                        Page::Dashboard => self.page_dashboard(ui, &p, &mut reqs),
                        Page::Sounds => self.page_sounds(ui, &p, &mut reqs),
                        Page::Characters => self.page_characters(ui, &p, &mut reqs),
                        Page::Detection => self.page_detection(ui, &p, &mut reqs),
                        Page::About => self.page_about(ui, &p),
                    });
            });

        for r in reqs {
            self.handle(r);
        }
        self.flush_config();

        // 提示条
        if let Some((msg, at)) = self.toast.clone() {
            if at.elapsed() < Duration::from_millis(2200) {
                egui::Area::new(egui::Id::new("toast"))
                    .anchor(egui::Align2::CENTER_BOTTOM, egui::vec2(0.0, -24.0))
                    .show(ctx, |ui| {
                        egui::Frame::new()
                            .fill(p.accent)
                            .corner_radius(CornerRadius::same(8))
                            .inner_margin(egui::Margin::symmetric(14, 8))
                            .show(ui, |ui| {
                                ui.label(RichText::new(msg).size(12.5).color(p.on_accent));
                            });
                    });
                ctx.request_repaint_after(Duration::from_millis(120));
            } else {
                self.toast = None;
            }
        }

        ctx.request_repaint_after(Duration::from_millis(120));
    }
}

fn pick_sound() -> Option<PathBuf> {
    rfd::FileDialog::new()
        .set_title("选择音效文件")
        .add_filter("音频", &["mp3", "wav", "m4a", "flac", "ogg", "aac"])
        .pick_file()
}

/// 显示某个区域的实时截图（按高度等比缩放）
fn button_texture(
    ui: &mut Ui,
    p: &Palette,
    st: &State,
    textures: &HashMap<String, (u64, egui::TextureHandle)>,
    id: &str,
    height: f32,
) {
    let Some((_, tex)) = textures.get(id) else {
        let (rect, _) = ui.allocate_exact_size(Vec2::new(height * 3.0, height), egui::Sense::hover());
        ui.painter().rect_filled(
            rect,
            CornerRadius::same(4),
            p.track,
        );
        ui.painter().text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            "无画面",
            egui::FontId::proportional(10.0),
            p.text_dim,
        );
        return;
    };
    let pv = st.previews.get(id);
    let aspect = pv
        .map(|v| v.w as f32 / v.h.max(1) as f32)
        .unwrap_or(3.0)
        .clamp(0.4, 12.0);
    let size = Vec2::new((height * aspect).max(16.0), height);
    let img = egui::Image::new(egui::load::SizedTexture::new(tex.id(), size))
        .corner_radius(CornerRadius::same(4));
    ui.add(img);
}

/// 归一化矩形 <-> 像素编辑器
fn rect_editor(ui: &mut Ui, p: &Palette, label: &str, rect: &mut RectF, sw: u32, sh: u32) {
    let (mut x, mut y, mut w, mut h) = rect.to_px(sw, sh);
    let (mut wi, mut hi) = (w as i32, h as i32);
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.allocate_ui(Vec2::new(80.0, 20.0), |ui| {
            ui.label(RichText::new(label).size(12.0).color(p.text_dim));
        });
        for (name, val) in [
            ("x", &mut x),
            ("y", &mut y),
        ] {
            theme::dim(ui, p, name);
            changed |= ui
                .add(egui::DragValue::new(val).speed(1.0).range(-2000..=8000))
                .changed();
        }
        theme::dim(ui, p, "w");
        changed |= ui
            .add(egui::DragValue::new(&mut wi).speed(1.0).range(1..=4000))
            .changed();
        theme::dim(ui, p, "h");
        changed |= ui
            .add(egui::DragValue::new(&mut hi).speed(1.0).range(1..=4000))
            .changed();
        theme::dim(ui, p, format!("px ({sw}x{sh})"));
    });
    if changed {
        *rect = RectF::from_px(x, y, wi.max(1) as u32, hi.max(1) as u32, sw, sh);
        rect.clamp();
    }
}
