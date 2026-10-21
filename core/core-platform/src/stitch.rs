//! 长截图采集控制（M5）。
//!
//! # 职责
//! 把 core-algo 的拼接算法接到平台能力上：
//! - **滚动输入**：Windows `SetCursorPos` + `mouse_event`；macOS `CGEvent`（需辅助
//!   功能权限）；其它平台返回 unsupported（前端降级为手动滚动模式）。
//! - **稳态采集**：滚完后等待画面静止（连续两帧差 < 阈值）再交给拼接器，
//!   避开平滑滚动动画的中间态。
//! - **光标治愈**：GDI BitBlt 会把鼠标光标截进帧里（实测确认），而光标停在
//!   选区中心会被拼进长图。对策：每次采集采两帧——光标在 P 采一帧、把光标
//!   移开到 P+nudge 再采一帧——用后者的像素补前者光标所在的矩形区域
//!   （页面静止时两帧内容一致）。这是 GDI 采集下的标准做法。
//! - **overlay 排除**：Windows 用 `SetWindowDisplayAffinity(WDA_EXCLUDEFROMCAPTURE)`
//!   把浮层排除出 BitBlt 采集（实测确认有效，见 scripts/test_wda_cursor.ps1）；
//!   macOS 无对应 API，改为每次 grab 前后隐藏/显示浮层窗口（由调用方通过
//!   `hide_ui`/`show_ui` 回调注入）。
//!
//! # 线程模型
//! `LongCapture` 由 src-tauri 的驱动循环在阻塞线程里逐步 `step()`，
//! 因此内部持有的 `xcap::Monitor` 必须可跨线程传递——xcap 的 Monitor 本身
//! 是 `!Send`（Windows 端 `ImplMonitor` 仅含 `HMONITOR` 值语义句柄），
//! 这里用 crate 内的 `SendMonitor` 包装（安全性论证见其文档）。
//! 会话整体（含 Monitor）因此可安全存进 Tauri 的 `Mutex<AppState>`。

use crate::SendMonitor;
use snappin_core_algo::stitch::{frame_mean_abs_diff, StitchOptions, Stitcher};
// 再导出：调用方匹配 StitchStepKind 时不必同时依赖 core-algo
pub use snappin_core_algo::stitch::{StitchStep, StitchStepKind};
use snappin_core_algo::{Rect, RgbaBuffer};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// 光标治愈：移开光标的偏移（像素）。取 -72 是为了让光标本体（≤44px）
/// 完全离开治愈框。
const CURSOR_NUDGE: (i32, i32) = (-72, -72);
/// 光标治愈框半边长（以光标热点为中心 44×44，覆盖箭头/手型/沙漏等标准光标）。
const CURSOR_HEAL_HALF: u32 = 22;
/// 稳态判定：连续两帧平均差 < 2.0（灰度域）认为画面已静止。
const STABLE_DIFF: f32 = 2.0;
/// 单步稳态等待上限（平滑滚动动画最长约 500ms，留一倍余量）。
const MAX_SETTLE_MS: u64 = 1200;
/// 稳态轮询间隔。
const SETTLE_POLL_MS: u64 = 100;

/// 采集前后的 UI 隐藏回调（macOS 用；Windows 传 None）。
pub type UiCallback = Box<dyn Fn() + Send + 'static>;

/// 一步 `step()` 的结果：正常拼接 / 被外部叫停。
#[derive(Debug, Clone, Copy)]
pub enum StepOutcome {
    /// 完成一步（结果见内层 StitchStep）
    Stepped(StitchStep),
    /// `stop` 标志在等待稳态期间被置位（用户按了 Esc），本步作废
    Stopped,
}

