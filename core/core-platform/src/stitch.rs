//! 长截图采集控制（M5）。
//!
//! # 职责
//! 把 core-algo 的拼接算法接到平台能力上：
//! - **滚动输入**：Windows `SetCursorPos` + `mouse_event`；macOS `CGEvent`（需辅助
//!   功能权限）；其它平台返回 unsupported（前端降级为手动滚动模式）。
//! - **稳态采集**：滚完后等待画面静止（连续两帧差 < 阈值）再交给拼接器，
//!   避开平滑滚动动画的中间态。
//! - **光标处理（v2，丝滑化改造）**：旧实现每采一帧要把光标瞬移到 A、采一帧、
//!   再瞬移到 B 采一帧、用 B 补 A 的光标区域——**光标在两个点之间每 100ms 跳一次，
//!   用户看到的就是"屏幕不停闪"**，而且每轮两次全屏 BitBlt 本身也会让画面抖。
//!   现在改为：会话开始时**隐藏系统光标**（Windows `ShowCursor(FALSE)`，
//!   macOS `CGDisplayHideCursor`，都不需要额外权限）并一次性把光标送到滚动点，
//!   之后每轮只做**一次选区采集**（`capture_region`，不再整屏 BitBlt + crop）。
//!   光标隐藏期间用户看不到光标，也就没有瞬移的观感；会话结束（Drop）必然恢复。
//!   隐藏失败（罕见）才回退到旧的双帧治愈逻辑，见 `grab_frame`。
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
const MAX_SETTLE_MS: u64 = 800;
/// 稳态轮询间隔。100ms 时"已静止"要白等一轮才被发现，观感就是一步一卡；
/// 50ms 让等待随动画结束立即收尾，长截图节奏更跟手。
const SETTLE_POLL_MS: u64 = 50;

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
    /// 下一步滚动量（**物理像素**，自适应调整，见 `step_auto`）
    next_px: i32,
    /// 滚动量下限 / 上限（上限 = 3/4 帧高：再大就没有可拼的重叠了）
    min_px: i32,
    max_px: i32,
    hide_ui: Option<UiCallback>,
    show_ui: Option<UiCallback>,
    /// 外部停止标志（用户按 Esc 时置位；None = 不可中断，测试用）
    stop: Option<Arc<AtomicBool>>,
    /// 会话期间系统光标是否被本会话隐藏（决定采集走单帧还是双帧治愈，
    /// 以及 Drop 时要不要恢复显示）。
    cursor_hidden: bool,
    /// 帧相对选区的像素倍率（**各平台不同**）。
    ///
    /// - Windows：`capture_region` 收物理像素、返回物理像素 → 1
    /// - macOS：`capture_region` 收**逻辑点**、返回**物理像素**
    ///   → Retina 上是 2（例：传 1680x925，返回 3360x1850）
    ///
    /// 这不是可以忽略的细节：双帧治愈里 `cx = 光标x - rect.x` 是拿逻辑
    /// 选区算的，直接当帧内坐标用会差整整一个倍率（光标残影留在别处）。
    frame_scale: f32,
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
        // 起始滚动量（物理像素）；之后由 `step_auto` 按拼接结果自适应
        init_px: i32,
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

        // 第一帧：当前选区内容（此时用户刚框选完，页面静止）。
        //
        // 光标策略：先隐藏系统光标，再把光标一次性送到滚动点——隐藏状态下
        // 这次移动用户看不见，也不会被采进帧里。之后整个会话不再动光标，
        // 滚动事件照样能落到光标所在窗口（滚动点）。
        let cursor_hidden = hide_cursor();
        set_cursor_pos(scroll_point.0, scroll_point.1);
        let first = match if cursor_hidden {
            grab_once(monitor.get(), &rect, &hide_ui, &show_ui)
        } else {
            // 隐藏失败（极少数平台/远程桌面）：退回双帧治愈，保证图里不进光标
            // 倍率未知时按 1 处理（治疗框位置偏一点，总比不出图强）
            grab_with_heal(monitor.get(), &rect, 1.0, scroll_point, &hide_ui, &show_ui)
        } {
            Ok(f) => f,
            Err(e) => {
                // 建会话失败也必须把光标还回去，否则用户看到的是"没有鼠标箭头"
                if cursor_hidden {
                    show_cursor();
                }
                return Err(e);
            }
        };
        // 帧尺寸 = 选区 × 平台倍率（macOS Retina 上是 2）。
        //
        // 原来这里要求「完全相等」，结果 macOS 上必然失败：
        //   selection 1680x925 outside monitor bounds (got 3360x1850)
        // 3360 = 1680*2、1850 = 925*2 —— 不是越界，是**帧分辨率就是物理像素**。
        // 判据改成「两个方向倍率一致 + 不小于 1」，这样既能容纳平台差异，
        // 又能继续挡住真正越界（被系统裁掉一块时两个方向的倍率会对不上）。
        let sx = first.width as f32 / rect.w as f32;
        let sy = first.height as f32 / rect.h as f32;
        if (sx - sy).abs() > 0.02 || sx < 0.99 {
            if cursor_hidden {
                show_cursor();
            }
            return Err(format!(
                "selection {}x{} outside monitor bounds (got {}x{}, scale {:.2}x{:.2})",
                rect.w, rect.h, first.width, first.height, sx, sy
            ));
        }
        let stitcher = match Stitcher::new(&first, opts) {
            Ok(s) => s,
            Err(e) => {
                if cursor_hidden {
                    show_cursor();
                }
                return Err(e.to_string());
            }
        };
        Ok(Self {
            monitor,
            rect,
            scroll_point,
            stitcher,
            // 起始步进由调用方给（默认 1/6 帧高），夹在 [8, 3/4 帧高]：
            // 太大没有可拼重叠，太小一步几乎看不出滚动。之后 step_auto 自适应。
            next_px: init_px.clamp(8, (rect.h / 4 * 3).max(64) as i32),
            min_px: 8,
            max_px: (rect.h / 4 * 3).max(64) as i32,
            hide_ui,
            show_ui,
            stop,
            cursor_hidden,
            frame_scale: sx,
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

    /// 采集结束后立即还原光标；结果窗仍可继续持有本会话的画布。
    pub fn restore_cursor(&mut self) {
        if self.cursor_hidden {
            show_cursor();
            self.cursor_hidden = false;
        }
    }

    /// 结束会话，取出最终长图。
    ///
    /// 因为 `LongCapture` 实现了 `Drop`（恢复光标），不能直接按字段 move，
    /// 这里用 `ManuallyDrop` 阻止自动 drop、由本函数显式完成"恢复光标"，
    /// 再把 `stitcher` 读出来（`ptr::read` 配合 ManuallyDrop 是标准做法：
    /// 原值不会被二次 drop，`cursor_hidden` 是 Copy 类型无所谓）。
    pub fn into_canvas(self) -> RgbaBuffer {
        let this = std::mem::ManuallyDrop::new(self);
        if this.cursor_hidden {
            show_cursor();
        }
        // SAFETY: `this` 是 ManuallyDrop，函数返回后不会有 Drop 跑到 stitcher；
        // 这里把 stitcher 的所有权读出来交给调用方，正好一次。
        let stitcher = unsafe { std::ptr::read(&this.stitcher) };
        stitcher.into_canvas()
    }

    /// 推进一步：滚动 → 等稳态 → 采集 → 拼接 → **按结果自适应下一步滚动量**。
    ///
    /// 为什么自适应：不同浏览器/不同页面对同一个合成滚轮事件的实际滚动量差异
    /// 极大（0.1.13 真机上 Chrome 对同一事件的反应和系统 App 完全不同），固定
    /// "3 行"不是滚太少（页面几乎不动、重叠超出匹配范围）就是滚太多（一帧跨过
    /// 半个视口、没有重叠可拼）。这里用上一步的拼接结果自动调：
    /// - `Appended` 拼上了 → 步进不变（已经合适）；
    /// - `Still` 画面没动 → 步进 ×2（上限 3/4 帧高）；
    /// - `NoMatch` 拼不上 → 步进 ÷2（下限 8px）。
    ///
    /// 等待稳态期间若 `stop` 被置位，返回 `StepOutcome::Stopped`（本步作废，
    /// 拼接器保持原状，调用方应结束会话并取 `into_canvas()`）。
    ///
    /// 光标在 `new()` 时已一次性送到滚动点并隐藏，本函数**不再碰光标**：
    /// 旧实现每步 `SetCursorPos` 一次、每帧瞬移两次，是"屏幕不停闪"的主因。
    pub fn step_auto(&mut self) -> Result<StepOutcome, String> {
        let px = self.next_px;
        scroll_wheel_px(px, self.scroll_point)?;
        // CGEventPost 异步投递；立刻取帧会把尚未开始滚动的页面误判为到底。
        #[cfg(target_os = "macos")]
        std::thread::sleep(Duration::from_millis(160));
        let frame = match self.grab_stable()? {
            Some(f) => f,
            None => return Ok(StepOutcome::Stopped),
        };
        let step = self.stitcher.push(&frame).map_err(|e| e.to_string())?;
        // 自适应：只在上一次失败的方向上调整，成功就稳住
        self.next_px = match step.kind {
            StitchStepKind::Appended => self.next_px,
            StitchStepKind::Still => (self.next_px * 2).min(self.max_px),
            StitchStepKind::NoMatch => (self.next_px / 2).max(self.min_px),
        };
        Ok(StepOutcome::Stepped(step))
    }

    /// 下一步的滚动量（物理像素）。供日志/测试观察自适应过程。
    pub fn next_px(&self) -> i32 {
        self.next_px
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
        if self.cursor_hidden {
            // 光标已隐藏：单帧即可，无需治愈
            grab_once(
                self.monitor.get(),
                &self.rect,
                &self.hide_ui,
                &self.show_ui,
            )
        } else {
            grab_with_heal(
                self.monitor.get(),
                &self.rect,
                self.frame_scale,
                self.scroll_point,
                &self.hide_ui,
                &self.show_ui,
            )
        }
    }
}

/// 会话结束（正常取出 / 异常丢弃）时恢复系统光标。
///
/// 这是"光标隐藏"方案的保险：任何路径退出都必须让鼠标箭头回来，
/// 否则用户会以为系统坏了。`into_canvas()` 消耗 self 前也会走到这里。
impl Drop for LongCapture {
    fn drop(&mut self) {
        if self.cursor_hidden {
            show_cursor();
        }
    }
}

/// 采一帧选区（物理像素）。光标已隐藏或已被治愈，直接采。
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
        // 只采选区：xcap 0.9.8 的 capture_region 直接问系统要这块矩形，
        // 比"整屏 BitBlt + crop"少一次全屏拷贝——既快，也少一次全屏读，
        // 屏幕观感更稳（整屏采集在高分屏上本身就会让画面抖一下）。
        monitor
            .capture_region(rect.x.max(0) as u32, rect.y.max(0) as u32, rect.w, rect.h)
            .map_err(|e| format!("capture failed: {e}"))
            .and_then(|img| RgbaBuffer::new(img.width(), img.height(), img.into_raw()).map_err(|e| e.to_string()))
    })();
    if let Some(f) = show_ui {
        f();
    }
    result
}

