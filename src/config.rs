//! 配置模型与默认值。
//!
//! 所有屏幕区域都存成「相对于主显示器分辨率的归一化坐标」（0.0~1.0），
//! 这样换分辨率 / 窗口化都不会失准。默认值来自 2560x1440 实机截图的像素测量。

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub const APP_NAME: &str = "超弦体技能音效";
pub const APP_NAME_EN: &str = "Strinova Skill Cue";
pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");
pub const AUTHOR: &str = "Leaked-翼龙";
pub const AUTHOR_URL: &str = "https://space.bilibili.com/616045770";
pub const CONFIG_VERSION: u32 = 1;

/// 归一化矩形：x/y/w/h 都是 0..1
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct RectF {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl RectF {
    pub const fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        Self { x, y, w, h }
    }

    /// 换算成当前屏幕像素
    pub fn to_px(&self, sw: u32, sh: u32) -> (i32, i32, u32, u32) {
        let sw = sw.max(1) as f32;
        let sh = sh.max(1) as f32;
        let x = (self.x * sw).round();
        let y = (self.y * sh).round();
        let w = (self.w * sw).round();
        let h = (self.h * sh).round();
        (x as i32, y as i32, w.max(1.0) as u32, h.max(1.0) as u32)
    }

    pub fn from_px(x: i32, y: i32, w: u32, h: u32, sw: u32, sh: u32) -> Self {
        let sw = sw.max(1) as f32;
        let sh = sh.max(1) as f32;
        Self {
            x: (x as f32 / sw).clamp(0.0, 1.0),
            y: (y as f32 / sh).clamp(0.0, 1.0),
            w: (w as f32 / sw).clamp(0.0, 1.0),
            h: (h as f32 / sh).clamp(0.0, 1.0),
        }
    }

    pub fn clamp(&mut self) {
        self.x = self.x.clamp(0.0, 0.999);
        self.y = self.y.clamp(0.0, 0.999);
        self.w = self.w.clamp(0.001, 1.0 - self.x);
        self.h = self.h.clamp(0.001, 1.0 - self.y);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThemeMode {
    System,
    Light,
    Dark,
}

impl Default for ThemeMode {
    fn default() -> Self {
        Self::System
    }
}

/// 一个可触发音效的技能槽
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct ActionCfg {
    /// 稳定 id：skill / ultimate / tactical
    pub id: String,
    pub label: String,
    /// 绑定的按键（大写字母/数字/F1..F12 等）
    pub key: String,
    /// 判定区域（黄条）
    pub rect: RectF,
    /// 黄色占比达到多少算「已就绪」
    pub min_fill: f32,
    /// 全局音效（未给当前角色单独配置时使用）
    pub sound: Option<String>,
    /// 是否做黄条检查（关掉＝按键即响，用于调试）
    pub check_bar: bool,
    pub enabled: bool,
}

impl Default for ActionCfg {
    fn default() -> Self {
        Self {
            id: String::new(),
            label: String::new(),
            key: "Q".into(),
            rect: RectF::new(0.0, 0.0, 0.01, 0.01),
            min_fill: 0.5,
            sound: None,
            check_bar: true,
            enabled: true,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct CharacterCfg {
    pub id: String,
    pub name: String,
    /// 是否参与角色识别
    pub enabled: bool,
}

impl Default for CharacterCfg {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: String::new(),
            enabled: true,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub version: u32,
    /// 总开关
    pub enabled: bool,
    pub theme: ThemeMode,
    /// 0.0 ~ 1.5
    pub volume: f32,
    /// 同一技能两次触发的间隔下限（毫秒），防抖
    pub cooldown_ms: u64,
    pub actions: Vec<ActionCfg>,
    /// 角色头像识别区域
    pub avatar_rect: RectF,
    /// 头像识别间隔（毫秒），默认 10000
    pub avatar_interval_ms: u64,
    /// 头像匹配阈值 0..1
    pub avatar_threshold: f32,
    pub auto_detect_avatar: bool,
    /// 手动锁定角色（None = 自动识别）
    pub locked_character: Option<String>,
    pub characters: Vec<CharacterCfg>,
    /// 角色 -> 技能 -> 音效
    pub char_sounds: BTreeMap<String, BTreeMap<String, String>>,
    /// 本工具窗口在前台时忽略按键
    pub ignore_when_focused: bool,
    /// 截图失败 / 未配置音效时也播放默认音
    pub play_fallback: bool,
    /// 日志条数上限
    pub log_limit: usize,
    /// 预览刷新（UI 里看实时截图）
    pub preview_enabled: bool,
    /// 首次启动引导是否已看过
    pub onboarded: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            version: CONFIG_VERSION,
            enabled: true,
            theme: ThemeMode::System,
            volume: 0.9,
            cooldown_ms: 250,
            actions: default_actions(),
            // 2560x1440 实测：头像框 (44,1309)-(124,1388)
            avatar_rect: RectF::new(0.0171875, 0.9090278, 0.03125, 0.0548611),
            avatar_interval_ms: 10_000,
            avatar_threshold: 0.80,
            auto_detect_avatar: true,
            locked_character: None,
            characters: Vec::new(),
            char_sounds: BTreeMap::new(),
            ignore_when_focused: true,
            play_fallback: false,
            log_limit: 200,
            preview_enabled: true,
            onboarded: false,
        }
    }
}

/// Q/X/C 三个默认技能槽，区域来自截图实测（2560x1440）
pub fn default_actions() -> Vec<ActionCfg> {
    vec![
        ActionCfg {
            id: "skill".into(),
            label: "技能".into(),
            key: "Q".into(),
            // (994,1401)-(1089,1410)
            rect: RectF::new(0.3882813, 0.9729167, 0.0371094, 0.0062500),
            min_fill: 0.80,
            sound: None,
            check_bar: true,
            enabled: true,
        },
        ActionCfg {
            id: "ultimate".into(),
            label: "大招".into(),
            key: "X".into(),
            // (1224,1374)-(1340,1396) 大招充能三角行
            rect: RectF::new(0.4781250, 0.9541667, 0.0453125, 0.0152778),
            min_fill: 0.06,
            sound: None,
            check_bar: true,
            enabled: true,
        },
        ActionCfg {
            id: "tactical".into(),
            label: "战术技能".into(),
            key: "C".into(),
            // (1462,1394)-(1606,1410)
            rect: RectF::new(0.5710938, 0.9680556, 0.0562500, 0.0111111),
            min_fill: 0.18,
            sound: None,
            check_bar: true,
            enabled: true,
        },
    ]
}

impl Config {
    pub fn action(&self, id: &str) -> Option<&ActionCfg> {
        self.actions.iter().find(|a| a.id == id)
    }
    pub fn action_mut(&mut self, id: &str) -> Option<&mut ActionCfg> {
        self.actions.iter_mut().find(|a| a.id == id)
    }
    pub fn action_by_key(&self, key: &str) -> Option<&ActionCfg> {
        self.actions.iter().find(|a| a.enabled && a.key.eq_ignore_ascii_case(key))
    }
    pub fn character_name(&self, id: &str) -> String {
        self.characters
            .iter()
            .find(|c| c.id == id)
            .map(|c| c.name.clone())
            .unwrap_or_else(|| id.to_string())
    }
    /// 角色级音效覆盖
    pub fn char_sound(&self, character: &str, action: &str) -> Option<&str> {
        self.char_sounds
            .get(character)
            .and_then(|m| m.get(action))
            .map(|s| s.as_str())
            .filter(|s| !s.is_empty())
    }
    pub fn set_char_sound(&mut self, character: &str, action: &str, path: Option<&str>) {
        match path {
            Some(p) if !p.is_empty() => {
                self.char_sounds
                    .entry(character.to_string())
                    .or_default()
                    .insert(action.to_string(), p.to_string());
            }
            _ => {
                if let Some(m) = self.char_sounds.get_mut(character) {
                    m.remove(action);
                    if m.is_empty() {
                        self.char_sounds.remove(character);
                    }
                }
            }
        }
    }
    /// 最终生效的音效：角色覆盖 > 全局
    pub fn resolve_sound(&self, character: Option<&str>, action: &str) -> Option<String> {
        if let Some(c) = character {
            if let Some(p) = self.char_sound(c, action) {
                return Some(p.to_string());
            }
        }
        self.action(action).and_then(|a| a.sound.clone()).filter(|s| !s.is_empty())
    }
}

// ---------------------------------------------------------------- 路径

pub fn data_dir() -> PathBuf {
    let base = std::env::var("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."));
    base.join("StrinovaSkillCue")
}

pub fn config_path() -> PathBuf {
    data_dir().join("config.json")
}

pub fn avatars_dir() -> PathBuf {
    data_dir().join("avatars")
}

pub fn sounds_dir() -> PathBuf {
    data_dir().join("sounds")
}

/// 保证数据目录存在
pub fn ensure_dirs() {
    for d in [data_dir(), avatars_dir(), sounds_dir()] {
        let _ = std::fs::create_dir_all(d);
    }
}

impl Config {
    pub fn load() -> Self {
        let p = config_path();
        match std::fs::read_to_string(&p) {
            Ok(text) => match serde_json::from_str::<Config>(&text) {
                Ok(mut c) => {
                    c.normalize();
                    c
                }
                Err(e) => {
                    eprintln!("配置解析失败({e})，已备份并重建：{}", p.display());
                    let _ = std::fs::copy(&p, p.with_extension("json.bak"));
                    Config::default()
                }
            },
            Err(_) => Config::default(),
        }
    }

    pub fn save(&self) {
        ensure_dirs();
        let p = config_path();
        match serde_json::to_string_pretty(self) {
            Ok(text) => {
                let tmp = p.with_extension("json.tmp");
                if std::fs::write(&tmp, text.as_bytes()).is_ok() {
                    let _ = std::fs::rename(&tmp, &p);
                }
            }
            Err(e) => eprintln!("配置序列化失败: {e}"),
        }
    }

    /// 修正非法值 / 补齐缺失字段
    pub fn normalize(&mut self) {
        self.version = CONFIG_VERSION;
        self.volume = self.volume.clamp(0.0, 1.5);
        self.avatar_threshold = self.avatar_threshold.clamp(0.1, 0.99);
        self.avatar_interval_ms = self.avatar_interval_ms.clamp(500, 600_000);
        self.cooldown_ms = self.cooldown_ms.clamp(0, 5000);
        if self.log_limit < 20 {
            self.log_limit = 200;
        }
        self.avatar_rect.clamp();
        if self.actions.is_empty() {
            self.actions = default_actions();
        }
        // 保证三个内置槽位存在
        for d in default_actions() {
            if self.action(&d.id).is_none() {
                self.actions.push(d);
            }
        }
        for a in &mut self.actions {
            if a.key.trim().is_empty() {
                a.key = "Q".into();
            }
            a.key = a.key.trim().to_uppercase();
            a.rect.clamp();
            a.min_fill = a.min_fill.clamp(0.01, 0.99);
        }
        self.characters.sort_by(|a, b| a.name.cmp(&b.name));
        self.characters.dedup_by(|a, b| a.id == b.id);
    }
}

/// 从内置 assets/characters.json 读角色名册（只有名字和官方立绘链接，不含图片）
#[derive(Debug, Clone, Deserialize)]
pub struct RosterEntry {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub name_en: Option<String>,
    #[serde(default)]
    pub art_url: Option<String>,
    #[serde(default)]
    pub icon_url: Option<String>,
}

pub fn bundled_roster() -> Vec<RosterEntry> {
    // 编译期嵌入，随 exe 走
    const RAW: &str = include_str!("../assets/characters.json");
    #[derive(Deserialize)]
    struct Doc {
        #[serde(default)]
        characters: Vec<RosterEntry>,
    }
    match serde_json::from_str::<Doc>(RAW) {
        Ok(d) => d.characters,
        Err(_) => Vec::new(),
    }
}

/// 首次启动：把内置名册灌进配置
pub fn seed_characters(cfg: &mut Config) -> bool {
    let roster = bundled_roster();
    if roster.is_empty() {
        return false;
    }
    let mut changed = false;
    for r in roster {
        if r.id.is_empty() || r.name.is_empty() {
            continue;
        }
        if !cfg.characters.iter().any(|c| c.id == r.id) {
            cfg.characters.push(CharacterCfg {
                id: r.id,
                name: r.name,
                enabled: true,
            });
            changed = true;
        }
    }
    if changed {
        cfg.normalize();
    }
    changed
}

/// 首次启动时把内置示例音效写到数据目录，并挂到技能/大招上
pub fn install_demo_sounds(cfg: &mut Config) -> bool {
    const SKILL: &[u8] = include_bytes!("../assets/sounds/demo_skill.wav");
    const ULT: &[u8] = include_bytes!("../assets/sounds/demo_ultimate.wav");
    ensure_dirs();
    let dir = sounds_dir();
    let sp = dir.join("demo_skill.wav");
    let up = dir.join("demo_ultimate.wav");
    if !sp.exists() {
        let _ = std::fs::write(&sp, SKILL);
    }
    if !up.exists() {
        let _ = std::fs::write(&up, ULT);
    }
    let mut changed = false;
    for a in &mut cfg.actions {
        if a.sound.is_some() {
            continue;
        }
        match a.id.as_str() {
            "skill" => {
                a.sound = Some(sp.to_string_lossy().to_string());
                changed = true;
            }
            "ultimate" => {
                a.sound = Some(up.to_string_lossy().to_string());
                changed = true;
            }
            _ => {}
        }
    }
    changed
}

pub fn is_sound_file(p: &Path) -> bool {
    matches!(
        p.extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_ascii_lowercase())
            .as_deref(),
        Some("mp3") | Some("wav") | Some("m4a") | Some("flac") | Some("ogg") | Some("aac")
    )
}