/// 一次长截图采集会话。
pub struct LongCapture {
    /// 目标显示器（会话期间不变，反复采集；SendMonitor 包装使会话可跨线程）
    monitor: SendMonitor,
    /// 物理像素选区（相对显示器原点）
    rect: Rect,
    /// 滚轮发送点 / 光标停留点（**绝对物理像素**，Windows 虚拟桌面坐标）
    scroll_point: (i32, i32),
    stitcher: Stitcher,
    /// 每步默认滚轮格数（一格 ≈ 120 delta ≈ 3 行）
    scroll_lines: i32,
    hide_ui: Option<UiCallback>,
    show_ui: Option<UiCallback>,
    /// 外部停止标志（用户按 Esc 时置位；None = 不可中断，测试用）
    stop: Option<Arc<AtomicBool>>,
}

impl LongCapture {
    /// 创建会话并采第一帧（选区内容作为拼接基底）。
    ///
    /// `rect` 为物理像素、相对显示器原点；`monitor` 由调用方按
    /// `monitor_id`（"m0" 形式）从 `xcap::Monitor::all()` 取出。
    pub fn new(
        monitor: xcap::Monitor,
        rect: Rect,
        opts: StitchOptions,
        scroll_lines: i32,
        hide_ui: Option<UiCallback>,
        show_ui: Option<UiCallback>,
        stop: Option<Arc<AtomicBool>>,
    ) -> Result<Self, String> {
        if rect.is_empty() {
            return Err("long capture rect is empty".to_string());
        }
        let monitor = SendMonitor(monitor);
        // 滚轮点 = 显示器原点 + 选区中心（物理像素；macOS 的 x()/y() 是点，
        // 与 scroll_wheel 的 CGEvent 坐标系一致，无需换算）
        let mx = monitor.get().x().map_err(|e| e.to_string())?;
        let my = monitor.get().y().map_err(|e| e.to_string())?;
        let scroll_point = (
            (mx as f64 + rect.x as f64 + rect.w as f64 / 2.0).round() as i32,
            (my as f64 + rect.y as f64 + rect.h as f64 / 2.0).round() as i32,
        );

        // 第一帧：当前选区内容（此时用户刚框选完，页面静止）
        let first = grab_cursorless(monitor.get(), &rect, scroll_point, &hide_ui, &show_ui)?;
        if first.width != rect.w || first.height != rect.h {
            return Err(format!(
                "selection {}x{} outside monitor bounds (got {}x{})",
                rect.w, rect.h, first.width, first.height
            ));
        }
        let stitcher = Stitcher::new(&first, opts).map_err(|e| e.to_string())?;
        Ok(Self {
            monitor,
            rect,
            scroll_point,
            stitcher,
            scroll_lines: scroll_lines.max(1),
            hide_ui,
            show_ui,
            stop,
        })
    }

    /// 当前拼接高度（物理像素）。
    pub fn height(&self) -> u32 {
        self.stitcher.height()
    }

    /// 累计步数。
    pub fn steps(&self) -> u32 {
        self.stitcher.steps()
    }

    /// 连续未动轮次（到底判定由调用方消费）。
    pub fn still_count(&self) -> u32 {
        self.stitcher.still_count()
    }

    /// 是否已判定到底（连续 still_rounds 轮画面没动）。
    pub fn at_bottom(&self) -> bool {
        self.stitcher.at_bottom()
    }

    /// 累积图引用（缩略图用）。
    pub fn canvas(&self) -> &RgbaBuffer {
        self.stitcher.canvas()
    }

    /// 结束会话，取出最终长图。
    pub fn into_canvas(self) -> RgbaBuffer {
        self.stitcher.into_canvas()
    }

    /// 推进一步：滚动 → 等稳态 → 采集 → 拼接。
    ///
    /// `lines` 覆盖默认格数（NoMatch 重试时调用方传 1 轻滚）。
    /// 等待稳态期间若 `stop` 被置位，返回 `StepOutcome::Stopped`（本步作废，
    /// 拼接器保持原状，调用方应结束会话并取 `into_canvas()`）。
    pub fn step(&mut self, lines: i32) -> Result<StepOutcome, String> {
        // 光标必须先位于滚动点，滚轮才落得到目标窗口
        set_cursor_pos(self.scroll_point.0, self.scroll_point.1);
        scroll_wheel(lines)?;
        let frame = match self.grab_stable()? {
            Some(f) => f,
            None => return Ok(StepOutcome::Stopped),
        };
        let step = self.stitcher.push(&frame).map_err(|e| e.to_string())?;
        Ok(StepOutcome::Stepped(step))
    }

