//! 引擎：工作线程里串起「按键 -> 截图判定 -> 找音效 -> 立刻播放」，
//! 同时负责角色头像定时识别、UI 预览帧、日志与统计。

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::audio::AudioEngine;
use crate::capture::{screen_size, Capturer, Frame};
use crate::config::{is_sound_file, Config};
use crate::hook::{self, PressFilter, VkEvent};
use crate::vision::{self, Template};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LogKind {
    Info,
    Trigger,
    Skip,
    Warn,
}

#[derive(Clone, Debug)]
pub struct LogEvent {
    pub time: String,
    pub kind: LogKind,
    pub text: String,
}

#[derive(Clone, Debug, Default)]
pub struct ActionState {
    pub fill: f32,
    pub ready: bool,
    pub fired: u32,
    pub last_at: String,
    pub latency_ms: f32,
    pub have_sample: bool,
}

#[derive(Clone, Debug, Default)]
pub struct Preview {
    pub w: u32,
    pub h: u32,
    pub rgba: Vec<u8>,
    pub version: u64,
}

#[derive(Clone, Debug, Default)]
pub struct State {
    pub audio_ok: bool,
    pub audio_device: String,
    pub audio_err: Option<String>,
    pub capture_ok: bool,
    pub screen: (u32, u32),
    pub current_char: Option<String>,
    pub char_score: f32,
    pub char_at: String,
    pub actions: BTreeMap<String, ActionState>,
    pub events: VecDeque<LogEvent>,
    pub previews: HashMap<String, Preview>,
    pub triggers: u64,
    pub last_latency_ms: f32,
    pub templates: usize,
    pub focused: bool,
    pub version: u64,
}

#[derive(Debug)]
pub enum Cmd {
    Key(VkEvent),
    SetVolume(f32),
    ReloadSounds,
    CaptureAvatar(String),
    RefreshNow,
    StopAll,
    AutoCalibrate,
    ResetTemplates,
    PreviewSound(String),
    ReloadTemplates,
    Quit,
}

pub struct Engine {
    pub cfg: Arc<Mutex<Config>>,
    pub state: Arc<Mutex<State>>,
    pub tx: Sender<Cmd>,
    pub screen: (u32, u32),
}

impl Engine {
    pub fn start(cfg: Config) -> Self {
        let screen = screen_size();
        let cfg = Arc::new(Mutex::new(cfg));
        let state = Arc::new(Mutex::new(State {
            screen,
            ..Default::default()
        }));
        let (tx, rx) = channel::<Cmd>();
        let _ = spawn_hook_into(tx.clone());

        let (c, s) = (cfg.clone(), state.clone());
        std::thread::Builder::new()
            .name("engine".into())
            .spawn(move || {
                let mut w = Worker::new(c.clone(), s.clone());
                w.run(rx);
                let _ = (c, s);
            })
            .ok();

        Self {
            cfg,
            state,
            tx,
            screen,
        }
    }

    pub fn send(&self, cmd: Cmd) {
        let _ = self.tx.send(cmd);
    }

    pub fn snapshot(&self) -> State {
        self.state.lock().map(|s| s.clone()).unwrap_or_default()
    }

    pub fn config(&self) -> Config {
        self.cfg.lock().map(|c| c.clone()).unwrap_or_default()
    }

    pub fn update_config(&self, f: impl FnOnce(&mut Config)) {
        if let Ok(mut c) = self.cfg.lock() {
            f(&mut c);
            c.normalize();
        }
        self.send(Cmd::ReloadSounds);
    }
}

/// 把钩子事件转发成 Cmd::Key
fn spawn_hook_into(tx: Sender<Cmd>) -> bool {
    let (vtx, vrx) = channel::<VkEvent>();
    if !hook::spawn(vtx) {
        return false;
    }
    std::thread::Builder::new()
        .name("kbd-fwd".into())
        .spawn(move || {
            while let Ok(ev) = vrx.recv() {
                if tx.send(Cmd::Key(ev)).is_err() {
                    break;
                }
            }
        })
        .is_ok()
}

