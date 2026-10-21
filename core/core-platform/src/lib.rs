//! SnapPin 平台胶合层（core-platform）。
//!
//! 职责：把 core-algo 的纯算法接到各平台原生能力上
//! （屏幕采集 / 剪贴板 / 临时图像 IO / 二维码识别）。
//! 本 crate 不依赖 Tauri，可独立编译与测试。

pub mod qr;
pub mod ocr;
pub mod stitch;
pub mod translate;

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
    /// 显示器原点（虚拟桌面坐标；**单位随平台不同**，见 `logical_origin`）
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

impl MonitorShot {
    /// 显示器原点换算成**逻辑像素**（CSS px），供窗口定位使用。
    ///
    /// 为什么需要这个函数：xcap 各平台 `x()/y()` 单位不一致（已核对
    /// xcap 0.9.8 源码）——
    /// - Windows：来自 MONITORINFO，是**物理像素**，要除以 scale_factor；
    /// - macOS：来自 `CGDisplayBounds`，单位是**点**，本身就是逻辑坐标，
    ///   再除一次 scale_factor 会算出 1/2 的位置（Retina 副屏会严重偏移）。
    /// 统一成逻辑像素后，上层只按逻辑像素算窗口位置。
    pub fn logical_origin(&self) -> (f64, f64) {
        #[cfg(target_os = "macos")]
        {
            (self.x as f64, self.y as f64)
        }
        #[cfg(not(target_os = "macos"))]
        {
            let sf = self.scale_factor as f64;
            (self.x as f64 / sf, self.y as f64 / sf)
        }
    }
}

/// 光标在虚拟桌面中的物理像素坐标（多屏坐标系，与 `MonitorShot.x/y` 同系）。
///
/// 用途：剪贴板贴图（Ctrl+V）时把贴图窗口放到鼠标当前位置。
/// 失败或平台不支持时返回 None，调用方降级为主屏居中。
///
/// 实现：直接 FFI 声明 Win32 `GetCursorPos`，不为一个函数引入
/// windows crate 全量依赖（xcap 已传递依赖，但显式声明会拖慢编译）。
#[cfg(windows)]
pub fn cursor_position() -> Option<(i32, i32)> {
    #[repr(C)]
    struct POINT {
        x: i32,
        y: i32,
    }
    extern "system" {
        fn GetCursorPos(lpPoint: *mut POINT) -> i32;
    }
    let mut p = POINT { x: 0, y: 0 };
    // SAFETY: p 是本地可写结构体，GetCursorPos 仅写入 8 字节
    let ok = unsafe { GetCursorPos(&mut p) };
    if ok == 0 {
        return None;
    }
    Some((p.x, p.y))
}

#[cfg(not(windows))]
pub fn cursor_position() -> Option<(i32, i32)> {
    None
}

/// 确保已获得"屏幕录制"权限（仅 macOS 有意义）。
///
/// macOS 10.15 起，任何程序读取屏幕像素都要用户在
/// 「系统设置 → 隐私与安全性 → 屏幕录制」里授权；未授权时
/// CGWindowListCreateImage / ScreenCaptureKit 只返回空白或纯黑图，
/// 用户看到的现象就是"截图全黑 / 采集失败"。
///
/// 返回 true 表示已授权。未授权时会触发系统授权弹窗（首次），
/// 授权后**必须重启应用**才生效（系统限制，不是本程序的问题）。
#[cfg(target_os = "macos")]
pub fn has_screen_capture_permission() -> bool {
    #[link(name = "CoreGraphics", kind = "framework")]
    extern "C" {
        fn CGPreflightScreenCaptureAccess() -> bool;
    }
    // SAFETY: 无参纯查询接口，不涉及指针所有权。
    unsafe { CGPreflightScreenCaptureAccess() }
}

/// 弹出系统授权框（每个进程调一次即可）。
///
/// 只在**首次**需要时调用：`CGRequestScreenCaptureAccess` 会弹系统对话框，
/// 用户已明确拒绝过就不会再弹，此后再调只是静默返回 false。因此
/// 截图前的检查必须先走 `has_screen_capture_permission`（纯查询、无副作用），
/// 只有确实没权限时才请求一次。
#[cfg(target_os = "macos")]
pub fn request_screen_capture_permission() -> bool {
    #[link(name = "CoreGraphics", kind = "framework")]
    extern "C" {
        fn CGRequestScreenCaptureAccess() -> bool;
    }
    // SAFETY: 无参接口，只触发系统授权流程。
    unsafe { CGRequestScreenCaptureAccess() }
}