    /// 用默认格数推进一步。
    pub fn step_default(&mut self) -> Result<StepOutcome, String> {
        let lines = self.scroll_lines;
        self.step(lines)
    }

    // ---------------------------------------------------------------- 采集

    /// 滚动后等待画面静止，返回稳定的帧；`stop` 被置位则返回 `None`。
    fn grab_stable(&self) -> Result<Option<RgbaBuffer>, String> {
        let mut frame = self.grab_cursorless()?;
        let deadline = Instant::now() + Duration::from_millis(MAX_SETTLE_MS);
        loop {
            if self.is_stopped() {
                return Ok(None);
            }
            std::thread::sleep(Duration::from_millis(SETTLE_POLL_MS));
            let next = self.grab_cursorless()?;
            let diff = frame_mean_abs_diff(&frame, &next).unwrap_or(255.0);
            frame = next;
            if diff < STABLE_DIFF || Instant::now() >= deadline {
                return Ok(Some(frame));
            }
        }
    }

    fn is_stopped(&self) -> bool {
        self.stop
            .as_ref()
            .map(|s| s.load(Ordering::SeqCst))
            .unwrap_or(false)
    }

    fn grab_cursorless(&self) -> Result<RgbaBuffer, String> {
        grab_cursorless(
            self.monitor.get(),
            &self.rect,
            self.scroll_point,
            &self.hide_ui,
            &self.show_ui,
        )
    }
}

/// 采一帧选区（物理像素）；`hide_ui`/`show_ui` 在采集前后调用
/// （macOS 隐藏浮层用，Windows 为 None）。
fn grab_cursorless(
    monitor: &xcap::Monitor,
    rect: &Rect,
    scroll_point: (i32, i32),
    hide_ui: &Option<UiCallback>,
    show_ui: &Option<UiCallback>,
) -> Result<RgbaBuffer, String> {
    let mut a = grab_once(monitor, rect, hide_ui, show_ui)?;
    let (px, py) = scroll_point;
    // 帧内坐标：光标热点相对选区左上角
    let cx = px - rect.x;
    let cy = py - rect.y;
    // 移开光标采第二帧（页面静止时与 a 内容一致）
    set_cursor_pos(px + CURSOR_NUDGE.0, py + CURSOR_NUDGE.1);
    let b = grab_once(monitor, rect, hide_ui, show_ui)?;
    // 光标移回滚轮点，供下一步滚动使用
    set_cursor_pos(px, py);
    heal_cursor_at(&mut a, &b, cx, cy, CURSOR_HEAL_HALF);
    Ok(a)
}

/// 单次采集（含 hide/show 包裹）。
fn grab_once(
    monitor: &xcap::Monitor,
    rect: &Rect,
    hide_ui: &Option<UiCallback>,
    show_ui: &Option<UiCallback>,
) -> Result<RgbaBuffer, String> {
    if let Some(f) = hide_ui {
        f();
    }
    let result = (|| {
        let img = monitor
            .capture_image()
            .map_err(|e| format!("capture failed: {e}"))?;
        let full =
            RgbaBuffer::new(img.width(), img.height(), img.into_raw()).map_err(|e| e.to_string())?;
        full.crop(rect).map_err(|e| e.to_string())
    })();
    if let Some(f) = show_ui {
        f();
    }
    result
}

