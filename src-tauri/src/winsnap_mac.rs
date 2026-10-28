//! macOS 窗口吸附实现（CGWindowList）。
//!
//! 与 Windows 那套（`winsnap.rs` 的 `platform` 模块）目标一致：
//! 找出光标下方的顶层窗口，供浮层高亮 + 单击选中整窗。
//!
//! **为什么单独成文件**：它刻意不依赖父模块的类型（返回自己的 `MacWindow`），
//! 这样可以用
//! `rustc --emit=metadata --target aarch64-apple-darwin winsnap_mac.rs`
//! 单独做类型检查 —— Windows 机器上没有 macOS 交叉工具链，
//! `cargo check --target aarch64-apple-darwin` 会卡在 objc2 的 C 编译步骤，
//! 这是唯一能在非 mac 机器上验证这些 FFI 能否编译通过的办法。
//!
//! 几个 macOS 特有的坑（与 Windows 相反）：
//! 1. **坐标单位是"点"不是像素**：`CGWindowList` 的 bounds 与
//!    `CGDisplayBounds`（我们的 `MonitorShot.x/y`）都在全局屏幕坐标里、
//!    单位是点，所以 `屏幕点 = 显示器原点 + 浮层内坐标`，
//!    **不要乘 scale_factor**（Windows 那边才要）。
//! 2. `CGWindowListCopyWindowInfo` 返回的数组**本身按 z 序从前到后**排列，
//!    所以第一个命中的就是最上层窗口，不需要自己做 z 序回溯。
//! 3. `kCGWindowLayer != 0` 的是菜单栏 / Dock / 浮动面板 / 输入法候选框，
//!    不是"窗口"，必须跳过。
//! 4. `kCGWindowName`（窗口标题）在未授予屏幕录制权限时可能为空，
//!    此时退回 `kCGWindowOwnerName`（应用名）。
//! 5. 必须 `CFRelease` 掉返回的数组，否则每次悬停查询（~15fps）都漏一块内存。

use core::ffi::c_void;

type CFTypeRef = *const c_void;
type CFArrayRef = *const c_void;
type CFDictionaryRef = *const c_void;
type CFStringRef = *const c_void;
type CFNumberRef = *const c_void;

#[repr(C)]
#[derive(Default, Clone, Copy)]
struct CGPoint {
    x: f64,
    y: f64,
}

#[repr(C)]
#[derive(Default, Clone, Copy)]
struct CGSize {
    width: f64,
    height: f64,
}

#[repr(C)]
#[derive(Default, Clone, Copy)]
struct CGRect {
    origin: CGPoint,
    size: CGSize,
}

// `kCGWindowListOptionOnScreenOnly`(1<<0) | `kCGWindowListExcludeDesktopElements`(1<<4)
const LIST_OPTS: u32 = (1 << 0) | (1 << 4);
/// `kCFNumberSInt64Type`
const NUM_SINT64: i32 = 4;
/// `kCFStringEncodingUTF8`
const ENC_UTF8: u32 = 0x0800_0100;

// CoreGraphics / CoreFoundation。
//
// **必须显式声明 framework**：Rust 只把 `extern "C"` 当"符号在外面"，
// 不会自动加 `-framework`。虽然 Tauri/wry 自己会链 CoreGraphics，
// 靠传递链接参数能"碰巧"解析到，但那是隐式依赖 —— 换构建配置就炸。
#[link(name = "CoreGraphics", kind = "framework")]
#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    fn CGWindowListCopyWindowInfo(option: u32, relative: u32) -> CFArrayRef;
    fn CGRectMakeWithDictionaryRepresentation(dict: CFDictionaryRef, rect: *mut CGRect) -> bool;
    fn CFArrayGetCount(arr: CFArrayRef) -> isize;
    fn CFArrayGetValueAtIndex(arr: CFArrayRef, idx: isize) -> CFTypeRef;
    fn CFDictionaryGetValue(dict: CFDictionaryRef, key: *const c_void) -> CFTypeRef;
    fn CFNumberGetValue(num: CFNumberRef, kind: i32, out: *mut c_void) -> bool;
    fn CFStringGetCString(s: CFStringRef, buf: *mut i8, size: isize, enc: u32) -> bool;
    fn CFRelease(cf: CFTypeRef);
    fn CFStringCreateWithCString(allocator: CFTypeRef, text: *const i8, encoding: u32)
        -> CFStringRef;

    // CoreGraphics 以全局常量形式导出的字典键
    static kCGWindowBounds: CFStringRef;
    static kCGWindowLayer: CFStringRef;
    static kCGWindowName: CFStringRef;
    static kCGWindowNumber: CFStringRef;
    static kCGWindowOwnerName: CFStringRef;
    static kCGWindowOwnerPID: CFStringRef;
}

