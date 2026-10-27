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

/// macOS：Quartz 全局坐标（逻辑点，左上原点）→ **物理像素**。
///
/// 为什么必须自己乘 DPI：菜单窗定位走 `set_position(PhysicalPosition)`，
/// Retina（sf=2）屏上直接传逻辑点会偏到四分之一处（实测 3360x2100 屏上
/// 菜单出现在 1/4 位置）。Tauri 的 `cursor_position()` 在 macOS 上返回的是
/// 未乘 scale 的 NSPoint，同样不能直接用。
///
/// 取光标所在显示器（而非主屏）的 scale：混合 DPI 多屏下按主屏算会错。
#[cfg(target_os = "macos")]
pub fn cursor_position() -> Option<(i32, i32)> {
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct CGPoint {
        x: f64,
        y: f64,
    }
    #[link(name = "CoreGraphics", kind = "framework")]
    extern "C" {
        fn CGEventCreate(source: *const core::ffi::c_void) -> *const core::ffi::c_void;
        fn CGEventGetLocation(event: *const core::ffi::c_void) -> CGPoint;
        fn CFRelease(cf: *const core::ffi::c_void);
        fn CGGetDisplaysWithPoint(
            point: CGPoint,
            max_displays: u32,
            displays: *mut u32,
            matching_count: *mut u32,
        ) -> i32;
        fn CGDisplayCopyDisplayMode(display: u32) -> *const core::ffi::c_void;
        fn CGDisplayModeGetWidth(mode: *const core::ffi::c_void) -> usize;
        fn CGDisplayModeGetPixelWidth(mode: *const core::ffi::c_void) -> usize;
    }

    // SAFETY: 下面只碰本地变量与 CoreGraphics 的查询类接口；CF 对象成对释放。
    unsafe {
        let event = CGEventCreate(core::ptr::null());
        if event.is_null() {
            return None;
        }
        let pt = CGEventGetLocation(event);
        CFRelease(event);

        // 找光标落在哪块屏（最多取 1 块就够）
        let mut display: u32 = 0;
        let mut count: u32 = 0;
        let rc = CGGetDisplaysWithPoint(pt, 1, &mut display, &mut count);
        let mut scale = 1.0f64;
        if rc == 0 && count >= 1 {
            let mode = CGDisplayCopyDisplayMode(display);
            if !mode.is_null() {
                let logical = CGDisplayModeGetWidth(mode);
                let physical = CGDisplayModeGetPixelWidth(mode);
                CFRelease(mode);
                if logical > 0 && physical > 0 {
                    scale = physical as f64 / logical as f64;
                }
            }
        }
        Some(((pt.x * scale).round() as i32, (pt.y * scale).round() as i32))
    }
}

#[cfg(all(not(windows), not(target_os = "macos")))]
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

/// 重置本应用的「屏幕录制」授权记录（macOS 官方 `tccutil`）。
///
/// 为什么需要：TCC 里那条记录可能"勾着但无效" —— 授权与代码签名绑定，
/// 而 adhoc 签名的 cdhash 每次重新构建都会变。此时用户无论怎么点开关都不生效，
/// 而且**系统不会再弹授权框**（因为有记录）。`tccutil reset ScreenCapture <id>`
/// 是唯一能把这个 app 的记录清掉、让系统重新走一遍授权流程的手段。
///
/// 只重置自己的 bundle id，不影响其它应用。重置后需要**重启本应用**
/// 才会重新触发系统授权框（系统限制）。
#[cfg(target_os = "macos")]
pub fn reset_screen_capture_permission(bundle_id: &str) -> Result<(), String> {
    let out = std::process::Command::new("tccutil")
        .args(["reset", "ScreenCapture", bundle_id])
        .output()
        .map_err(|e| format!("tccutil 启动失败：{e}"))?;
    if out.status.success() {
        Ok(())
    } else {
        Err(format!(
            "tccutil 退出码 {:?}：{}",
            out.status.code(),
            String::from_utf8_lossy(&out.stderr).trim()
        ))
    }
}

