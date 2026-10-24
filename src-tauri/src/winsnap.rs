//! 窗口吸附：光标悬停时找出下方的顶层窗口，供浮层高亮 + 单击选中整窗。
//!
//! 参考行为：QQ 截图 / 微信截图 / Shottr 的"悬停识窗、单击选中"。
//!
//! **平台门控**：实现全部是 Win32，只在 Windows 编译；其它平台给空实现
//! （返回 None = 不吸附）。不能让 FFI 声明漏到 macOS 构建里——那里没有
//! EnumWindows 这些符号，会直接链接失败（本项目 CI 会打 macOS dmg）。

use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct WindowSnap {
    /// 逻辑坐标（与浮层选区同一坐标系，已按当前显示器 scale_factor 换算）
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
    /// 窗口标题（高亮框上的小标签用）
    pub title: String,
}

#[cfg(not(windows))]
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

#[cfg(not(windows))]
pub fn clamp_to_monitor(_s: &mut WindowSnap, _x: f64, _y: f64, _w: f64, _h: f64) {}

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

    /// 把吸附矩形钳制进当前显示器（窗口可能跨屏，浮层只能选自己这块）。
    pub fn clamp_to_monitor(
        snap: &mut WindowSnap,
        mon_x: f64,
        mon_y: f64,
        mon_w: f64,
        mon_h: f64,
    ) {
        let x0 = snap.x.max(mon_x);
        let y0 = snap.y.max(mon_y);
        let x1 = (snap.x + snap.w).min(mon_x + mon_w);
        let y1 = (snap.y + snap.h).min(mon_y + mon_h);
        snap.x = x0;
        snap.y = y0;
        snap.w = (x1 - x0).max(0.0);
        snap.h = (y1 - y0).max(0.0);
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
pub use platform::{clamp_to_monitor, snap_at};