struct Worker {
    cfg: Arc<Mutex<Config>>,
    state: Arc<Mutex<State>>,
    cap: Option<Capturer>,
    audio: Option<AudioEngine>,
    templates: Vec<Template>,
    press: PressFilter,
    last_fire: HashMap<String, Instant>,
    last_avatar: Instant,
    last_preview: Instant,
    frame_seq: u64,
    sounds_loaded: bool,
}

impl Worker {
    fn new(cfg: Arc<Mutex<Config>>, state: Arc<Mutex<State>>) -> Self {
        let mut w = Self {
            cfg,
            state,
            cap: Capturer::new(),
            audio: None,
            templates: Vec::new(),
            press: PressFilter::default(),
            last_fire: HashMap::new(),
            last_avatar: Instant::now() - Duration::from_secs(3600),
            last_preview: Instant::now() - Duration::from_secs(3600),
            frame_seq: 0,
            sounds_loaded: false,
        };
        let (vol, interval) = {
            let c = w.cfg.lock().unwrap();
            (c.volume, c.avatar_interval_ms)
        };
        match AudioEngine::new(vol) {
            Ok(a) => {
                if let Ok(mut s) = w.state.lock() {
                    s.audio_ok = true;
                    s.audio_device = format!(
                        "{} {} Hz / {} ch",
                        a.device_name, a.sample_rate, a.channels
                    );
                    s.audio_err = None;
                }
                w.audio = Some(a);
            }
            Err(e) => {
                if let Ok(mut s) = w.state.lock() {
                    s.audio_ok = false;
                    s.audio_err = Some(e.clone());
                }
                w.log(LogKind::Warn, format!("音频设备不可用: {e}"));
            }
        }
        if let Ok(mut s) = w.state.lock() {
            s.capture_ok = w.cap.is_some();
        }
        w.reload_templates();
        w.reload_sounds();
        let _ = interval;
        w
    }

    fn run(&mut self, rx: Receiver<Cmd>) {
        loop {
            match rx.recv_timeout(Duration::from_millis(15)) {
                Ok(cmd) => match cmd {
                    Cmd::Quit => break,
                    Cmd::Key(ev) => self.on_key(ev),
                    Cmd::SetVolume(v) => {
                        if let Some(a) = &self.audio {
                            a.set_volume(v);
                        }
                    }
                    Cmd::ReloadSounds => self.reload_sounds(),
                    Cmd::ReloadTemplates => self.reload_templates(),
                    Cmd::CaptureAvatar(id) => self.capture_avatar(&id),
                    Cmd::ResetTemplates => {
                        let _ = std::fs::remove_dir_all(crate::config::avatars_dir());
                        let _ = std::fs::create_dir_all(crate::config::avatars_dir());
                        self.reload_templates();
                        self.log(LogKind::Info, "已清空头像模板".into());
                    }
                    Cmd::RefreshNow => {
                        self.last_preview = Instant::now() - Duration::from_secs(60);
                        self.last_avatar = Instant::now() - Duration::from_secs(3600);
                    }
                    Cmd::StopAll => {
                        if let Some(a) = &self.audio {
                            a.stop_all();
                        }
                    }
                    Cmd::AutoCalibrate => self.auto_calibrate(),
                    Cmd::PreviewSound(p) => {
                        let path = std::path::PathBuf::from(&p);
                        match self.audio.as_mut() {
                            Some(a) => match a.play(&path, 1.0) {
                                Ok(_) => self.log(
                                    LogKind::Info,
                                    format!("试听 {}", short_path(&p)),
                                ),
                                Err(e) => self
                                    .log(LogKind::Warn, format!("试听失败: {e}")),
                            },
                            None => self.log(LogKind::Warn, "音频设备不可用".into()),
                        }
                    }
                },
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
            }
            self.tick();
        }
    }

    fn tick(&mut self) {
        let (interval, preview_on) = {
            let c = self.cfg.lock().unwrap();
            (c.avatar_interval_ms, c.preview_enabled)
        };
        if self.last_avatar.elapsed() >= Duration::from_millis(interval) {
            self.last_avatar = Instant::now();
            self.sample_avatar();
        }
        if preview_on && self.last_preview.elapsed() >= Duration::from_millis(60) {
            self.last_preview = Instant::now();
            self.refresh_previews();
        }
    }

    // ---------------------------------------------------------------- 按键