/// 把 `b` 的像素复制到 `a` 的 (cx, cy) 附近矩形（光标治愈）。
/// 两帧尺寸必须一致；矩形越界部分跳过。
pub fn heal_cursor_at(a: &mut RgbaBuffer, b: &RgbaBuffer, cx: i32, cy: i32, half: u32) {
    if (a.width, a.height) != (b.width, b.height) {
        return;
    }
    let half = half as i32;
    let x0 = (cx - half).max(0) as u32;
    let y0 = (cy - half).max(0) as u32;
    let x1 = ((cx + half + 1) as u32).min(a.width);
    let y1 = ((cy + half + 1) as u32).min(a.height);
    let w = a.width as usize;
    for y in y0..y1 {
        let row_a = &mut a.data[y as usize * w * 4..(y as usize + 1) * w * 4];
        let row_b = &b.data[y as usize * w * 4..(y as usize + 1) * w * 4];
        for x in x0..x1 {
            let i = x as usize * 4;
            row_a[i..i + 4].copy_from_slice(&row_b[i..i + 4]);
        }
    }
}

/// 保存拼接图的缩略图（宽 ≤ 240px，fast PNG）。
pub fn write_thumbnail(canvas: &RgbaBuffer, path: &Path) -> Result<(), String> {
    use image::imageops::FilterType;
    let img = image::RgbaImage::from_raw(canvas.width, canvas.height, canvas.data.clone())
        .ok_or_else(|| "invalid canvas buffer".to_string())?;
    let tw = 240.min(canvas.width);
    let th = ((canvas.height as f64 * tw as f64 / canvas.width as f64).round() as u32).max(1);
    let small = image::imageops::resize(&img, tw, th, FilterType::Triangle);
    let buffer = RgbaBuffer::new(tw, th, small.into_raw()).map_err(|e| e.to_string())?;
    crate::save_png_fast(&buffer, path).map_err(|e| e.to_string())
}

// ---------------------------------------------------------------- 平台输入

/// 移动光标（绝对坐标；系统会钳制到虚拟桌面内，见实测）。
pub fn set_cursor_pos(x: i32, y: i32) {
    #[cfg(windows)]
    {
        extern "system" {
            fn SetCursorPos(x: i32, y: i32) -> i32;
        }
        // SAFETY: 纯值参数，不涉及指针所有权
        unsafe {
            SetCursorPos(x, y);
        }
    }
    #[cfg(target_os = "macos")]
    {
        #[repr(C)]
        struct CGPoint {
            x: f64,
            y: f64,
        }
        extern "C" {
            fn CGWarpMouseCursorPosition(point: CGPoint) -> i32;
        }
        // SAFETY: 纯值参数。CG 坐标为"点"（逻辑像素），左上原点。
        unsafe {
            CGWarpMouseCursorPosition(CGPoint {
                x: x as f64,
                y: y as f64,
            });
        }
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        let _ = (x, y);
    }
}

