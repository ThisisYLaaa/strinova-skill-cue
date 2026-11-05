//! 音频引擎：symphonia 解码 + cpal 输出。
//!
//! 关键点：
//! * 触发时**不解码**——音频在配置时就解码成 f32 常驻内存，触发只做一次 push。
//! * 输出流（Stream）常开，不会在触发时去开设备。
//! * 因此「按键 -> 出声」只有一次混音回调的延迟（默认争取 10 ms 缓冲）。

use std::collections::HashMap;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SampleFormat, SizedSample, StreamConfig};

use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::{DecoderOptions, CODEC_TYPE_NULL};
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

struct Voice {
    data: Arc<Vec<f32>>,
    ch: usize,
    pos: usize,
    gain: f32,
}

#[derive(Default)]
struct Mixer {
    voices: Vec<Voice>,
    volume: f32,
}

#[derive(Clone, Debug)]
pub struct CachedSound {
    pub data: Arc<Vec<f32>>,
    pub ch: u16,
    pub frames: usize,
    pub src_rate: u32,
}

pub struct AudioEngine {
    shared: Arc<Mutex<Mixer>>,
    _stream: cpal::Stream,
    failed: Arc<AtomicBool>,
    cache: HashMap<PathBuf, CachedSound>,
    pub device_name: String,
    pub sample_rate: u32,
    pub channels: u16,
}

impl AudioEngine {
    pub fn new(volume: f32) -> Result<Self, String> {
        let host = cpal::default_host();
        let device = host
            .default_output_device()
            .ok_or_else(|| "找不到默认音频输出设备".to_string())?;
        let device_name = device.name().unwrap_or_else(|_| "未知设备".into());
        let supported = device
            .default_output_config()
            .map_err(|e| format!("读取输出设备配置失败: {e}"))?;
        let sample_format = supported.sample_format();
        let mut config: StreamConfig = supported.config();
        let rate = config.sample_rate.0;
        let channels = config.channels;

        let shared = Arc::new(Mutex::new(Mixer {
            voices: Vec::new(),
            volume: volume.clamp(0.0, 1.5),
        }));
        let failed = Arc::new(AtomicBool::new(false));

        // 先试 10 ms 小缓冲（更低延迟），失败再退回设备默认
        let mut stream = None;
        if let cpal::SupportedBufferSize::Range { min, .. } = supported.buffer_size() {
            let want = (rate / 100).max(*min).max(64);
            config.buffer_size = cpal::BufferSize::Fixed(want);
            stream = build(&device, &config, sample_format, shared.clone(), failed.clone()).ok();
        }
        let stream = match stream {
            Some(s) => s,
            None => {
                config.buffer_size = cpal::BufferSize::Default;
                build(&device, &config, sample_format, shared.clone(), failed.clone())
                    .map_err(|e| format!("创建音频输出流失败: {e}"))?
            }
        };
        stream
            .play()
            .map_err(|e| format!("启动音频流失败: {e}"))?;

        Ok(Self {
            shared,
            _stream: stream,
            failed,
            cache: HashMap::new(),
            device_name,
            sample_rate: rate,
            channels,
        })
    }

    pub fn healthy(&self) -> bool {
        !self.failed.load(Ordering::Relaxed)
    }

    pub fn set_volume(&self, v: f32) {
        if let Ok(mut m) = self.shared.lock() {
            m.volume = v.clamp(0.0, 1.5);
        }
    }

    pub fn stop_all(&self) {
        if let Ok(mut m) = self.shared.lock() {
            m.voices.clear();
        }
    }

    /// 预解码并缓存
    pub fn preload(&mut self, paths: &[PathBuf]) -> Vec<(PathBuf, String)> {
        let mut errs = Vec::new();
        for p in paths {
            if self.cache.contains_key(p) || !p.exists() {
                continue;
            }
            match decode(p, self.sample_rate) {
                Ok(s) => {
                    self.cache.insert(p.clone(), s);
                }
                Err(e) => errs.push((p.clone(), e)),
            }
        }
        errs
    }

