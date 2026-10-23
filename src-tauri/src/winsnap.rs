//! 窗口吸附（Windows）：光标悬停时找出下方的**别人家**顶层窗口，
//! 供浮层高亮 + 单击选中整窗。
//!
//! 为什么需要它：截图浮层是全屏置顶窗口，`WindowFromPoint` 只会返回浮层自己
//! （甚至它的 WebView2 子窗），拿不到背后的窗口。所以要：
//! 1. 先归到顶层（`GetAncestor(GA_ROOT)`）
//! 2. 是自己的/不可见的 → 沿 z 序往下走（`GetWindow(GW_HWNDPREV)`）
//! 3. 找到第一个可见、矩形包含该点的窗口即命中
//!
//! 参考行为：QQ 截图 / 微信截图 / Shottr 的"悬停识窗、单击选中"。

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

const GA_ROOT: u32 = 2;
const GW_HWNDPREV: u32 = 3;

// 全部走 extern "system" 直接 FFI，不为几个函数引 windows-sys 依赖。
// HWND 在 64 位下是指针宽度的不透明句柄，用 isize 承载。
extern "system" {
    fn WindowFromPoint(pt: POINT) -> isize;
    fn GetAncestor(hwnd: isize, flags: u32) -> isize;
    fn GetWindow(hwnd: isize, cmd: u32) -> isize;
    fn GetWindowThreadProcessId(hwnd: isize, pid: *mut u32) -> u32;
    fn IsWindowVisible(hwnd: isize) -> i32;
    fn GetWindowRect(hwnd: isize, rect: *mut RECT) -> i32;
    fn GetWindowTextW(hwnd: isize, text: *mut u16, count: i32) -> i32;
    fn DwmGetWindowAttribute(hwnd: isize, attr: u32, pv: *mut core::ffi::c_void, cb: u32) -> i32;
}

/// `DwmGetWindowAttribute(DWMWA_EXTENDED_FRAME_BOUNDS)`：
/// 返回**视觉可见**的窗口边界。`GetWindowRect` 会把 Win10+ 的窗口阴影
/// （左右各约 8px 透明边）算进去，直接用来框选会多带一圈空白。
const DWMWA_EXTENDED_FRAME_BOUNDS: u32 = 9;

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
        // DWM 不可用（如远程桌面/兼容模式）退回普通窗口矩形
        rect_of(hwnd)
    }
}

/// 我们的进程 id：任何属于本进程的窗口（浮层 / 悬浮球 / 菜单…）都要跳过，
/// 否则悬停会"吸附到自己身上"。
fn own_pid() -> u32 {
    std::process::id()
}

fn is_own(hwnd: isize) -> bool {
    let mut pid: u32 = 0;
    unsafe { GetWindowThreadProcessId(hwnd, &mut pid) };
    pid == own_pid()
}

fn rect_of(hwnd: isize) -> RECT {
    let mut r = RECT::default();
    unsafe { GetWindowRect(hwnd, &mut r) };
    r
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

/// 找光标处应吸附的顶层窗口；没有（光标下只有我们自己的窗口）返回 None。
fn window_at_point(pt: POINT) -> Option<isize> {
    let mut h = unsafe { WindowFromPoint(pt) };
    if h == 0 {
        return None;
    }
    // WebView2 子窗 / 控件 → 归到顶层窗口
    h = unsafe { GetAncestor(h, GA_ROOT) };
    let mut guard = 0;
    while h != 0 && guard < 128 {
        guard += 1;
        // 自己的窗口一律跳过（沿 z 序继续往下找）
        if is_own(h) || unsafe { IsWindowVisible(h) } == 0 {
            h = unsafe { GetWindow(h, GW_HWNDPREV) };
            continue;
        }
        let r = rect_of(h);
        // z 序里上方的窗口可能不含该点（比如在另一块屏上），继续往下
        if r.right - r.left < 8 || r.bottom - r.top < 8 {
            h = unsafe { GetWindow(h, GW_HWNDPREV) };
            continue;
        }
        if contains(&r, pt) {
            return Some(h);
        }
        h = unsafe { GetWindow(h, GW_HWNDPREV) };
    }
    None
}

/// 查询吸附窗口。`x/y` 是浮层内的逻辑坐标，`mon_x/mon_y/sf` 是当前显示器
/// 的原点与缩放，用于逻辑 ↔ 物理换算。
pub fn snap_at(x: f64, y: f64, mon_x: f64, mon_y: f64, sf: f64) -> Option<WindowSnap> {
    let px = (mon_x + x * sf).round() as i32;
    let py = (mon_y + y * sf).round() as i32;
    let h = window_at_point(POINT { x: px, y: py })?;
    // 用 DWM 可见边界：GetWindowRect 含窗口阴影的透明边，框选会多一圈
    let r = visual_rect(h);
    let title = title_of(h);
    // 物理 → 逻辑
    let lx = (r.left as f64 - mon_x) / sf;
    let ly = (r.top as f64 - mon_y) / sf;
    let lw = (r.right - r.left) as f64 / sf;
    let lh = (r.bottom - r.top) as f64 / sf;
    Some(WindowSnap {
        x: lx,
        y: ly,
        w: lw,
        h: lh,
        title,
    })
}

/// 把吸附矩形钳制进当前显示器（窗口可能跨屏，浮层只能选自己这块）。
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn point_in_rect() {
        let r = RECT { left: 10, top: 20, right: 110, bottom: 220 };
        assert!(contains(&r, POINT { x: 10, y: 20 }));
        assert!(contains(&r, POINT { x: 109, y: 219 }));
        assert!(!contains(&r, POINT { x: 110, y: 20 })); // 右开区间
        assert!(!contains(&r, POINT { x: 5, y: 20 }));
    }

    #[test]
    fn clamp_cross_monitor_window() {
        // 窗口从副屏伸到主屏外：钳到主屏 (0,0,1920,1200)
        let mut s = WindowSnap { x: -300.0, y: 100.0, w: 1500.0, h: 900.0, title: "w".into() };
        clamp_to_monitor(&mut s, 0.0, 0.0, 1920.0, 1200.0);
        assert_eq!((s.x, s.y, s.w, s.h), (0.0, 100.0, 1200.0, 900.0));
    }

    #[test]
    fn clamp_window_larger_than_monitor() {
        let mut s = WindowSnap { x: -5000.0, y: -5000.0, w: 99999.0, h: 99999.0, title: String::new() };
        clamp_to_monitor(&mut s, 0.0, 0.0, 1920.0, 1200.0);
        assert_eq!((s.x, s.y, s.w, s.h), (0.0, 0.0, 1920.0, 1200.0));
    }

    /// 人工探针（`cargo test -- --ignored manual_probe_desktop --nocapture`）：
    /// 在真实桌面上验证 WindowFromPoint + z 序回溯逻辑能找到窗口并算出矩形。
    /// CI 不跑（依赖桌面状态），但排查"吸附失效"时是最快的证据来源。
    #[test]
    #[ignore]
    fn manual_probe_desktop() {
        for (x, y) in [(900.0, 400.0), (100.0, 300.0), (1800.0, 1100.0)] {
            let s = snap_at(x, y, 0.0, 0.0, 1.0);
            println!("probe ({x},{y}) -> {s:?}");
        }
    }
}
