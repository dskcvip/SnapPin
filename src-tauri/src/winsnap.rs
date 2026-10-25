//! 窗口吸附：光标悬停时找出下方的顶层窗口，供浮层高亮 + 单击选中整窗。
//!
//! 参考行为：QQ 截图 / 微信截图 / Shottr 的"悬停识窗、单击选中"。
//!
//! **平台分派**：
//! - Windows：本文件的 `platform` 模块（EnumWindows + 沿 z 序扫 + DWM 边框）
//! - macOS：`winsnap_mac.rs`（CGWindowList，数组本身即 z 序）
//! - 其它（Linux）：空实现，返回 None = 不吸附
//!
//! 平台实现必须隔离：Win32 的 FFI 符号在 macOS 上不存在，混进 macOS 构建
//! 会直接链接失败（本项目 CI 会打 universal dmg）。

use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct WindowSnap {
    /// 逻辑坐标（与浮层选区同一坐标系；Windows 上已按 scale_factor 换算）
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
    /// 窗口标题（高亮框上的小标签用）
    pub title: String,
}

/// 把吸附矩形钳制进当前显示器（窗口可能跨屏，浮层只能选自己这块）。
///
/// 纯数学，与平台无关 —— 两个平台的原点都已换算成"浮层内逻辑坐标"。
pub fn clamp_to_monitor(snap: &mut WindowSnap, mon_x: f64, mon_y: f64, mon_w: f64, mon_h: f64) {
    let x0 = snap.x.max(mon_x);
    let y0 = snap.y.max(mon_y);
    let x1 = (snap.x + snap.w).min(mon_x + mon_w);
    let y1 = (snap.y + snap.h).min(mon_y + mon_h);
    snap.x = x0;
    snap.y = y0;
    snap.w = (x1 - x0).max(0.0);
    snap.h = (y1 - y0).max(0.0);
}

#[cfg(windows)]
mod platform {
    use super::WindowSnap;

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct POINT {
        x: i32,
        y: i32,
    }

    #[repr(C)]
    #[derive(Default)]
    struct RECT {
        left: i32,
        top: i32,
        right: i32,
        bottom: i32,
    }

    // 全部走 extern "system" 直接 FFI，不为几个函数引 windows-sys 依赖。
    // HWND 在 64 位下是指针宽度的不透明句柄，用 isize 承载。
    extern "system" {
        fn EnumWindows(
            cb: Option<unsafe extern "system" fn(isize, isize) -> i32>,
            lparam: isize,
        ) -> i32;
        fn IsWindowVisible(hwnd: isize) -> i32;
        fn GetWindowRect(hwnd: isize, rect: *mut RECT) -> i32;
        fn GetWindowTextW(hwnd: isize, text: *mut u16, count: i32) -> i32;
        fn GetClassNameW(hwnd: isize, text: *mut u16, count: i32) -> i32;
        fn GetWindowThreadProcessId(hwnd: isize, pid: *mut u32) -> u32;
        fn GetWindowLongW(hwnd: isize, index: i32) -> i32;
        fn DwmGetWindowAttribute(hwnd: isize, attr: u32, pv: *mut core::ffi::c_void, cb: u32)
            -> i32;
    }

    /// `DWMWA_EXTENDED_FRAME_BOUNDS`：返回**视觉可见**的窗口边界。
    /// `GetWindowRect` 会把 Win10+ 的窗口阴影（左右各约 8px 透明边）算进去，
    /// 直接拿来框选会多带一圈空白（实测 -8,-8/1936x1176 vs 真实 0,0/1920x1160）。
    const DWMWA_EXTENDED_FRAME_BOUNDS: u32 = 9;
    /// `DWMWA_CLOAKED`：UWP / 切虚拟桌面后窗口仍 IsWindowVisible=true 但不可见，
    /// 必须排掉，否则会吸附到一个"看不见的"窗口上。
    const DWMWA_CLOAKED: u32 = 14;
    const GWL_EXSTYLE: i32 = -20;
    const WS_EX_TRANSPARENT: i32 = 0x0000_0020;
    const WS_EX_NOACTIVATE: i32 = 0x0800_0000;

    /// 把当前**所有可见顶层窗口**的标题/类名/位置/尺寸/进程写进日志。
    ///
    /// 用途：用户报告"屏幕上多出一块灰色小窗"但截图认不出是谁 —— 与其猜，
    /// 不如在长截图结束那一刻把窗口清单打到日志里，一眼定位。
    pub fn log_window_inventory(tag: &str) {
        unsafe extern "system" fn cb(hwnd: isize, _lparam: isize) -> i32 {
            unsafe {
                if IsWindowVisible(hwnd) == 0 {
                    return 1;
                }
                let mut title = [0u16; 128];
                let n = GetWindowTextW(hwnd, title.as_mut_ptr(), 128);
                let title = String::from_utf16_lossy(&title[..n.max(0) as usize]);
                let mut cls = [0u16; 128];
                let cn = GetClassNameW(hwnd, cls.as_mut_ptr(), 128);
                let cls = String::from_utf16_lossy(&cls[..cn.max(0) as usize]);
                let mut r = RECT::default();
                GetWindowRect(hwnd, &mut r);
                let mut pid: u32 = 0;
                GetWindowThreadProcessId(hwnd, &mut pid);
                let w = r.right - r.left;
                let h = r.bottom - r.top;
                // 只记有可能被用户当成"多出来的小窗"的尺寸段，日志才不至于淹没
                if (60..1200).contains(&w) && (40..1200).contains(&h) {
                    crate::logln!(
                        "[win-inv] ({},{}) {}x{} pid={} class={cls:?} title={title:?}",
                        r.left, r.top, w, h, pid
                    );
                }
                1
            }
        }
        crate::logln!("[win-inv] ---- {tag} ----");
        unsafe {
            EnumWindows(Some(cb), 0);
        }
    }