    pub fn ensure_loaded(&mut self, path: &Path) -> Result<(), String> {
        if self.cache.contains_key(path) {
            return Ok(());
        }
        let s = decode(path, self.sample_rate)?;
        self.cache.insert(path.to_path_buf(), s);
        Ok(())
    }

    pub fn is_loaded(&self, path: &Path) -> bool {
        self.cache.contains_key(path)
    }

    /// 立刻从头播放（若同名音效正在播放，先把旧的掐掉，避免叠加成噪音）
    pub fn play(&mut self, path: &Path, gain: f32) -> Result<(), String> {
        self.ensure_loaded(path)?;
        let snd = self.cache.get(path).ok_or("音频未加载")?.clone();
        let mut m = self.shared.lock().map_err(|_| "音频锁中毒")?;
        m.voices.retain(|v| v.pos < v.data.len());
        // 同一份音效重新触发：直接重置进度，保证「从 0:00 开始」
        for v in m.voices.iter_mut() {
            if Arc::ptr_eq(&v.data, &snd.data) {
                v.pos = 0;
                v.gain = gain * snd_gain(&snd);
                return Ok(());
            }
        }
        m.voices.push(Voice {
            data: snd.data.clone(),
            ch: snd.ch.max(1) as usize,
            pos: 0,
            gain: gain * snd_gain(&snd),
        });
        Ok(())
    }

    pub fn active_voices(&self) -> usize {
        self.shared
            .lock()
            .map(|m| m.voices.iter().filter(|v| v.pos < v.data.len()).count())
            .unwrap_or(0)
    }
}

fn snd_gain(s: &CachedSound) -> f32 {
    // 短促提示音整体略微压低，避免刺耳
    let _ = s.frames;
    1.0
}

fn build(
    device: &cpal::Device,
    config: &StreamConfig,
    format: SampleFormat,
    shared: Arc<Mutex<Mixer>>,
    failed: Arc<AtomicBool>,
) -> Result<cpal::Stream, String> {
    let err_fn = move |e| {
        eprintln!("音频流错误: {e}");
        failed.store(true, Ordering::Relaxed);
    };
    macro_rules! mk {
        ($t:ty) => {
            build_typed::<$t>(device, config, shared, err_fn)
        };
    }
    match format {
        SampleFormat::F32 => mk!(f32),
        SampleFormat::I16 => mk!(i16),
        SampleFormat::U16 => mk!(u16),
        SampleFormat::I32 => mk!(i32),
        SampleFormat::I8 => mk!(i8),
        SampleFormat::U8 => mk!(u8),
        other => Err(format!("不支持的采样格式: {other:?}")),
    }
}

fn build_typed<T>(
    device: &cpal::Device,
    config: &StreamConfig,
    shared: Arc<Mutex<Mixer>>,
    err_fn: impl FnMut(cpal::StreamError) + Send + 'static,
) -> Result<cpal::Stream, String>
where
    T: SizedSample + FromSample<f32> + 'static,
{
    let nch = config.channels as usize;
    let stream = device
        .build_output_stream(
            config,
            move |out: &mut [T], _: &cpal::OutputCallbackInfo| {
                let mut guard = match shared.try_lock() {
                    Ok(g) => g,
                    Err(_) => {
                        for s in out.iter_mut() {
                            *s = T::from_sample(0.0f32);
                        }
                        return;
                    }
                };
                let master = guard.volume;
                for frame in out.chunks_mut(nch) {
                    let mut l = 0.0f32;
                    let mut r = 0.0f32;
                    for v in guard.voices.iter_mut() {
                        if v.pos >= v.data.len() {
                            continue;
                        }
                        let c = v.ch;
                        let base = v.pos;
                        let (ls, rs) = if c == 1 {
                            let s = v.data[base] * v.gain;
                            (s, s)
                        } else {
                            let a = v.data[base] * v.gain;
                            let b = v.data[base + (c - 1).min(1)] * v.gain;
                            (a, b)
                        };
                        l += ls;
                        r += rs;
                        v.pos += c;
                    }
                    l = (l * master).clamp(-1.0, 1.0);
                    r = (r * master).clamp(-1.0, 1.0);
                    if nch == 1 {
                        frame[0] = T::from_sample((l + r) * 0.5);
                    } else {
                        frame[0] = T::from_sample(l);
                        frame[1] = T::from_sample(r);
                        for s in frame.iter_mut().skip(2) {
                            *s = T::from_sample(0.0f32);
                        }
                    }
                }
                guard.voices.retain(|v| v.pos < v.data.len());
            },
            err_fn,
            None,
        )
        .map_err(|e| format!("创建输出流失败: {e}"))?;
    Ok(stream)
}

