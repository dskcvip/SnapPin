//! SnapPin 平台胶合层（core-platform）。
//!
//! 职责：把 core-algo 的纯算法接到各平台原生能力上
//! （屏幕采集 / 剪贴板 / 临时图像 IO）。
//! 本 crate 不依赖 Tauri，可独立编译与测试。

use anyhow::{Context, Result};
use snappin_core_algo::RgbaBuffer;
use std::borrow::Cow;
use std::path::{Path, PathBuf};

/// 一块显示器的一次截图结果。
#[derive(Debug, Clone)]
pub struct MonitorShot {
    /// 会话内稳定 ID（如 "m0"）
    pub id: String,
    /// 显示器名称
    pub name: String,
    /// 显示器原点（虚拟桌面坐标）
    pub x: i32,
    pub y: i32,
    /// 物理像素尺寸
    pub width: u32,
    pub height: u32,
    /// DPI 缩放因子
    pub scale_factor: f32,
    pub is_primary: bool,
    /// 截图像素缓冲（RGBA8）
    pub image: RgbaBuffer,
    /// 冻结显示用的临时 PNG 路径
    pub path: PathBuf,
}

/// 截图临时目录：`%TEMP%/snappin`
pub fn temp_shot_dir() -> PathBuf {
    std::env::temp_dir().join("snappin")
}

/// 截取全部显示器：返回每块的像素缓冲与冻结用 PNG。
pub fn capture_all_monitors() -> Result<Vec<MonitorShot>> {
    let dir = temp_shot_dir();
    std::fs::create_dir_all(&dir).context("create temp shot dir failed")?;

    let monitors = xcap::Monitor::all().context("enumerate monitors failed")?;
    let mut shots = Vec::with_capacity(monitors.len());
    for (i, m) in monitors.iter().enumerate() {
        let rgba = m
            .capture_image()
            .with_context(|| format!("capture monitor {} failed", i))?;
        let width = rgba.width();
        let height = rgba.height();
        let buffer = RgbaBuffer::new(width, height, rgba.into_raw())
            .context("captured image has invalid buffer")?;

        let id = format!("m{}", i);
        let path = dir.join(format!("shot-{}.png", id));
        save_png(&buffer, &path)?;

        shots.push(MonitorShot {
            id,
            name: m
                .friendly_name()
                .unwrap_or_else(|_| format!("Display {}", i + 1)),
            x: m.x().unwrap_or(0),
            y: m.y().unwrap_or(0),
            width,
            height,
            scale_factor: m.scale_factor().unwrap_or(1.0),
            is_primary: m.is_primary().unwrap_or(false),
            image: buffer,
            path,
        });
    }
    Ok(shots)
}

/// 把 RGBA 缓冲保存为 PNG。
pub fn save_png(buffer: &RgbaBuffer, path: &Path) -> Result<()> {
    let image = image::RgbaImage::from_raw(buffer.width, buffer.height, buffer.data.clone())
        .ok_or_else(|| anyhow::anyhow!("invalid rgba buffer for {}", path.display()))?;
    image
        .save(path)
        .with_context(|| format!("save png failed: {}", path.display()))?;
    Ok(())
}

/// 从图片文件读回 RGBA 缓冲。
pub fn load_rgba(path: &Path) -> Result<RgbaBuffer> {
    let image = image::open(path)
        .with_context(|| format!("open image failed: {}", path.display()))?
        .to_rgba8();
    RgbaBuffer::new(image.width(), image.height(), image.into_raw())
        .context("decoded image has invalid buffer")
}

/// 复制图像到剪贴板。
pub fn clipboard_copy_image(buffer: &RgbaBuffer) -> Result<()> {
    let mut ctx = arboard::Clipboard::new().context("open clipboard failed")?;
    let image = arboard::ImageData {
        width: buffer.width as usize,
        height: buffer.height as usize,
        bytes: Cow::from(buffer.data.as_slice()),
    };
    ctx.set_image(image).context("set clipboard image failed")?;
    Ok(())
}

/// 复制文本到剪贴板。
pub fn clipboard_copy_text(text: &str) -> Result<()> {
    let mut ctx = arboard::Clipboard::new().context("open clipboard failed")?;
    ctx.set_text(text)
        .context("set clipboard text failed")?;
    Ok(())
}

/// 读取剪贴板图像；剪贴板无图像时返回 `None`。
pub fn clipboard_get_image() -> Result<Option<RgbaBuffer>> {
    let mut ctx = arboard::Clipboard::new().context("open clipboard failed")?;
    match ctx.get_image() {
        Ok(image) => {
            let buffer = RgbaBuffer::new(
                image.width as u32,
                image.height as u32,
                image.bytes.into_owned(),
            )
            .context("clipboard image has invalid buffer")?;
            Ok(Some(buffer))
        }
        Err(arboard::Error::ContentNotAvailable) => Ok(None),
        Err(e) => Err(e).context("get clipboard image failed"),
    }
}

/// 生成一个新的临时 PNG 路径（不落盘）。
pub fn new_temp_png_path() -> PathBuf {
    temp_shot_dir().join(format!("snap-{}.png", uuid::Uuid::new_v4()))
}