#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    fn AXUIElementCreateApplication(pid: i32) -> CFTypeRef;
    fn AXUIElementCopyElementAtPosition(
        application: CFTypeRef,
        x: f32,
        y: f32,
        element: *mut CFTypeRef,
    ) -> i32;
    fn AXUIElementCopyAttributeValue(
        element: CFTypeRef,
        attribute: CFStringRef,
        value: *mut CFTypeRef,
    ) -> i32;
    fn AXValueGetValue(value: CFTypeRef, kind: u32, out: *mut c_void) -> bool;
}

// Objective-C 运行时（`objc_msgSend` / `sel_registerName` 在 libobjc.A.dylib）
#[link(name = "objc")]
extern "C" {
    fn objc_msgSend(obj: *mut c_void, sel: *mut c_void) -> isize;
    fn sel_registerName(name: *const i8) -> *mut c_void;
}

/// 吸附结果（屏幕点坐标，已扣掉显示器原点 = 浮层内逻辑坐标）。
pub struct MacWindow {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
    pub title: String,
}

fn ax_attribute(name: &'static [u8]) -> CFStringRef {
    unsafe { CFStringCreateWithCString(core::ptr::null(), name.as_ptr().cast(), ENC_UTF8) }
}

fn ax_rect(element: CFTypeRef, position: CFStringRef, size: CFStringRef) -> Option<CGRect> {
    let mut position_value: CFTypeRef = core::ptr::null();
    let mut size_value: CFTypeRef = core::ptr::null();
    let position_ok = unsafe { AXUIElementCopyAttributeValue(element, position, &mut position_value) == 0 };
    let size_ok = unsafe { AXUIElementCopyAttributeValue(element, size, &mut size_value) == 0 };
    let mut origin = CGPoint::default();
    let mut dimensions = CGSize::default();
    let valid = position_ok
        && size_ok
        && !position_value.is_null()
        && !size_value.is_null()
        && unsafe { AXValueGetValue(position_value, 1, (&mut origin as *mut CGPoint).cast()) }
        && unsafe { AXValueGetValue(size_value, 2, (&mut dimensions as *mut CGSize).cast()) };
    if !position_value.is_null() {
        unsafe { CFRelease(position_value) };
    }
    if !size_value.is_null() {
        unsafe { CFRelease(size_value) };
    }
    valid.then_some(CGRect { origin, size: dimensions })
}

/// 在命中应用的辅助功能树中向上找可截图的区域；权限不足时回退整窗。
fn ax_region(pid: i64, x: f64, y: f64, window: CGRect) -> Option<CGRect> {
    let app = unsafe { AXUIElementCreateApplication(i32::try_from(pid).ok()?) };
    if app.is_null() {
        return None;
    }
    let mut element: CFTypeRef = core::ptr::null();
    let hit = unsafe { AXUIElementCopyElementAtPosition(app, x as f32, y as f32, &mut element) };
    unsafe { CFRelease(app) };
    if hit != 0 || element.is_null() {
        return None;
    }

    let position = ax_attribute(b"AXPosition\0");
    let size = ax_attribute(b"AXSize\0");
    let parent = ax_attribute(b"AXParent\0");
    if position.is_null() || size.is_null() || parent.is_null() {
        unsafe { CFRelease(element) };
        for key in [position, size, parent] {
            if !key.is_null() {
                unsafe { CFRelease(key) };
            }
        }
        return None;
    }

    let window_area = window.size.width * window.size.height;
    let mut best: Option<CGRect> = None;
    for _ in 0..16 {
        if let Some(rect) = ax_rect(element, position, size) {
            let area = rect.size.width * rect.size.height;
            if rect.size.width >= 120.0
                && rect.size.height >= 32.0
                && area >= 30_000.0
                && area < window_area * 0.95
                && rect.origin.x >= window.origin.x - 2.0
                && rect.origin.y >= window.origin.y - 2.0
                && rect.origin.x + rect.size.width <= window.origin.x + window.size.width + 2.0
                && rect.origin.y + rect.size.height <= window.origin.y + window.size.height + 2.0
            {
                if best.map_or(true, |current| area < current.size.width * current.size.height) {
                    best = Some(rect);
                }
            }
        }
        let mut next: CFTypeRef = core::ptr::null();
        let status = unsafe { AXUIElementCopyAttributeValue(element, parent, &mut next) };
        unsafe { CFRelease(element) };
        if status != 0 || next.is_null() {
            break;
        }
        element = next;
    }
    for key in [position, size, parent] {
        unsafe { CFRelease(key) };
    }
    best
}

fn dict_get(dict: CFDictionaryRef, key: CFStringRef) -> CFTypeRef {
    if dict.is_null() || key.is_null() {
        return core::ptr::null();
    }
    unsafe { CFDictionaryGetValue(dict, key) }
}