#[cfg(not(target_os = "macos"))]
pub fn reset_screen_capture_permission(_bundle_id: &str) -> Result<(), String> {
    Ok(())
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

/// 是否已获得「辅助功能」（Accessibility）授权。
///
/// 长截图的自动滚动靠 `CGEventPost` 注入滚轮事件，而 macOS 要求进程必须在
/// 「隐私与安全性 → 辅助功能」里被勾选，否则事件**被系统静默丢弃** ——
/// 不会报错，只会表现成"滚了但页面不动"。
///
/// 2026-10-27 真机日志即此：长截图首帧采到了 1850px，之后 0 步就报
/// "需要辅助功能权限"。所以这个权限和屏幕录制一样是**硬前置**。
#[cfg(target_os = "macos")]
pub fn has_accessibility_permission() -> bool {
    #[link(name = "ApplicationServices", kind = "framework")]
    extern "C" {
        fn AXIsProcessTrusted() -> bool;
    }
    // SAFETY: 无参纯查询接口。
    unsafe { AXIsProcessTrusted() }
}

#[cfg(not(target_os = "macos"))]
pub fn has_accessibility_permission() -> bool {
    true
}

/// 弹系统「辅助功能」授权框（macOS 会把本应用加入列表并高亮）。
///
/// 与屏幕录制不同，这个系统框**不会**让应用立即可用：用户必须自己去
/// 系统设置里勾选（macOS 的既定流程）。所以调用后仍要引导用户去设置页。
///
/// **2026-10-27：已从热路径摘掉，当前没有调用方。** 0.1.10 真机日志里，
/// 「点长截图 → 闪退」恰好死在紧接这条调用之后的 `accessibility_hint` 里，
/// 而这条调用是整个被拦路径上**唯一没被验证过**的原生调用（同一台机器上
/// `AXIsProcessTrusted()` 在 5 次启动里都正常返回、`tccutil`/`open` 子进程
/// 也正常），所以先摘掉它排出嫌疑；等拿到 .ips 崩溃报告再决定是否恢复。
///
/// 顺带修掉一个**静默失效**的老 bug：原实现给 `CFDictionaryCreate` 传了
/// NULL 的 key/value callbacks —— 那样字典按**指针身份**比较 key，
/// `AXIsProcessTrustedWithOptions` 内部用 `CFSTR("AXTrustedCheckOptionPrompt")`
/// 查表永远查不到，等于"带 prompt 的请求"退化成"纯查询"，系统框从来没弹过。
/// 现在改成真正的 `kCFTypeDictionaryKeyCallBacks` / `...ValueCallBacks`。
#[cfg(target_os = "macos")]
pub fn request_accessibility_permission() -> bool {
    use core::ffi::c_void;

    /// CFDictionaryKeyCallBacks（CFDictionary.h）：6 个字段，全是函数指针
    #[repr(C)]
    struct KeyCallBacks {
        version: isize,
        retain: *const c_void,
        release: *const c_void,
        copy_description: *const c_void,
        equal: *const c_void,
        hash: *const c_void,
    }
    /// CFDictionaryValueCallBacks：5 个字段（没有 hash）
    #[repr(C)]
    struct ValueCallBacks {
        version: isize,
        retain: *const c_void,
        release: *const c_void,
        copy_description: *const c_void,
        equal: *const c_void,
    }

    #[link(name = "ApplicationServices", kind = "framework")]
    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        fn AXIsProcessTrustedWithOptions(options: *const c_void) -> bool;
        fn CFDictionaryCreate(
            allocator: *const c_void,
            keys: *const *const c_void,
            values: *const *const c_void,
            num_values: isize,
            key_callbacks: *const c_void,
            value_callbacks: *const c_void,
        ) -> *const c_void;
        fn CFStringCreateWithCString(
            allocator: *const c_void,
            c_str: *const u8,
            encoding: u32,
        ) -> *const c_void;
        static kCFBooleanTrue: *const c_void;
        static kCFTypeDictionaryKeyCallBacks: KeyCallBacks;
        static kCFTypeDictionaryValueCallBacks: ValueCallBacks;
        fn CFRelease(cf: *const c_void);
    }
    const K_CF_STRING_ENCODING_UTF8: u32 = 0x08000100;
    // SAFETY: 只构造一个键值对字典并立即释放；指针都来自 CoreFoundation 分配，
    // 生命周期覆盖整个调用。
    unsafe {
        let key = CFStringCreateWithCString(
            core::ptr::null(),
            b"AXTrustedCheckOptionPrompt\0".as_ptr(),
            K_CF_STRING_ENCODING_UTF8,
        );
        if key.is_null() {
            return false;
        }
        let value = kCFBooleanTrue;
        let keys = [key];
        let values = [value];
        let dict = CFDictionaryCreate(
            core::ptr::null(),
            keys.as_ptr(),
            values.as_ptr(),
            1,
            &kCFTypeDictionaryKeyCallBacks as *const KeyCallBacks as *const c_void,
            &kCFTypeDictionaryValueCallBacks as *const ValueCallBacks as *const c_void,
        );
        let trusted = if dict.is_null() {
            false
        } else {
            AXIsProcessTrustedWithOptions(dict)
        };
        if !dict.is_null() {
            CFRelease(dict);
        }
        CFRelease(key);
        trusted
    }
}