    fn rect_of(hwnd: isize) -> RECT {
        let mut r = RECT::default();
        unsafe { GetWindowRect(hwnd, &mut r) };
        r
    }

    fn visual_rect(hwnd: isize) -> RECT {
        let mut r = RECT::default();
        let hr = unsafe {
            DwmGetWindowAttribute(
                hwnd,
                DWMWA_EXTENDED_FRAME_BOUNDS,
                &mut r as *mut RECT as *mut core::ffi::c_void,
                std::mem::size_of::<RECT>() as u32,
            )
        };
        if hr == 0 && r.right > r.left && r.bottom > r.top {
            r
        } else {
            // DWM 不可用（远程桌面/兼容模式）退回普通窗口矩形
            rect_of(hwnd)
        }
    }

    fn title_of(hwnd: isize) -> String {
        let mut buf = [0u16; 256];
        let n = unsafe { GetWindowTextW(hwnd, buf.as_mut_ptr(), buf.len() as i32) };
        if n <= 0 {
            return String::new();
        }
        String::from_utf16_lossy(&buf[..n as usize])
    }

    fn contains(r: &RECT, pt: POINT) -> bool {
        pt.x >= r.left && pt.x < r.right && pt.y >= r.top && pt.y < r.bottom
    }

    /// 点穿透 / 不激活的窗口（悬浮工具条之类）不该被吸附
    fn is_click_through(hwnd: isize) -> bool {
        let ex = unsafe { GetWindowLongW(hwnd, GWL_EXSTYLE) };
        ex & WS_EX_TRANSPARENT != 0 || ex & WS_EX_NOACTIVATE != 0
    }

    fn is_cloaked(hwnd: isize) -> bool {
        let mut cloaked: u32 = 0;
        let hr = unsafe {
            DwmGetWindowAttribute(
                hwnd,
                DWMWA_CLOAKED,
                &mut cloaked as *mut u32 as *mut core::ffi::c_void,
                std::mem::size_of::<u32>() as u32,
            )
        };
        hr == 0 && cloaked != 0
    }

    struct Scan<'a> {
        pt: POINT,
        found: isize,
        /// 要跳过的 HWND（只有截图浮层自己）。**不要按进程过滤**：
        /// 本进程的其它窗口（设置窗、贴图窗…）应当能被吸附，否则光标
        /// 停在设置窗上时会"穿过去"高亮到它背后的程序（用户实测反馈）。
        skip: &'a [isize],
    }

    unsafe extern "system" fn scan_cb(hwnd: isize, lparam: isize) -> i32 {
        let scan = &mut *(lparam as *mut Scan);
        if scan.found != 0 {
            return 0; // 已找到，停止枚举
        }
        if scan.skip.contains(&hwnd)
            || IsWindowVisible(hwnd) == 0
            || is_click_through(hwnd)
            || is_cloaked(hwnd)
        {
            return 1;
        }
        let r = visual_rect(hwnd);
        // 太小的是拖动把手 / tooltip 之类，吸附它没有意义
        if r.right - r.left < 12 || r.bottom - r.top < 12 {
            return 1;
        }
        if contains(&r, scan.pt) {
            scan.found = hwnd;
            return 0;
        }
        1
    }

    /// 找光标处应吸附的顶层窗口；没有返回 None。
    ///
    /// 为什么用 `EnumWindows` 而不是 `WindowFromPoint`：
    /// 浮层自己是全屏置顶窗口，`WindowFromPoint` 只会返回浮层（或其
    /// WebView2 子窗）。也不能沿 z 序用 `GetWindow(GW_HWNDNEXT)` 下探——
    /// topmost 窗口会困在置顶带里（文档明确），走两步就断了。
    /// `EnumWindows` 本身就是按 z 序（最上层在前）枚举所有顶层窗口。
    fn window_at_point(pt: POINT, skip: &[isize]) -> Option<isize> {
        let mut scan = Scan {
            pt,
            found: 0,
            skip,
        };
        unsafe {
            EnumWindows(Some(scan_cb), &mut scan as *mut Scan as isize);
        }
        if scan.found != 0 {
            Some(scan.found)
        } else {
            None
        }
    }

    /// 查询吸附窗口。`x/y` 是浮层内的逻辑坐标，`mon_x/mon_y/sf` 是当前显示器
    /// 的原点与缩放，用于逻辑 ↔ 物理换算；`skip` 是浮层自己的 HWND。
    pub fn snap_at(
        x: f64,
        y: f64,
        mon_x: f64,
        mon_y: f64,
        sf: f64,
        skip: &[isize],
    ) -> Option<WindowSnap> {
        let px = (mon_x + x * sf).round() as i32;
        let py = (mon_y + y * sf).round() as i32;
        let h = window_at_point(POINT { x: px, y: py }, skip)?;
        // 用 DWM 可见边界：GetWindowRect 含窗口阴影的透明边，框选会多一圈
        let r = visual_rect(h);
        let title = title_of(h);
        // 物理 → 逻辑
        Some(WindowSnap {
            x: (r.left as f64 - mon_x) / sf,
            y: (r.top as f64 - mon_y) / sf,
            w: (r.right - r.left) as f64 / sf,
            h: (r.bottom - r.top) as f64 / sf,
            title,
        })
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn point_in_rect() {
            let r = RECT {
                left: 10,
                top: 20,
                right: 110,
                bottom: 220,
            };
            assert!(contains(&r, POINT { x: 10, y: 20 }));
            assert!(contains(&r, POINT { x: 109, y: 219 }));
            assert!(!contains(&r, POINT { x: 110, y: 20 })); // 右开区间
            assert!(!contains(&r, POINT { x: 5, y: 20 }));
        }

        /// 人工探针（`cargo test -- --ignored manual_probe_desktop --nocapture`）：
        /// 在真实桌面上验证 EnumWindows + z 序 + DWM 边界。CI 不跑。
        #[test]
        #[ignore]
        fn manual_probe_desktop() {
            for (x, y) in [(900.0, 400.0), (100.0, 300.0), (1800.0, 1100.0)] {
                let s = snap_at(x, y, 0.0, 0.0, 1.0, &[]);
                println!("probe ({x},{y}) -> {s:?}");
            }
        }
    }
}