    fn on_key(&mut self, ev: VkEvent) {
        if !ev.down {
            self.press.release(ev.vk);
            return;
        }
        if !self.press.press(ev.vk) {
            return; // 长按自动重复
        }
        let cfg = { self.cfg.lock().unwrap().clone() };
        if !cfg.enabled {
            return;
        }
        let state = self.state.lock().map(|s| s.focused).unwrap_or(false);
        if cfg.ignore_when_focused && state {
            return;
        }
        let key_name = hook::vk_name(ev.vk);
        let action = match cfg.action_by_key(&key_name) {
            Some(a) => a.clone(),
            None => return,
        };
        if let Some(prev) = self.last_fire.get(&action.id) {
            if prev.elapsed() < Duration::from_millis(cfg.cooldown_ms) {
                return;
            }
        }

        let screen = self.screen();
        let mut stats = None;
        let mut cap_failed = false;
        if action.check_bar {
            let (x, y, w, h) = action.rect.to_px(screen.0, screen.1);
            match self.cap.as_mut().and_then(|c| c.grab(x, y, w, h)) {
                Some(f) => stats = Some(vision::analyze_bar(&f)),
                None => cap_failed = true,
            }
        }

        let ready = match stats {
            Some(st) => st.ready(action.min_fill),
            None => cfg.play_fallback || !action.check_bar,
        };

        // 更新动作状态
        let fill = stats.map(|s| s.fill).unwrap_or(1.0);
        if let Ok(mut s) = self.state.lock() {
            let e = s.actions.entry(action.id.clone()).or_default();
            e.fill = fill;
            e.ready = ready;
            e.have_sample = stats.is_some();
        }

        if !ready {
            self.log(
                LogKind::Skip,
                format!(
                    "{} [{}] 未就绪 · 黄条 {:.0}% < 阈值 {:.0}%{}",
                    action.label,
                    action.key,
                    fill * 100.0,
                    action.min_fill * 100.0,
                    if cap_failed { " · 截图失败" } else { "" }
                ),
            );
            return;
        }

        let cur = self.state.lock().ok().and_then(|s| s.current_char.clone());
        let path = cfg.resolve_sound(cur.as_deref(), &action.id);
        let gain = 1.0;
        let played = match (&path, self.audio.as_mut()) {
            (Some(p), Some(a)) => match a.play(std::path::Path::new(p), gain) {
                Ok(_) => true,
                Err(e) => {
                    self.log(LogKind::Warn, format!("播放失败: {e}"));
                    false
                }
            },
            (Some(_), None) => false,
            (None, _) => false,
        };
        let latency = ev.at.elapsed().as_secs_f32() * 1000.0;
        self.last_fire.insert(action.id.clone(), Instant::now());
        if let Ok(mut s) = self.state.lock() {
            let e = s.actions.entry(action.id.clone()).or_default();
            e.fired += 1;
            e.latency_ms = latency;
            e.last_at = now_hms();
            s.triggers += 1;
            s.last_latency_ms = latency;
            s.version += 1;
        }
        let who = cur
            .as_deref()
            .map(|c| cfg.character_name(c))
            .unwrap_or_else(|| "全局".into());
        let msg = if played {
            format!(
                "{} [{}] 触发 · {} · {}",
                action.label,
                action.key,
                who,
                path.as_deref()
                    .map(|p| short_path(p))
                    .unwrap_or_else(|| "未配置音效".into())
            )
        } else {
            format!("{} [{}] 就绪但没有可播放的音效（{}）", action.label, action.key, who)
        };
        self.log(if played { LogKind::Trigger } else { LogKind::Warn }, msg);
    }

    // ---------------------------------------------------------------- 头像