#[cfg(not(target_os = "macos"))]
pub fn request_accessibility_permission() -> bool {
    true
}

/// 重置本应用的「辅助功能」授权记录（`tccutil reset Accessibility`）。
///
/// 与屏幕录制同源的坑：adhoc 签名每次构建 cdhash 都变，旧条目会变成
/// "勾着但无效"的僵尸记录，用户怎么点都没用。此时只能先删记录再重新授权。
#[cfg(target_os = "macos")]
pub fn reset_accessibility_permission(bundle_id: &str) -> Result<(), String> {
    let out = std::process::Command::new("tccutil")
        .args(["reset", "Accessibility", bundle_id])
        .output()
        .map_err(|e| format!("tccutil 启动失败：{e}"))?;
    if out.status.success() {
        Ok(())
    } else {
        Err(format!(
            "tccutil 退出码 {:?}：{}",
            out.status.code(),
            String::from_utf8_lossy(&out.stderr).trim()
        ))
    }
}

#[cfg(not(target_os = "macos"))]
pub fn reset_accessibility_permission(_bundle_id: &str) -> Result<(), String> {
    Ok(())
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

/// 清理临时目录里的历史残留文件，返回删除数量。
///
/// 截图裁剪图 / OCR 原图 / 贴图缩略图每次都会 `new_temp_png_path()` 生成一个
/// uuid 文件名，正常退出时没人删 → `%TEMP%/snappin` 会一路攒下去
/// （实测几天就有十几个）。启动时清一次，但**只删 1 小时前的**：
/// 多实例抢单实例锁之前、或另一进程正在用的文件不能被误删。
pub fn cleanup_old_temp_files() -> usize {
    let dir = temp_shot_dir();
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return 0;
    };
    let now = std::time::SystemTime::now();
    let mut removed = 0usize;
    for entry in entries.flatten() {
        let Ok(meta) = entry.metadata() else { continue };
        if !meta.is_file() {
            continue;
        }
        let Ok(modified) = meta.modified() else {
            continue;
        };
        let stale = now
            .duration_since(modified)
            .map(|age| age.as_secs() > 3600)
            .unwrap_or(false);
        if stale && std::fs::remove_file(entry.path()).is_ok() {
            removed += 1;
        }
    }
    removed
}

/// 生成一个新的临时 PNG 路径（不落盘）。
pub fn new_temp_png_path() -> PathBuf {    temp_shot_dir().join(format!("snap-{}.png", uuid::Uuid::new_v4()))
}