#[cfg(windows)]
pub use platform::snap_at;

/// 窗口清单诊断（排查"屏幕上多出来的灰框"用，见 platform 里的实现注释）。
#[cfg(windows)]
pub use platform::log_window_inventory;

/// macOS 实现（CGWindowList）。单独文件便于用
/// `rustc --emit=metadata --target aarch64-apple-darwin` 在不带 mac 工具链的
/// 机器上做类型检查（`cargo check --target` 会卡在 objc2 的 C 编译步骤）。
#[cfg(target_os = "macos")]
#[path = "winsnap_mac.rs"]
mod mac;

#[cfg(target_os = "macos")]
pub use mac::ns_window_number;

#[cfg(target_os = "macos")]
pub fn snap_at(
    x: f64,
    y: f64,
    mon_x: f64,
    mon_y: f64,
    _sf: f64,
    skip: &[isize],
) -> Option<WindowSnap> {
    // macOS 的坐标单位是"点"，与显示器原点同系，不需要乘 scale_factor
    // （Windows 那边才要，那部分换算在 platform::snap_at 内完成）。
    mac::snap_at(x, y, mon_x, mon_y, skip).map(|m| WindowSnap {
        x: m.x,
        y: m.y,
        w: m.w,
        h: m.h,
        title: m.title,
    })
}

#[cfg(not(any(windows, target_os = "macos")))]
pub fn snap_at(
    _x: f64,
    _y: f64,
    _mon_x: f64,
    _mon_y: f64,
    _sf: f64,
    _skip: &[isize],
) -> Option<WindowSnap> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clamp_cross_monitor_window() {
        // 窗口从副屏伸到主屏外：钳到主屏 (0,0,1920,1200)
        let mut s = WindowSnap {
            x: -300.0,
            y: 100.0,
            w: 1500.0,
            h: 900.0,
            title: "w".into(),
        };
        clamp_to_monitor(&mut s, 0.0, 0.0, 1920.0, 1200.0);
        assert_eq!((s.x, s.y, s.w, s.h), (0.0, 100.0, 1200.0, 900.0));
    }

    #[test]
    fn clamp_window_larger_than_monitor() {
        let mut s = WindowSnap {
            x: -5000.0,
            y: -5000.0,
            w: 99999.0,
            h: 99999.0,
            title: String::new(),
        };
        clamp_to_monitor(&mut s, 0.0, 0.0, 1920.0, 1200.0);
        assert_eq!((s.x, s.y, s.w, s.h), (0.0, 0.0, 1920.0, 1200.0));
    }

    #[test]
    fn clamp_keeps_inner_window_untouched() {
        let mut s = WindowSnap {
            x: 100.0,
            y: 100.0,
            w: 400.0,
            h: 300.0,
            title: String::new(),
        };
        clamp_to_monitor(&mut s, 0.0, 0.0, 1920.0, 1200.0);
        assert_eq!((s.x, s.y, s.w, s.h), (100.0, 100.0, 400.0, 300.0));
    }
}