    fn sample_avatar(&mut self) {
        let (rect, threshold, auto, locked) = {
            let c = self.cfg.lock().unwrap();
            (
                c.avatar_rect,
                c.avatar_threshold,
                c.auto_detect_avatar,
                c.locked_character.clone(),
            )
        };
        let screen = self.screen();
        let (x, y, w, h) = rect.to_px(screen.0, screen.1);
        let frame = match self.cap.as_mut().and_then(|c| c.grab(x, y, w, h)) {
            Some(f) => f,
            None => return,
        };
        if !vision::sane_rect(frame.w, frame.h) {
            return;
        }
        self.publish_preview("avatar", &frame);

        if let Some(id) = locked {
            if let Ok(mut s) = self.state.lock() {
                s.current_char = Some(id);
                s.char_score = 1.0;
                s.char_at = now_hms();
            }
            return;
        }
        if !auto || self.templates.is_empty() {
            return;
        }
        let sig = vision::avatar_signature(&frame, 0.10);
        if let Some((tpl, score)) = vision::best_match(&sig, &self.templates) {
            let (id, name) = (tpl.id.clone(), tpl.name.clone());
            let mut accepted = score >= threshold;
            if let Ok(mut s) = self.state.lock() {
                // 低于阈值就保留上一次的识别结果，避免闪断
                if accepted {
                    let changed = s.current_char.as_deref() != Some(id.as_str());
                    s.current_char = Some(id.clone());
                    s.char_score = score;
                    s.char_at = now_hms();
                    if changed {
                        s.version += 1;
                    }
                } else {
                    s.char_score = score;
                }
            }
            if accepted {
                self.log(LogKind::Info, format!("识别到角色 {name}（相似度 {:.2}）", score));
            } else {
                accepted = false;
                let _ = accepted;
            }
        }
    }

    fn capture_avatar(&mut self, id: &str) {
        let rect = { self.cfg.lock().unwrap().avatar_rect };
        let screen = self.screen();
        let (x, y, w, h) = rect.to_px(screen.0, screen.1);
        let frame = match self.cap.as_mut().and_then(|c| c.grab(x, y, w, h)) {
            Some(f) => f,
            None => {
                self.log(LogKind::Warn, "采集头像失败：截图不可用".into());
                return;
            }
        };
        crate::config::ensure_dirs();
        let path = crate::config::avatars_dir().join(format!("{id}.png"));
        match image::save_buffer(
            &path,
            &frame.rgba,
            frame.w,
            frame.h,
            image::ExtendedColorType::Rgba8,
        ) {
            Ok(_) => {
                self.log(
                    LogKind::Info,
                    format!("已采集头像模板: {}", short_path(&path.to_string_lossy())),
                );
                self.reload_templates();
                if let Ok(mut s) = self.state.lock() {
                    s.current_char = Some(id.to_string());
                    s.char_score = 1.0;
                    s.char_at = now_hms();
                }
            }
            Err(e) => self.log(LogKind::Warn, format!("保存头像失败: {e}")),
        }
    }

    fn reload_templates(&mut self) {
        let mut list = Vec::new();
        let dir = crate::config::avatars_dir();
        let names: HashMap<String, String> = self
            .cfg
            .lock()
            .map(|c| {
                c.characters
                    .iter()
                    .map(|ch| (ch.id.clone(), ch.name.clone()))
                    .collect()
            })
            .unwrap_or_default();
        if let Ok(rd) = std::fs::read_dir(&dir) {
            for e in rd.flatten() {
                let p = e.path();
                if p.extension().and_then(|s| s.to_str()).map(|s| s.eq_ignore_ascii_case("png"))
                    != Some(true)
                {
                    continue;
                }
                let id = p
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("unknown")
                    .to_string();
                match load_frame(&p) {
                    Ok(f) => {
                        let sig = vision::avatar_signature(&f, 0.10);
                        let name = names.get(&id).cloned().unwrap_or_else(|| id.clone());
                        list.push(Template { id, name, vec: sig });
                    }
                    Err(e) => self.log(LogKind::Warn, format!("头像模板读取失败: {e}")),
                }
            }
        }
        let n = list.len();
        self.templates = list;
        if let Ok(mut s) = self.state.lock() {
            s.templates = n;
        }
    }

    fn reload_sounds(&mut self) {
        let cfg = { self.cfg.lock().unwrap().clone() };
        let mut paths: Vec<PathBuf> = Vec::new();
        for a in &cfg.actions {
            if let Some(p) = &a.sound {
                if is_sound_file(std::path::Path::new(p)) {
                    paths.push(PathBuf::from(p));
                }
            }
        }
        for m in cfg.char_sounds.values() {
            for p in m.values() {
                if is_sound_file(std::path::Path::new(p)) {
                    paths.push(PathBuf::from(p));
                }
            }
        }
        paths.sort();
        paths.dedup();
        if let Some(a) = self.audio.as_mut() {
            let errs = a.preload(&paths);
            for (p, e) in errs {
                self.log(
                    LogKind::Warn,
                    format!("音效加载失败 {}: {e}", short_path(&p.to_string_lossy())),
                );
            }
        }
        self.sounds_loaded = true;
    }