#[cfg(target_os = "macos")]
pub fn ensure_screen_capture_permission() -> bool {
    if has_screen_capture_permission() {
        true
    } else {
        request_screen_capture_permission()
    }
}

/// 非 macOS 平台无需屏幕录制授权，恒为 true。
#[cfg(not(target_os = "macos"))]
pub fn ensure_screen_capture_permission() -> bool {
    true
}

#[cfg(not(target_os = "macos"))]
pub fn has_screen_capture_permission() -> bool {
    true
}

#[cfg(not(target_os = "macos"))]
pub fn request_screen_capture_permission() -> bool {
    true
}

/// 截图临时目录：`%TEMP%/snappin`
pub fn temp_shot_dir() -> PathBuf {
    std::env::temp_dir().join("snappin")
}

/// 截取全部显示器：返回每块的像素缓冲与冻结用 PNG。
///
/// 多台显示器并行采集（每台独立线程做 DXGI/GDI 采集 + PNG 编码），
/// 避免串行等待；冻结图用 fast 压缩，牺牲体积换取速度。
pub fn capture_all_monitors() -> Result<Vec<MonitorShot>> {
    let dir = temp_shot_dir();
    std::fs::create_dir_all(&dir).context("create temp shot dir failed")?;

    let monitors = xcap::Monitor::all().context("enumerate monitors failed")?;
    let count = monitors.len();
    let mut slots: Vec<Option<MonitorShot>> = (0..count).map(|_| None).collect();

    let result: Result<()> = std::thread::scope(|s| {
        let mut handles = Vec::with_capacity(count);
        for (i, m) in monitors.into_iter().enumerate() {
            let dir = dir.clone();
            let m = SendMonitor(m); // 外层包装：闭包只捕获 Send 类型
            handles.push(s.spawn(move || m.capture_shot(i, &dir)));
        }
        for (i, h) in handles.into_iter().enumerate() {
            match h.join() {
                Ok(Ok(shot)) => slots[i] = Some(shot),
                Ok(Err(e)) => return Err(e),
                Err(_) => return Err(anyhow::anyhow!("capture thread {} panicked", i)),
            }
        }
        Ok(())
    });
    result?;

    Ok(slots.into_iter().flatten().collect())
}

/// 按 `capture_all_monitors` 的下标取显示器（会话 ID `"m{idx}"` 的逆运算）。
///
/// 长截图会话需要一枚可反复采集的 `xcap::Monitor`，而 `MonitorShot` 只存了
/// 像素结果，所以这里按同一套枚举顺序重新取。顺序一致性由
/// `capture_all_monitors` 与 `xcap::Monitor::all()` 的同一次枚举保证
/// （同一进程内显示器热插拔才会改变顺序，概率极低）。
pub fn monitor_by_index(idx: usize) -> Result<xcap::Monitor> {
    let all = xcap::Monitor::all().context("enumerate monitors failed")?;
    all.into_iter()
        .nth(idx)
        .ok_or_else(|| anyhow::anyhow!("monitor index {idx} not found"))
}

/// 跨线程传递 Monitor 的包装（并行采集与长截图会话共用）。
///
/// 安全性依据（xcap 0.9.8 Windows 实现）：
/// - 本项目未启用 xcap 的 `wgc` feature，`capture_image` 走 GDI BitBlt 路径，
///   每次调用新建 DC/位图，Monitor 本身无共享可变状态
/// - Windows 端 `ImplMonitor` 仅含 `h_monitor: HMONITOR` 不透明句柄，
///   HMONITOR 作为显示器标识符可跨线程使用（Win32 语义）
/// - macOS 端 `SCShareableInfo`/`CGDirectDisplayID` 同理为值语义句柄
///
/// ⚠️ 若未来启用 xcap `wgc` feature（WinRT 对象有 STA 线程亲和要求），
/// 必须重新评估此包装的线程安全性，或改回串行采集。
pub struct SendMonitor(xcap::Monitor);
unsafe impl Send for SendMonitor {}

impl SendMonitor {
    /// 借出内部 Monitor（采集 / 几何查询用）。
    pub fn get(&self) -> &xcap::Monitor {
        &self.0
    }