/// 发送滚轮（向下滚动 `lines` 格）。光标此时应已位于滚动点。
pub fn scroll_wheel(lines: i32) -> Result<(), String> {
    let lines = lines.max(1);
    #[cfg(windows)]
    {
        const MOUSEEVENTF_WHEEL: u32 = 0x0800;
        const WHEEL_DELTA: i32 = 120;
        extern "system" {
            fn mouse_event(dw_flags: u32, dx: i32, dy: i32, dw_data: u32, dw_extra_info: usize);
        }
        // 负 delta = 向后（向下）滚
        // SAFETY: 纯值参数（mouse_event 无指针语义）
        unsafe {
            mouse_event(MOUSEEVENTF_WHEEL, 0, 0, (-(WHEEL_DELTA * lines)) as u32, 0);
        }
        Ok(())
    }
    #[cfg(target_os = "macos")]
    {
        const K_CG_SCROLL_EVENT_UNIT_LINE: u32 = 1;
        const K_CG_HID_EVENT_TAP: u32 = 0; // CGEventTapLocation 枚举首项
        extern "C" {
            fn AXIsProcessTrusted() -> bool;
            fn CGEventCreateScrollWheelEvent(
                source: *const std::ffi::c_void,
                units: u32,
                wheel_count: u32,
                wheel1: i32,
            ) -> *const std::ffi::c_void;
            fn CGEventPost(tap: u32, event: *const std::ffi::c_void);
            fn CFRelease(cf: *const std::ffi::c_void);
        }
        // 未授权辅助功能时 CGEventPost 会被系统静默丢弃——必须显式报错，
        // 否则表现为"滚了但页面不动"的灵异问题。
        if !unsafe { AXIsProcessTrusted() } {
            return Err(
                "macOS 需要「辅助功能」权限才能自动滚动：系统设置 → 隐私与安全性 → 辅助功能"
                    .to_string(),
            );
        }
        unsafe {
            let event = CGEventCreateScrollWheelEvent(
                std::ptr::null(),
                K_CG_SCROLL_EVENT_UNIT_LINE,
                1,
                -lines,
            );
            if event.is_null() {
                return Err("CGEventCreateScrollWheelEvent failed".to_string());
            }
            CGEventPost(K_CG_HID_EVENT_TAP, event);
            CFRelease(event);
        }
        Ok(())
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        let _ = lines;
        Err("当前平台暂不支持自动滚动，请使用手动滚动模式".to_string())
    }
}

// ---------------------------------------------------------------- 采集排除（Windows）

/// 把窗口（含全部子窗口，覆盖 WebView2 的 Chrome_WidgetWin）标记为
/// 「排除出屏幕采集」——窗口照常显示，但不出现在 BitBlt 采集结果里。
///
/// 实测确认对 GDI BitBlt 有效（scripts/test_wda_cursor.ps1）：
/// 设了 WDA 的窗口在采集帧里透出下层内容。
/// 仅 Windows 10 2004+（build 19041+）有效；旧系统返回 false，调用方
/// 应降级（见 src-tauri 长截图命令的注释）。
///
/// 参数 `hwnd` 为 Win32 窗口句柄（isize 承载）；本 crate 不依赖 Tauri，
/// 由调用方（src-tauri）用 raw_window_handle 从窗口对象取出。
#[cfg(windows)]
pub fn set_hwnd_capture_excluded(hwnd: isize, excluded: bool) -> bool {
    if hwnd == 0 {
        return false;
    }
    const WDA_NONE: u32 = 0x0;
    const WDA_EXCLUDEFROMCAPTURE: u32 = 0x11;
    unsafe {
        set_hwnd_excluded_recursive(
            hwnd,
            if excluded {
                WDA_EXCLUDEFROMCAPTURE
            } else {
                WDA_NONE
            },
        )
    }
}

#[cfg(windows)]
unsafe fn set_hwnd_excluded_recursive(hwnd: isize, affinity: u32) -> bool {
    use std::ffi::c_void;
    extern "system" {
        fn SetWindowDisplayAffinity(hwnd: *mut c_void, affinity: u32) -> i32;
        fn EnumChildWindows(
            hwnd_parent: *mut c_void,
            enum_proc: Option<unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32>,
            param: *mut c_void,
        ) -> i32;
    }
    if SetWindowDisplayAffinity(hwnd as *mut c_void, affinity) == 0 {
        return false;
    }
    // 子窗口（WebView2 渲染宿主）也要设：WDA 按 HWND 生效，不自动继承
    struct Ctx {
        affinity: u32,
        all_ok: bool,
    }
    unsafe extern "system" fn enum_proc(child: *mut c_void, param: *mut c_void) -> i32 {
        let ctx = &mut *(param as *mut Ctx);
        if SetWindowDisplayAffinity(child, ctx.affinity) == 0 {
            ctx.all_ok = false;
        }
        1 // 继续枚举
    }
    let mut ctx = Ctx {
        affinity,
        all_ok: true,
    };
    EnumChildWindows(
        hwnd as *mut c_void,
        Some(enum_proc),
        &mut ctx as *mut Ctx as *mut c_void,
    );
    ctx.all_ok
}