    // ---------------------------------------------------------------- 预览

    fn refresh_previews(&mut self) {
        let cfg = { self.cfg.lock().unwrap().clone() };
        let screen = self.screen();
        let mut items: Vec<(String, (i32, i32, u32, u32))> = cfg
            .actions
            .iter()
            .map(|a| (a.id.clone(), a.rect.to_px(screen.0, screen.1)))
            .collect();
        items.push(("avatar".into(), cfg.avatar_rect.to_px(screen.0, screen.1)));
        for (id, (x, y, w, h)) in items {
            if let Some(f) = self.cap.as_mut().and_then(|c| c.grab(x, y, w, h)) {
                if id != "avatar" {
                    let st = vision::analyze_bar(&f);
                    if let Ok(mut s) = self.state.lock() {
                        let a = s.actions.entry(id.clone()).or_default();
                        a.fill = st.fill;
                        a.ready = st.ready(
                            cfg.action(&id).map(|a| a.min_fill).unwrap_or(0.5),
                        );
                        a.have_sample = true;
                    }
                }
                self.publish_preview(&id, &f);
            }
        }
    }

    fn publish_preview(&mut self, id: &str, f: &Frame) {
        self.frame_seq += 1;
        if let Ok(mut s) = self.state.lock() {
            s.previews.insert(
                id.to_string(),
                Preview {
                    w: f.w,
                    h: f.h,
                    rgba: f.rgba.clone(),
                    version: self.frame_seq,
                },
            );
        }
    }

    // ---------------------------------------------------------------- 自动校准

    fn auto_calibrate(&mut self) {
        let screen = self.screen();
        // 底部中间 30%~72% 宽、93%~100% 高的一条带
        let bx = (screen.0 as f32 * 0.30) as i32;
        let by = (screen.1 as f32 * 0.93) as i32;
        let bw = (screen.0 as f32 * 0.42) as u32;
        let bh = (screen.1 as f32 * 0.07).max(20.0) as u32;
        let frame = match self.cap.as_mut().and_then(|c| c.grab(bx, by, bw, bh)) {
            Some(f) => f,
            None => {
                self.log(LogKind::Warn, "自动校准失败：无法截图".into());
                return;
            }
        };
        let clusters = find_yellow_clusters(&frame);
        if clusters.len() < 2 {
            self.log(
                LogKind::Warn,
                format!(
                    "自动校准失败：只找到 {} 条黄条。请先把三个技能都攒满再校准。",
                    clusters.len()
                ),
            );
            return;
        }
        // 按 x 排序，依次对应 技能/大招/战术
        let order = ["skill", "ultimate", "tactical"];
        let mut msg = String::from("自动校准:");
        {
            let mut cfg = self.cfg.lock().unwrap();
            for (i, c) in clusters.iter().take(3).enumerate() {
                let id = order[i];
                // 上下各留 1px 余量
                let x = bx + c.0 as i32;
                let y = by + c.1 as i32 - 1;
                let w = c.2.max(4);
                let h = (c.3 + 2).max(6);
                let rect = crate::config::RectF::from_px(x, y, w, h, screen.0, screen.1);
                if let Some(a) = cfg.action_mut(id) {
                    a.rect = rect;
                    msg.push_str(&format!(" {}→{}x{}+{},{}", a.label, w, h, x, y));
                }
            }
            cfg.normalize();
            drop(cfg);
        }
        self.log(LogKind::Info, msg);
        self.last_preview = Instant::now() - Duration::from_secs(60);
    }

    // ---------------------------------------------------------------- 工具

    fn screen(&self) -> (u32, u32) {
        self.state.lock().map(|s| s.screen).unwrap_or((2560, 1440))
    }