    /// 采集单台显示器并保存冻结图（在调用方线程内执行）。
    fn capture_shot(self, index: usize, dir: &Path) -> Result<MonitorShot> {
        let m = self.0;
        let rgba = m
            .capture_image()
            .with_context(|| format!("capture monitor {} failed", index))?;
        let width = rgba.width();
        let height = rgba.height();
        let buffer = RgbaBuffer::new(width, height, rgba.into_raw())
            .context("captured image has invalid buffer")?;

        let id = format!("m{}", index);
        let path = dir.join(format!("shot-{}.png", id));
        save_png_fast(&buffer, &path)?;

        Ok(MonitorShot {
            id,
            name: m
                .friendly_name()
                .unwrap_or_else(|_| format!("Display {}", index + 1)),
            x: m.x().unwrap_or(0),
            y: m.y().unwrap_or(0),
            width,
            height,
            scale_factor: m.scale_factor().unwrap_or(1.0),
            is_primary: m.is_primary().unwrap_or(false),
            image: buffer,
            path,
        })
    }
}

/// 生成默认保存文件名：`snappin_YYYYMMDDHHmmss_NNNN.png`。
///
/// 规则：snappin_ + 本地时间时间戳（秒级）+ 四位随机数字。
pub fn default_save_name() -> String {
    let ts = chrono::Local::now().format("%Y%m%d%H%M%S");
    // uuid 的 u128 取模取四位随机数字（避免额外引入 rand 依赖）
    let rand = (uuid::Uuid::new_v4().as_u128() % 10000) as u32;
    format!("snappin_{}_{:04}.png", ts, rand)
}

/// 把 RGBA 缓冲保存为 PNG（默认压缩，体积优先，用于用户主动保存）。
pub fn save_png(buffer: &RgbaBuffer, path: &Path) -> Result<()> {
    let image = image::RgbaImage::from_raw(buffer.width, buffer.height, buffer.data.clone())
        .ok_or_else(|| anyhow::anyhow!("invalid rgba buffer for {}", path.display()))?;
    image
        .save(path)
        .with_context(|| format!("save png failed: {}", path.display()))?;
    Ok(())
}

/// 把 RGBA 缓冲保存为 PNG（fast 压缩，速度优先，用于冻结图等瞬时用途）。
pub fn save_png_fast(buffer: &RgbaBuffer, path: &Path) -> Result<()> {
    use image::codecs::png::{CompressionType, FilterType, PngEncoder};
    let file = std::fs::File::create(path)
        .with_context(|| format!("create png failed: {}", path.display()))?;
    let encoder = PngEncoder::new_with_quality(
        std::io::BufWriter::new(file),
        CompressionType::Fast,
        FilterType::Sub,
    );
    image::ImageEncoder::write_image(
        encoder,
        &buffer.data,
        buffer.width,
        buffer.height,
        image::ExtendedColorType::Rgba8,
    )
    .with_context(|| format!("save png failed: {}", path.display()))?;
    Ok(())
}

/// 按目标扩展名保存图像：`.jpg/.jpeg` → JPEG（q90，去 alpha）；其他 → PNG。
pub fn save_image(buffer: &RgbaBuffer, path: &Path) -> Result<()> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase());
    match ext.as_deref() {
        Some("jpg") | Some("jpeg") => save_jpeg(buffer, path),
        _ => save_png(buffer, path),
    }
}

/// 把 RGBA 缓冲保存为 JPEG（质量 90；JPEG 无 alpha，先转 RGB）。
pub fn save_jpeg(buffer: &RgbaBuffer, path: &Path) -> Result<()> {
    use image::codecs::jpeg::JpegEncoder;
    let rgba = image::RgbaImage::from_raw(buffer.width, buffer.height, buffer.data.clone())
        .ok_or_else(|| anyhow::anyhow!("invalid rgba buffer for {}", path.display()))?;
    let mut rgb = image::RgbImage::new(buffer.width, buffer.height);
    for (dst, src) in rgb.pixels_mut().zip(rgba.pixels()) {
        *dst = image::Rgb([src[0], src[1], src[2]]);
    }
    let file = std::fs::File::create(path)
        .with_context(|| format!("create jpeg failed: {}", path.display()))?;
    let encoder = JpegEncoder::new_with_quality(std::io::BufWriter::new(file), 90);
    image::ImageEncoder::write_image(
        encoder,
        rgb.as_raw(),
        buffer.width,
        buffer.height,
        image::ExtendedColorType::Rgb8,
    )
    .with_context(|| format!("save jpeg failed: {}", path.display()))?;
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