/// 解码任意支持格式 -> 目标采样率的 f32 交错数据
pub fn decode(path: &Path, target_rate: u32) -> Result<CachedSound, String> {
    let file = File::open(path).map_err(|e| format!("打开失败: {e}"))?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }
    let probed = symphonia::default::get_probe()
        .format(
            &hint,
            mss,
            &FormatOptions {
                enable_gapless: true,
                ..Default::default()
            },
            &MetadataOptions::default(),
        )
        .map_err(|e| format!("无法识别的音频容器: {e}"))?;
    let mut format = probed.format;
    let track = format
        .default_track()
        .ok_or_else(|| "音频文件里没有可用音轨".to_string())?;
    let track_id = track.id;
    let params = track.codec_params.clone();
    if params.codec == CODEC_TYPE_NULL {
        return Err("未知编码".into());
    }
    let mut decoder = symphonia::default::get_codecs()
        .make(&params, &DecoderOptions::default())
        .map_err(|e| format!("不支持的编码: {e}"))?;

    let mut samples: Vec<f32> = Vec::new();
    let mut src_ch = params.channels.map(|c| c.count()).unwrap_or(2).max(1);
    let mut src_rate = params.sample_rate.unwrap_or(target_rate).max(1);

    loop {
        let packet = match format.next_packet() {
            Ok(p) => p,
            Err(_) => break,
        };
        if packet.track_id() != track_id {
            continue;
        }
        match decoder.decode(&packet) {
            Ok(buf) => {
                let spec = *buf.spec();
                src_ch = spec.channels.count().max(1);
                src_rate = spec.rate.max(1);
                let frames = buf.frames() as u64;
                if frames == 0 {
                    continue;
                }
                let mut sb = SampleBuffer::<f32>::new(frames, spec);
                sb.copy_interleaved_ref(buf);
                let s = sb.samples();
                samples.extend_from_slice(s);
            }
            Err(symphonia::core::errors::Error::DecodeError(_)) => continue,
            Err(symphonia::core::errors::Error::IoError(ref e))
                if e.kind() == std::io::ErrorKind::UnexpectedEof =>
            {
                break
            }
            Err(_) => break,
        }
    }
    if samples.is_empty() {
        return Err("解码后没有采样数据".into());
    }
    let frames = samples.len() / src_ch;
    if frames == 0 {
        return Err("解码后没有采样数据".into());
    }

    // 线性重采样到设备采样率（避免播放时再算）
    let ratio = target_rate as f64 / src_rate as f64;
    let out_frames = ((frames as f64) * ratio).round().max(1.0) as usize;
    let mut data = Vec::with_capacity(out_frames * src_ch);
    let last = frames - 1;
    for i in 0..out_frames {
        let pos = i as f64 / ratio;
        let i0 = (pos.floor() as usize).min(last);
        let i1 = (i0 + 1).min(last);
        let frac = (pos - i0 as f64) as f32;
        for c in 0..src_ch {
            let a = samples[i0 * src_ch + c];
            let b = samples[i1 * src_ch + c];
            data.push(a + (b - a) * frac);
        }
    }

    Ok(CachedSound {
        data: Arc::new(data),
        ch: src_ch as u16,
        frames: out_frames,
        src_rate,
    })
}

/// 时长（毫秒），用于 UI 展示
pub fn duration_ms(path: &Path) -> Option<u64> {
    let s = decode(path, 48_000).ok()?;
    Some((s.frames as u64) * 1000 / 48_000)
}