    fn log(&mut self, kind: LogKind, text: String) {
        let limit = self.cfg.lock().map(|c| c.log_limit).unwrap_or(200);
        if let Ok(mut s) = self.state.lock() {
            s.events.push_back(LogEvent {
                time: now_hms(),
                kind,
                text,
            });
            while s.events.len() > limit {
                s.events.pop_front();
            }
            s.version += 1;
        }
    }
}

/// 在截图里找黄色横条，返回 (x, y, w, h) 以像素为单位（相对截图左上）
fn find_yellow_clusters(f: &Frame) -> Vec<(u32, u32, u32, u32)> {
    let (w, h) = (f.w as usize, f.h as usize);
    if w == 0 || h == 0 {
        return Vec::new();
    }
    let mut col_counts = vec![0u32; w];
    let mut col_top = vec![u32::MAX; w];
    let mut col_bot = vec![0u32; w];
    for y in 0..h {
        for x in 0..w {
            let i = (y * w + x) * 4;
            let (hh, s, v) = rgb_hsv(f.rgba[i], f.rgba[i + 1], f.rgba[i + 2]);
            if (32.0..=72.0).contains(&hh) && s >= 0.30 && v >= 0.40 {
                col_counts[x] += 1;
                if (y as u32) < col_top[x] {
                    col_top[x] = y as u32;
                }
                if (y as u32) > col_bot[x] {
                    col_bot[x] = y as u32;
                }
            }
        }
    }
    let mut groups: Vec<(usize, usize)> = Vec::new();
    let mut start: Option<usize> = None;
    let mut gap = 0;
    for x in 0..w {
        let on = col_counts[x] >= 2;
        if on {
            if start.is_none() {
                start = Some(x);
            }
            gap = 0;
        } else if let Some(s) = start {
            gap += 1;
            if gap > 24 {
                groups.push((s, x - gap));
                start = None;
            }
        }
    }
    if let Some(s) = start {
        groups.push((s, w - 1));
    }
    let mut out = Vec::new();
    for (x0, x1) in groups {
        if x1 <= x0 {
            continue;
        }
        let mut top = u32::MAX;
        let mut bot = 0u32;
        let mut px = 0u32;
        for x in x0..=x1 {
            px += col_counts[x];
            if col_counts[x] > 0 {
                top = top.min(col_top[x]);
                bot = bot.max(col_bot[x]);
            }
        }
        if px < 40 || top == u32::MAX {
            continue;
        }
        out.push((x0 as u32, top, (x1 - x0 + 1) as u32, bot - top + 1));
    }
    out.sort_by_key(|c| c.0);
    out
}

fn rgb_hsv(r: u8, g: u8, b: u8) -> (f32, f32, f32) {
    let rf = r as f32 / 255.0;
    let gf = g as f32 / 255.0;
    let bf = b as f32 / 255.0;
    let mx = rf.max(gf).max(bf);
    let mn = rf.min(gf).min(bf);
    let d = mx - mn;
    let s = if mx > 0.0 { d / mx } else { 0.0 };
    if d <= f32::EPSILON {
        return (0.0, s, mx);
    }
    let mut hh = if (mx - rf).abs() < f32::EPSILON {
        60.0 * ((gf - bf) / d)
    } else if (mx - gf).abs() < f32::EPSILON {
        60.0 * ((bf - rf) / d) + 120.0
    } else {
        60.0 * ((rf - gf) / d) + 240.0
    };
    if hh < 0.0 {
        hh += 360.0;
    }
    (hh, s, mx)
}

fn load_frame(path: &std::path::Path) -> Result<Frame, String> {
    let img = image::open(path).map_err(|e| e.to_string())?.to_rgba8();
    Ok(Frame {
        w: img.width(),
        h: img.height(),
        rgba: img.into_raw(),
    })
}

pub fn short_path(p: &str) -> String {
    let path = std::path::Path::new(p);
    path.file_name()
        .map(|f| f.to_string_lossy().to_string())
        .unwrap_or_else(|| p.to_string())
}

pub fn now_hms() -> String {
    unsafe {
        use windows::Win32::Foundation::SYSTEMTIME;
        use windows::Win32::System::SystemInformation::GetLocalTime;
        let st: SYSTEMTIME = GetLocalTime();
        format!("{:02}:{:02}:{:02}", st.wHour, st.wMinute, st.wSecond)
    }
}