/// 非 Windows 平台无 WDA 等价物，恒 false（调用方走 macOS 隐藏回调）。
#[cfg(not(windows))]
pub fn set_hwnd_capture_excluded(_hwnd: isize, _excluded: bool) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 造一张确定性纹理图（行内容互不相同）。
    fn tex(w: u32, h: u32) -> RgbaBuffer {
        let mut data = Vec::with_capacity(w as usize * h as usize * 4);
        for y in 0..h {
            for x in 0..w {
                let n = (y.wrapping_mul(2654435761) ^ x.wrapping_mul(40503)) >> 7;
                data.extend_from_slice(&[
                    (n & 0xff) as u8,
                    ((n >> 8) & 0xff) as u8,
                    128,
                    255,
                ]);
            }
        }
        RgbaBuffer::new(w, h, data).unwrap()
    }

    #[test]
    fn heal_removes_cursor_sprite_exactly() {        let clean = tex(200, 120);
        let mut with_cursor = clean.clone();
        // 在 (100, 60) 画一个 16x16 的"光标"（纯品红，与纹理完全不同）
        for dy in 0..16u32 {
            for dx in 0..16u32 {
                let i = ((60 + dy) as usize * 200 + 100 + dx as usize) * 4;
                with_cursor.data[i..i + 4].copy_from_slice(&[255, 0, 255, 255]);
            }
        }
        heal_cursor_at(&mut with_cursor, &clean, 100, 60, 22);
        assert_eq!(with_cursor, clean, "healed frame must equal clean frame");
    }

    #[test]
    fn heal_clips_at_edges() {
        let clean = tex(50, 50);
        let mut a = clean.clone();
        // 光标在 (0,0)：治愈框越界，只补不越界的部分
        a.data[0..4].copy_from_slice(&[9, 9, 9, 255]);
        heal_cursor_at(&mut a, &clean, 0, 0, 22);
        assert_eq!(a, clean);
    }

    #[test]
    fn heal_ignores_size_mismatch() {
        let mut a = tex(10, 10);
        let b = tex(20, 20);
        let before = a.clone();
        heal_cursor_at(&mut a, &b, 5, 5, 3);
        assert_eq!(a, before, "size mismatch must be a no-op");
    }

    #[test]
    fn thumbnail_dimensions_and_pixels() {
        // 平滑梯度（降采样后可预测）；噪声纹理不行——3.3 倍平均后单像素无意义
        let w = 800u32;
        let h = 3000u32;
        let mut data = Vec::with_capacity(w as usize * h as usize * 4);
        for y in 0..h {
            for x in 0..w {
                let v = (x * 255 / w) as u8;
                data.extend_from_slice(&[v, 255 - v, y as u8, 255]);
            }
        }
        let canvas = RgbaBuffer::new(w, h, data).unwrap();
        let dir = std::env::temp_dir().join("snappin-stitch-test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("thumb.png");
        write_thumbnail(&canvas, &path).unwrap();
        let back = crate::load_rgba(&path).unwrap();
        assert_eq!(back.width, 240);
        assert_eq!(back.height, 900); // 3000 * 240/800
        // 缩略图 (120,0) 应约等于原图 (400,0)（同一梯度位置）
        let a = canvas.pixel(400, 0).unwrap();
        let b = back.pixel(120, 0).unwrap();
        for c in 0..3 {
            assert!((a[c] as i32 - b[c] as i32).abs() <= 8, "channel {c} drifted");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 编译期保障：会话要存进 Tauri 的 `Mutex<AppState>`，必须 Send。
    /// xcap::Monitor 本身 !Send（Windows ImplMonitor 含 HMONITOR），
    /// 靠 SendMonitor 包装——哪天门卫松了，这个测试立刻红。
    #[test]
    fn long_capture_is_send() {
        fn assert_send<T: Send>() {}
        assert_send::<LongCapture>();
        assert_send::<StepOutcome>();
    }
}