/// 采一帧选区，并用"移开光标再采一帧"治愈光标残影（光标隐藏失败时的兜底）。
fn grab_with_heal(
    monitor: &xcap::Monitor,
    rect: &Rect,
    frame_scale: f32,
    scroll_point: (i32, i32),
    hide_ui: &Option<UiCallback>,
    show_ui: &Option<UiCallback>,
) -> Result<RgbaBuffer, String> {
    let mut a = grab_once(monitor, rect, hide_ui, show_ui)?;
    let (px, py) = scroll_point;
    // 帧内坐标：光标热点相对选区左上角，再乘平台倍率
    // （rect 是选区坐标系，帧是物理像素；macOS 上两者差 2 倍）
    let cx = ((px - rect.x) as f32 * frame_scale).round() as i32;
    let cy = ((py - rect.y) as f32 * frame_scale).round() as i32;
    // 移开光标采第二帧（页面静止时与 a 内容一致）
    set_cursor_pos(px + CURSOR_NUDGE.0, py + CURSOR_NUDGE.1);
    let b = grab_once(monitor, rect, hide_ui, show_ui)?;
    // 光标移回滚轮点，供下一步滚动使用
    set_cursor_pos(px, py);
    heal_cursor_at(&mut a, &b, cx, cy, CURSOR_HEAL_HALF);
    Ok(a)
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

/// 隐藏系统光标（长截图会话期间调用）。
///
/// 目的不是"界面好看"，而是**采集层不再需要瞬移光标**：光标隐藏后
/// - 采集帧里天然没有光标，不需要双帧治愈（每帧省一次全屏采集）；
/// - 会话开始时把光标送到滚动点这一次移动用户看不见，不会觉得屏幕在闪。
///
/// Windows `ShowCursor` 是**按线程**的引用计数；本进程的长截图驱动跑在
/// 阻塞线程里，调用与恢复都在同一线程，计数配平。macOS
/// `CGDisplayHideCursor` 不需要任何权限。
///
/// 返回 false = 当前平台不支持（调用方回退到双帧治愈逻辑）。
pub fn hide_cursor() -> bool {
    #[cfg(windows)]
    {
        extern "system" {
            fn ShowCursor(show: i32) -> i32;
        }
        // SAFETY: 纯值参数。一次 FALSE 即可把可见光标收起来
        // （计数器 ≥1 时减到 0 即隐藏）。
        unsafe {
            ShowCursor(0) >= 0
        }
    }
    #[cfg(target_os = "macos")]
    {
        extern "C" {
            fn CGMainDisplayID() -> u32;
            fn CGDisplayHideCursor(display: u32) -> i32;
        }
        // SAFETY: 纯值参数（display id）
        unsafe {
            CGDisplayHideCursor(CGMainDisplayID()) == 0
        }
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        false
    }
}

/// 恢复系统光标（会话结束 / 建会话失败时调用）。
///
/// 与 `hide_cursor` 成对出现；本函数在 `LongCapture::drop` 里被调用，
/// 是"光标必须回来"的最终保险。
pub fn show_cursor() {
    #[cfg(windows)]
    {
        extern "system" {
            fn ShowCursor(show: i32) -> i32;
        }
        // SAFETY: 纯值参数。内部计数从 -1 回到 0，光标重新可见。
        unsafe {
            ShowCursor(1);
        }
    }
    #[cfg(target_os = "macos")]
    {
        extern "C" {
            fn CGMainDisplayID() -> u32;
            fn CGDisplayShowCursor(display: u32) -> i32;
        }
        // SAFETY: 纯值参数（display id）
        unsafe {
            CGDisplayShowCursor(CGMainDisplayID());
        }
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {}
}

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

/// 注入滚轮事件，向下滚动 `px` **物理像素**（自适应步进的唯一入口）。
///
/// 调用前光标应已位于滚动点（`new()` 里一次性送过去并隐藏）。
///
/// `at` 是滚动目标点（显示器原点 + 选区中心的绝对坐标，与光标坐标系一致）。
/// Windows 用"当前光标位置"即可（调用方已提前把光标送过去），`at` 只被
/// macOS 用来显式设置合成事件的发生点 —— 否则事件默认落在 (0,0)，
/// 系统会把滚轮投给屏幕左上角的窗口而不是目标页面。
///
/// 为什么用像素而不是"格"：Windows 一格 = 120 delta ≈ 3 行 ≈ 120px，但 macOS 上
/// "行"是个与 App 相关的抽象（系统偏好里 1~3 行可调，浏览器还会再乘），同一个
/// 事件在不同页面上实际滚出来的距离能差出几倍 —— 固定格数必然导致"滚太少拼不上"
/// 或"滚太多没重叠"。像素是唯一可预测、可自适应的单位。
pub fn scroll_wheel_px(px: i32, at: (i32, i32)) -> Result<(), String> {
    let px = px.max(1);
    #[cfg(not(target_os = "macos"))]
    let _ = at;
    #[cfg(windows)]
    {
        // Windows 只有"格"这个单位：120 delta = 一格 ≈ 3 行。向上取整到整格，
        // 至少滚一格，否则和没滚没区别。
        const MOUSEEVENTF_WHEEL: u32 = 0x0800;
        const WHEEL_DELTA: i32 = 120;
        let lines = (px / WHEEL_DELTA).max(1);
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
        let _ = px; // macOS 直接用像素，无需换算
        scroll_wheel_event(px, at)
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        let _ = (px, at);
        Err("当前平台暂不支持自动滚动，请使用手动滚动模式".to_string())
    }
}

/// macOS：合成一个**像素增量**的滚轮事件并投到 HID tap。
///
/// 0.1.12 真机第一次真正跑到自动滚动时：权限有、事件也 post 出去了，页面却一步
/// 不动。根因是 `CGEventCreateScrollWheelEvent(kCGScrollEventUnitLine, ...)` 只填
/// **line delta**，而 Chromium/WebKit 读的是 **point delta**
/// （NSEvent.scrollingDeltaY）—— 真实鼠标两个字段都填，合成事件只填一个时浏览器
/// 视作 0px。所以这里单位用 pixel，并把 line / point / fixed-point 三个 delta 与
/// `is_continuous` 一起显式写全。
#[cfg(target_os = "macos")]
fn scroll_wheel_event(px: i32, at: (i32, i32)) -> Result<(), String> {
    {
        // **单位用像素**（见 `scroll_wheel_px` 的说明）：Chrome / Safari 只认
        // point delta，line delta 对它们等于 0px。
        const K_CG_SCROLL_EVENT_UNIT_PIXEL: u32 = 0;
        const K_CG_HID_EVENT_TAP: u32 = 0; // CGEventTapLocation 枚举首项
        // line delta 按"一行 ≈ 40px"折算，只为兼容读 line delta 的老 App
        const PX_PER_LINE: i32 = 40;
        // CGEventField 的字段号（CGEventTypes.h）
        const F_DELTA_AXIS1: u32 = 11;
        const F_FIXED_PT_DELTA_AXIS1: u32 = 93;
        const F_POINT_DELTA_AXIS1: u32 = 96;
        const F_IS_CONTINUOUS: u32 = 88;
        #[repr(C)]
        #[derive(Clone, Copy)]
        struct CGPoint {
            x: f64,
            y: f64,
        }
        extern "C" {
            fn AXIsProcessTrusted() -> bool;
            fn CGEventCreateScrollWheelEvent(
                source: *const std::ffi::c_void,
                units: u32,
                wheel_count: u32,
                wheel1: i32,
            ) -> *const std::ffi::c_void;
            fn CGEventPost(tap: u32, event: *const std::ffi::c_void);
            fn CGEventSetLocation(event: *const std::ffi::c_void, location: CGPoint);
            fn CGEventSetIntegerValueField(event: *const std::ffi::c_void, field: u32, value: i64);
            fn CGEventSetDoubleValueField(event: *const std::ffi::c_void, field: u32, value: f64);
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
        // 负值 = 向后（向下）滚；调用方传入的是"向下多少像素"
        let px = -px;
        let lines = (px.abs() / PX_PER_LINE).max(1);
        unsafe {
            let event = CGEventCreateScrollWheelEvent(
                std::ptr::null(),
                K_CG_SCROLL_EVENT_UNIT_PIXEL,
                1,
                px,
            );
            if event.is_null() {
                return Err("CGEventCreateScrollWheelEvent failed".to_string());
            }
            // 三个 delta 字段都写：不同 App 读的字段不一样（WebKit 读
            // scrollingDeltaY / Chromium 读 point delta，老 App 读 line delta）
            CGEventSetIntegerValueField(event, F_POINT_DELTA_AXIS1, px as i64);
            CGEventSetDoubleValueField(event, F_FIXED_PT_DELTA_AXIS1, px as f64);
            CGEventSetIntegerValueField(event, F_DELTA_AXIS1, -lines as i64);
            // **标成"精确增量"（continuous）**：0.1.12 只填 line delta 时浏览器
            // 一步都没滚，说明它走的是 precise 分支（读 point delta）而不是
            // wheel ticks 分支。标 1 之后浏览器按 point delta 的像素值直接滚，
            // 这也是各种合成滚动工具对 Chrome 的标准写法。
            CGEventSetIntegerValueField(event, F_IS_CONTINUOUS, 1);
            // **必须显式设置事件位置**：合成滚轮事件的默认位置是 (0,0)，
            // 系统按该位置做窗口命中测试 —— 滚的是屏幕左上角那个窗口，
            // 而不是光标下的页面（表现为"滚了但页面不动"，和没权限一模一样）。
            CGEventSetLocation(
                event,
                CGPoint {
                    x: at.0 as f64,
                    y: at.1 as f64,
                },
            );
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