unsafe fn dict_int(dict: CFDictionaryRef, key: CFStringRef) -> i64 {
    let v = dict_get(dict, key) as CFNumberRef;
    if v.is_null() {
        return 0;
    }
    let mut out: i64 = 0;
    if unsafe { CFNumberGetValue(v, NUM_SINT64, &mut out as *mut i64 as *mut c_void) } {
        out
    } else {
        0
    }
}

unsafe fn dict_string(dict: CFDictionaryRef, key: CFStringRef) -> String {
    let v = dict_get(dict, key) as CFStringRef;
    if v.is_null() {
        return String::new();
    }
    let mut buf = [0i8; 512];
    let ok = unsafe { CFStringGetCString(v, buf.as_mut_ptr(), buf.len() as isize, ENC_UTF8) };
    if !ok {
        return String::new();
    }
    let bytes: Vec<u8> = buf
        .iter()
        .take_while(|c| **c != 0)
        .map(|c| *c as u8)
        .collect();
    String::from_utf8_lossy(&bytes).into_owned()
}

/// NSWindow → CGWindowNumber（`CGWindowList` 里 `kCGWindowNumber` 的值）。
///
/// 用途：截图浮层自己也是"某个进程的普通窗口"，必须按窗口号跳过它，
/// 否则扫描第一个命中的永远是浮层自己。拿不到返回 0（调用方据此退化为
/// "跳过本进程全部窗口"，宁可不高亮也不能高亮浮层本身）。
pub fn ns_window_number(ns_window: *mut c_void) -> isize {
    if ns_window.is_null() {
        return 0;
    }
    unsafe {
        let sel = sel_registerName(b"windowNumber\0".as_ptr() as *const i8);
        if sel.is_null() {
            return 0;
        }
        objc_msgSend(ns_window, sel)
    }
}

/// 找光标处应吸附的顶层窗口。
///
/// `x/y` 是浮层内逻辑坐标，`mon_x/mon_y` 是该显示器在全局屏幕坐标里的原点
/// （都是点）；`skip` 是要跳过的窗口号（截图浮层自己）。
pub fn snap_at(x: f64, y: f64, mon_x: f64, mon_y: f64, skip: &[isize]) -> Option<MacWindow> {
    // 点坐标直接相加：macOS 的显示器原点与浮层坐标同为"点"
    let px = mon_x + x;
    let py = mon_y + y;
    let own_pid = std::process::id() as i64;

    let list = unsafe { CGWindowListCopyWindowInfo(LIST_OPTS, 0) };
    if list.is_null() {
        // 常见原因：没有屏幕录制权限 / 系统拒绝枚举
        return None;
    }
    let count = unsafe { CFArrayGetCount(list) };
    let mut found: Option<MacWindow> = None;

    for i in 0..count {
        let info = unsafe { CFArrayGetValueAtIndex(list, i) } as CFDictionaryRef;
        if info.is_null() {
            continue;
        }
        // 只要普通窗口层级（0）；跳过菜单栏、Dock、浮动面板、输入法候选框
        if unsafe { dict_int(info, kCGWindowLayer) } != 0 {
            continue;
        }
        let pid = unsafe { dict_int(info, kCGWindowOwnerPID) };
        let number = unsafe { dict_int(info, kCGWindowNumber) } as isize;
        // 本进程的窗口：只有明确在 skip 里（浮层自己）才跳过；拿不到窗口号时
        // 保守地跳过本进程全部窗口 —— 宁可不高亮，也不能把浮层当前景。
        if pid == own_pid && (skip.is_empty() || skip.contains(&number)) {
            continue;
        }
        let bounds_dict = dict_get(info, unsafe { kCGWindowBounds }) as CFDictionaryRef;
        if bounds_dict.is_null() {
            continue;
        }
        let mut rect = CGRect::default();
        if !unsafe { CGRectMakeWithDictionaryRepresentation(bounds_dict, &mut rect) } {
            continue;
        }
        // 太小的（tooltip / 拖动把手）吸附没意义
        if rect.size.width < 12.0 || rect.size.height < 12.0 {
            continue;
        }
        let inside = px >= rect.origin.x
            && px < rect.origin.x + rect.size.width
            && py >= rect.origin.y
            && py < rect.origin.y + rect.size.height;
        if !inside {
            continue;
        }
        // 命中即止：数组本身就是 z 序（最上层在前）
        let mut title = unsafe { dict_string(info, kCGWindowName) };
        if title.is_empty() {
            // 无标题窗口（或被系统隐去标题）时退回应用名，比空标签有用
            title = unsafe { dict_string(info, kCGWindowOwnerName) };
        }
        let region = ax_region(pid, px, py, rect).unwrap_or(rect);
        found = Some(MacWindow {
            x: region.origin.x - mon_x,
            y: region.origin.y - mon_y,
            w: region.size.width,
            h: region.size.height,
            title,
        });
        break;
    }

    unsafe { CFRelease(list) };
    found
}
