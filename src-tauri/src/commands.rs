//! IPC 命令层：前端的唯一入口。契约见 docs/CONTRACTS.md。

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager, State, Window};
use tauri_plugin_global_shortcut::GlobalShortcutExt;
use tauri_plugin_global_shortcut::Shortcut;

use crate::config::AppConfig;
use crate::state::{AppState, LongSession, MenuKind, PinFlags, PinMeta};
use crate::windows;
use snappin_core_algo::stitch::StitchOptions;
use snappin_core_algo::Rect as AlgoRect;
use snappin_core_platform::stitch::{
    write_thumbnail, LongCapture, StepOutcome, StitchStepKind, UiCallback,
};

// ---------- 载荷类型（响应字段统一 camelCase，与前端 TS 契约一致） ----------

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MonitorInfoPayload {
    pub id: String,
    pub name: String,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub scale_factor: f32,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ShotAssetPayload {
    pub monitor_id: String,
    pub path: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CapturePayload {
    pub monitors: Vec<MonitorInfoPayload>,
    pub shots: Vec<ShotAssetPayload>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OverlayContextPayload {
    pub monitor_id: String,
    pub image_path: String,
    pub natural_width: u32,
    pub natural_height: u32,
    pub scale_factor: f32,
    /// false = 采集中（前端应轮询等待）；true = 冻结图就绪
    pub ready: bool,
    /// 本会话代号：取消/确认时回传，用于丢弃上一轮迟到的请求
    pub session_id: u64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfirmPayload {
    pub path: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PinContextPayload {
    pub label: String,
    pub src: String,
    pub width: f64,
    pub height: f64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppInfoPayload {
    pub version: String,
    pub platform: String,
    pub arch: String,
}

/// 选区参数（前端 overlay 窗口的逻辑像素坐标）。
///
/// **必须是浮点**：逻辑坐标 = 物理像素 / scaleFactor，高 DPI 屏上天然带小数
/// （Retina 2x 下一个物理像素 = 0.5 逻辑像素；Windows 125%/150% 同理）。
/// 用 `i32`/`u32` 时 serde 会直接拒绝浮点（`invalid type: floating point
/// '484.71484375', expected u32`），在"恰好 100% 缩放"的机器上才碰巧不发作。
///
/// 保留小数而不是让前端 `Math.round`：2x 屏上取整等于丢掉半个物理像素，
/// 裁剪结果会与用户框的边界差一像素。转物理像素时统一 `round`（见各命令）。
#[derive(Deserialize)]
pub struct RectPayload {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

/// 标注图形载荷（前端 overlay 的标注层坐标：逻辑像素，原点=裁剪图左上角）。
///
/// 用 `#[serde(tag = "type")]` 内部标签判别，前端发
/// `{ type: "rect", x, y, w, h, color: [r,g,b,a], width }` 形态的对象。
/// 到 Rust 后由 `to_shapes` 统一乘 DPI 缩放因子转成物理像素。
#[derive(Deserialize, serde::Serialize)]
/// **两个 rename 的作用不同，缺一不可**：/// - `rename_all`：变体名 → JSON 的 `type` 值（`Rect` → `"rect"`）；
/// - `rename_all_fields`：变体**字段**名 → camelCase（`step_x` → `"stepX"`）。
///
/// 只写前者时，单词字段（x/y/w/h/color/width…）刚好两边同名，问题被掩盖；
/// 一旦出现多词字段（水印的 `step_x`/`step_y`）就会报
/// `missing field step_x` —— 前端按契约发的都是 camelCase。
///
/// 派生 `Serialize` 纯粹为了让契约测试能"解析→回写"双向校验字段名，
/// 运行期只反序列化（载荷方向是前端 → Rust）。
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum ShapePayload {
    Rect {
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        color: [u8; 4],
        width: f32,
    },
    Ellipse {
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        color: [u8; 4],
        width: f32,
    },
    Arrow {
        x1: f32,
        y1: f32,
        x2: f32,
        y2: f32,
        color: [u8; 4],
        width: f32,
    },
    Pen {
        points: Vec<[f32; 2]>,
        color: [u8; 4],
        width: f32,
    },
    Highlighter {
        points: Vec<[f32; 2]>,
        color: [u8; 4],
        width: f32,
    },
    Mosaic {
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        block: u32,
    },
    /// UI 层光栅化的位图（文字 / 序列号等需要字体的标注）。
    ///
    /// `png`：base64 编码的 PNG，**无 `data:` 前缀**，尺寸已是物理像素
    /// （前端按 DPR 光栅化）。用 PNG 而非裸 RGBA，是因为同一段文字
    /// 裸 RGBA 有上百 KB、PNG 只有几 KB，IPC 体积差两个数量级。
    Bitmap {
        x: f32,
        y: f32,
        png: String,
    },
    /// 聚光灯：区域外压暗。`dim` 为 0-255 的压暗强度，
    /// `radius` 为圆角半径（逻辑像素）。
    Spotlight {
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        radius: f32,
        dim: u8,
    },
    /// 橡皮擦：把轨迹还原成原图（擦掉已画上去的标注）。
    /// `radius` 为擦除半径（逻辑像素），轨迹点为逻辑像素。
    Eraser {
        points: Vec<[f32; 2]>,
        radius: f32,
    },
    /// 水印：把一小块位图按步进平铺进指定区域。
    ///
    /// `png` 为 UI 层用系统字体光栅化的**单块**水印（base64 PNG，无
    /// `data:` 前缀，尺寸已是物理像素）——字体/CJK/emoji 都不进核心层。
    /// `step` 为平铺步进（逻辑像素），`w`/`h` 为平铺区域（逻辑像素）。
    Watermark {
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        step_x: f32,
        step_y: f32,
        png: String,
    },
    /// 区域放大（放大镜）：把原图内容按 `zoom` 倍放大画回同一区域，
    /// `color`/`width` 为放大区域边框样式（逻辑像素）。
    ///
    /// `rename = "zoom"`：Rust 变体名 `ZoomRegion` 按 camelCase 会序列化成
    /// `"zoomRegion"`，而前端 `tools.ts` 的判别字段是 `type: 'zoom'`——
    /// 不显式指定就会报 `unknown variant zoom`（放大镜标注保存失败）。
    #[serde(rename = "zoom")]
    ZoomRegion {
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        zoom: f32,
        color: [u8; 4],
        width: f32,
    },
    /// 智能擦除：内容感知填充（零依赖近似 inpaint）。
    /// `iterations` 为 0 表示按区域尺寸自动收敛。
    SmartErase {
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        iterations: u32,
    },
}

impl ShapePayload {
    /// 转 core-algo 的纯值类型（逻辑像素 × sf → 物理像素）。
    ///
    /// 为什么返回 `Result`：位图载荷需要解 base64 + 解 PNG，属于可能失败的
    /// 解码操作。宁可把错误回传给前端（前端会显示"复制失败：…"），
    /// 也不要静默丢一层标注让用户拿到缺字的图。
    fn to_shape(&self, sf: f64) -> Result<snappin_core_algo::shapes::Shape, String> {
        use snappin_core_algo::shapes::{Shape, ShapeStyle};
        let sf = sf as f32;
        let style = |color: [u8; 4], width: f32| ShapeStyle::new(color, width * sf);
        let shape = match self {
            ShapePayload::Rect { x, y, w, h, color, width } => Shape::Rect {
                x: (*x * sf) as i32,
                y: (*y * sf) as i32,
                w: (*w * sf).max(1.0) as u32,
                h: (*h * sf).max(1.0) as u32,
                style: style(*color, *width),
            },
            ShapePayload::Ellipse { x, y, w, h, color, width } => Shape::Ellipse {
                x: (*x * sf) as i32,
                y: (*y * sf) as i32,
                w: (*w * sf).max(1.0) as u32,
                h: (*h * sf).max(1.0) as u32,
                style: style(*color, *width),
            },
            ShapePayload::Arrow { x1, y1, x2, y2, color, width } => Shape::Arrow {
                x1: (*x1 * sf) as i32,
                y1: (*y1 * sf) as i32,
                x2: (*x2 * sf) as i32,
                y2: (*y2 * sf) as i32,
                style: style(*color, *width),
            },
            ShapePayload::Pen { points, color, width } => Shape::Pen {
                points: points.iter().map(|p| (p[0] * sf, p[1] * sf)).collect(),
                style: style(*color, *width),
            },
            ShapePayload::Highlighter { points, color, width } => Shape::Highlighter {
                points: points.iter().map(|p| (p[0] * sf, p[1] * sf)).collect(),
                style: style(*color, *width),
            },
            ShapePayload::Mosaic { x, y, w, h, block } => Shape::Mosaic {
                x: (*x * sf) as i32,
                y: (*y * sf) as i32,
                w: (*w * sf).max(1.0) as u32,
                h: (*h * sf).max(1.0) as u32,
                // 块边长同样是逻辑像素：前端按"块看起来多大"调，
                // 到了物理像素必须一起乘 DPR，否则高 DPI 下块会偏小一半
                block: ((*block as f32 * sf).round() as u32).max(2),
            },
            // 位图已是物理像素：只把锚点换算过去，尺寸与像素原样使用
            ShapePayload::Bitmap { x, y, png } => {
                let (w, h, rgba) = decode_png_base64(png)?;
                Shape::Bitmap {
                    x: (*x * sf) as i32,
                    y: (*y * sf) as i32,
                    w,
                    h,
                    rgba,
                }
            }
            ShapePayload::Spotlight { x, y, w, h, radius, dim } => Shape::Spotlight {
                x: (*x * sf) as i32,
                y: (*y * sf) as i32,
                w: (*w * sf).max(1.0) as u32,
                h: (*h * sf).max(1.0) as u32,
                radius: (*radius * sf).max(0.0) as u32,
                dim: *dim,
            },
            ShapePayload::Eraser { points, radius } => Shape::Eraser {
                points: points.iter().map(|p| (p[0] * sf, p[1] * sf)).collect(),
                radius: radius * sf,
            },
            // 水印位图已是物理像素（前端按 DPR 光栅化）：只换算锚点、
            // 平铺区域与步进，tile 尺寸与像素原样使用
            ShapePayload::Watermark {
                x, y, w, h, step_x, step_y, png,
            } => {
                let (tile_w, tile_h, tile) = decode_png_base64(png)?;
                Shape::Watermark {
                    x: (*x * sf) as i32,
                    y: (*y * sf) as i32,
                    w: (*w * sf).max(1.0) as u32,
                    h: (*h * sf).max(1.0) as u32,
                    // 步进同为逻辑像素：高 DPI 下不乘 sf 会让水印挤成一团
                    step_x: ((*step_x * sf).round() as u32).max(1),
                    step_y: ((*step_y * sf).round() as u32).max(1),
                    tile_w,
                    tile_h,
                    tile,
                }
            }
            ShapePayload::ZoomRegion { x, y, w, h, zoom, color, width } => Shape::ZoomRegion {
                x: (*x * sf) as i32,
                y: (*y * sf) as i32,
                w: (*w * sf).max(1.0) as u32,
                h: (*h * sf).max(1.0) as u32,
                zoom: *zoom,
                style: style(*color, *width),
            },
            ShapePayload::SmartErase { x, y, w, h, iterations } => Shape::SmartErase {
                x: (*x * sf) as i32,
                y: (*y * sf) as i32,
                w: (*w * sf).max(1.0) as u32,
                h: (*h * sf).max(1.0) as u32,
                iterations: *iterations,
            },
        };
        Ok(shape)
    }
}

/// base64 PNG → (宽, 高, RGBA8)。
///
/// 位图来自可信的本应用前端，但仍按不可信输入处理：长度、尺寸、
/// 解码失败都要给出明确错误，不做任何 `unwrap`。
fn decode_png_base64(png_b64: &str) -> Result<(u32, u32, Vec<u8>), String> {
    let raw = decode_base64_bytes(png_b64)?;
    let img = image::load_from_memory_with_format(&raw, image::ImageFormat::Png)
        .map_err(|e| format!("bitmap png decode failed: {e}"))?
        .to_rgba8();
    let (w, h) = (img.width(), img.height());
    if w == 0 || h == 0 {
        return Err("bitmap has zero size".to_string());
    }
    Ok((w, h, img.into_raw()))
}

/// base64（可带 data URL 前缀）→ 原始字节。
///
/// 与 `decode_png_base64` 共用同一套防御：空载荷、非法 base64 都返回错误。
fn decode_base64_bytes(b64: &str) -> Result<Vec<u8>, String> {
    use base64::Engine as _;
    let body = b64
        .rsplit_once(',')
        .filter(|(head, _)| head.starts_with("data:"))
        .map(|(_, tail)| tail)
        .unwrap_or(b64)
        .trim();
    if body.is_empty() {
        return Err("bitmap payload is empty".to_string());
    }
    base64::engine::general_purpose::STANDARD
        .decode(body)
        .map_err(|e| format!("bitmap base64 decode failed: {e}"))
}

/// 只校验 PNG 签名与 IHDR，不做完整解码。
///
/// 处理后的图直接落盘给 `load_rgba` 用，这里只要能挡住"空/截断/不是 PNG"
/// 的脏数据即可；完整解码留给读的那一侧（省一次 4K 解码）。
fn validate_png_header(raw: &[u8]) -> Result<(), String> {
    const SIG: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    if raw.len() < 24 || raw[..8] != SIG {
        return Err("处理后的图像不是合法的 PNG".to_string());
    }
    if &raw[12..16] != b"IHDR" {
        return Err("处理后的 PNG 缺少 IHDR 块".to_string());
    }
    Ok(())
}

/// 载荷列表 → core-algo 图形列表。
fn to_shapes(
    payloads: &[ShapePayload],
    sf: f64,
) -> Result<Vec<snappin_core_algo::shapes::Shape>, String> {
    payloads.iter().map(|p| p.to_shape(sf)).collect()
}

// ---------- 截图主流程（命令 / 快捷键 / 托盘共用） ----------

/// 截图主流程（可在任意线程调用；内部处理线程亲和性）。
///
/// 顺序至关重要（曾踩坑）：**必须先采集、后显示任何本应用 UI**。
/// 早期版本"先开浮层再采集"，导致浮层的 loading 遮罩/文字被截进冻结图；
/// 悬浮球若未隐藏也会出现在截图里。
///
/// 现行流程：
/// 1. 隐藏悬浮球（主线程执行，等合成器刷新）
/// 2. 后台线程并行预创建**隐藏**浮层（与采集重叠，降低感知延迟）
/// 3. xcap 并行采集全部显示器 + fast 压缩
/// 4. 写入状态 → 主线程显示浮层（页面此时才渲染冻结图）
/// 采集互斥守卫：`Drop` 时释放标志，任何返回路径（含 ? 提前返回）都安全。
struct CaptureGuard<'a>(&'a std::sync::atomic::AtomicBool);

impl Drop for CaptureGuard<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

/// macOS：截图前的「屏幕录制」权限闸门。
///
/// 未授权时 macOS **不会报错**，而是照常返回一张图 —— 只是把其他应用的窗口
/// 全部过滤掉，于是用户看到的现象是"能进截图，但屏幕上所有窗口都不见了"，
/// 最终截出一张空桌面壁纸。这种失败没有任何错误码可依，只能在采集之前拦住。
///
/// 提示节流 5 秒：用户连按快捷键时不至于叠出一摞对话框。
///
/// **故意不用 `#[cfg(target_os = "macos")]` 整块裁掉**：那样这段只在 mac 上
/// 运行的代码在 Windows 开发机上永远不被编译器检查（API 改名、类型不匹配
/// 都发现不了，只能等 CI/mac 构建时炸）。这里改成运行时判定——非 macOS 时
/// `has_screen_capture_permission()` 恒为 true，第一行就返回，零副作用。
/// 清掉本应用的屏幕录制授权记录，并提示用户重启（macOS）。
///
/// 这是"设置里明明勾着、却始终 preflight=false"的**唯一可靠解法**：
/// 授权与代码签名绑定，adhoc 签名的 cdhash 每次构建都变，旧记录就成了
/// "看着有效、实际匹配不上"的僵尸条目；而系统因为有这条记录，**再也不会
/// 主动弹授权框**。`tccutil reset ScreenCapture <bundle-id>` 把记录删掉，
/// 下次启动才会重新走一遍系统授权流程。
pub(crate) fn reset_screen_permission_and_hint(app: &AppHandle) {
    use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind};

    let id = app.config().identifier.clone();
    let app_cb = app.clone();
    match snappin_core_platform::reset_screen_capture_permission(&id) {
        Ok(()) => {
            crate::logln!("[snappin] tccutil reset ScreenCapture {id} ok");
            app_cb
                .dialog()
                .message(format!(
                    "已清除「{id}」的屏幕录制授权记录。\n\n现在请：\n① 完全退出 SnapPin（菜单栏图标右键 → 退出 SnapPin）；\n② 重新打开 SnapPin —— 系统这次会弹出「SnapPin 想要录制屏幕」，点「打开系统设置」并打开开关；\n③ 系统提示需要重新打开应用时，再退出重开一次。\n\n如果系统框一直不弹，说明还有残留：再点一次本按钮，然后在「系统设置 → 隐私与安全性 → 屏幕录制」里手动点「−」删掉 SnapPin 条目后重启。"
                ))
                .title("已重置，请重启应用")
                .kind(MessageDialogKind::Info)
                .buttons(MessageDialogButtons::Ok)
                .show(|_| {});
        }
        Err(e) => {
            crate::logln!("[snappin] tccutil reset failed: {e}");
            app_cb
                .dialog()
                .message(format!(
                    "自动重置失败：{e}\n\n请在「终端」里手动执行这一行，然后重启 SnapPin：\n\ntccutil reset ScreenCapture {id}"
                ))
                .title("重置失败")
                .kind(MessageDialogKind::Error)
                .buttons(MessageDialogButtons::Ok)
                .show(|_| {});
        }
    }
}

/// 一次检查两项 macOS 权限，并给出"一个入口搞定"的修复对话框。
///
/// 为什么合并：原来菜单里是四条（屏幕录制权限 / 重置屏幕录制授权 /
/// 辅助功能权限 / 重置辅助功能授权），用户反馈"反复点很麻烦"。
/// 现在只留一条「权限检查…」：先告诉你哪项缺，再让你选"去设置"或"重置授权"。
pub(crate) fn check_and_fix_permissions(app: &AppHandle) {
    use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind};

    let scr = snappin_core_platform::has_screen_capture_permission();
    let acc = snappin_core_platform::has_accessibility_permission();
    crate::logln!("[snappin] perm check: 屏幕录制={scr} 辅助功能={acc}");

    let mark = |ok: bool| if ok { "✓ 已授权" } else { "✗ 未授权" };
    let msg = format!(
        "屏幕录制：{}\n（截图/OCR 用；缺了只能截到桌面壁纸）\n\n\
         辅助功能：{}\n\n\
         ⚠️ 每升级一个版本，adhoc 签名的 cdhash 都会变，旧授权会变成\
         \"勾着但无效\"的僵尸记录 —— 这时要用下面的「重置授权」清一遍再重新勾选。",
        mark(scr),
        mark(acc),
    );

    if scr && acc {
        app.dialog()
            .message(msg)
            .title("权限检查：全部就绪")
            .kind(MessageDialogKind::Info)
            .buttons(MessageDialogButtons::Ok)
            .show(|_| {});
        return;
    }

    let app_cb = app.clone();
    app.dialog()
        .message(msg)
        .title("权限检查")
        .kind(MessageDialogKind::Warning)
        .buttons(MessageDialogButtons::OkCancelCustom(
            "重置授权".to_string(),
            "去系统设置".to_string(),
        ))
        .show(move |reset| {
            if reset {
                crate::commands::reset_all_permissions(&app_cb);
            } else if !scr {
                crate::open_external(crate::SCREEN_RECORDING_SETTINGS_URL);
            } else {
                crate::open_external(crate::ACCESSIBILITY_SETTINGS_URL);
            }
        });
}

/// 一次性重置两项授权记录（屏幕录制 + 辅助功能），然后提示重新授权。
pub(crate) fn reset_all_permissions(app: &AppHandle) {
    use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind};
    let id = app.config().identifier.clone();
    let r1 = snappin_core_platform::reset_screen_capture_permission(&id);
    let r2 = snappin_core_platform::reset_accessibility_permission(&id);
    crate::logln!("[snappin] reset perms: screen={r1:?} access={r2:?}");

    let detail = match (&r1, &r2) {
        (Ok(()), Ok(())) => "两项授权记录都已清除。".to_string(),
        _ => format!(
            "部分清理失败（屏幕录制：{}；辅助功能：{}）。\n\
             可在「终端」手动执行：\n\n\
             tccutil reset ScreenCapture {id}\ntccutil reset Accessibility {id}",
            r1.as_ref().err().map(|s| s.as_str()).unwrap_or("ok"),
            r2.as_ref().err().map(|s| s.as_str()).unwrap_or("ok"),
        ),
    };

    let app_cb = app.clone();
    app_cb
        .dialog()
        .message(format!(
            "{detail}\n\n接下来：\n① 完全退出 SnapPin；\n② 重新打开 —— 系统会重新弹授权框，\
             按提示允许；\n③ 若「辅助功能」没有自动勾上，去系统设置手动勾选 SnapPin；\n\
             ④ 系统提示需要重启时，再退出重开一次。"
        ))
        .title("已重置权限")
        .kind(MessageDialogKind::Info)
        .buttons(MessageDialogButtons::Ok)
        .show(|_| {});
}

/// 打开「隐私与安全性 → 辅助功能」面板（macOS）。
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub(crate) fn open_accessibility_settings() {
    crate::open_external(crate::ACCESSIBILITY_SETTINGS_URL);
}

/// 长截图缺「辅助功能」权限时的准备动作（**不弹对话框、不调授权 FFI**）。
///
/// 为什么不再弹模态框：0.1.10 真机表现是"点长截图 → 弹一个说明框 → 用户点
/// 打开设置 → 关掉框"，多一次点击且关框后浮层仍停在"正在启动…"。现在改为
/// 直接把系统设置拉到前台，由浮层里的"等授权"卡片给出按钮与进度。
///
/// **为什么不再调 `request_accessibility_permission()`**：0.1.10 真机日志里
/// 两次"点长截图 → 闪退"都恰好死在这条日志（`long capture blocked`）之后，
/// 而紧随其后的第一个原生调用就是它。同机已证安全的有：`AXIsProcessTrusted()`
/// （5 次启动都正常）、`tccutil` 与 `open` 子进程（日志里有成功记录）。
/// 所以从热路径摘掉，只留"打开设置面板"这一条最朴素的动作，等 .ips 定位。
///
/// 每一步都打日志：万一还闪退，日志能精确停在哪个动作上。
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub(crate) fn prepare_accessibility_grant(_app: &AppHandle) {
    crate::logln!("[snappin] blocked: 1/2 打开辅助功能设置面板");
    crate::commands::open_accessibility_settings();
    crate::logln!("[snappin] blocked: 2/2 设置面板已请求（若列表里没有 SnapPin，需点 + 添加）");
}

/// 长截图"等授权"卡片用：辅助功能是否已授予（前端 ~800ms 轮询一次）。
///
/// macOS 的辅助功能授权是**即时生效**的（不像屏幕录制要重启进程），
/// 所以轮询到 true 就能立刻重跑长截图。
#[tauri::command]
pub fn accessibility_granted() -> bool {
    snappin_core_platform::has_accessibility_permission()
}

/// 打开「隐私与安全性 → 辅助功能」面板（等授权卡片上的按钮）。
#[tauri::command]
pub fn accessibility_settings_open() {
    crate::open_external(crate::ACCESSIBILITY_SETTINGS_URL);
}

/// 清掉本应用的「辅助功能」授权记录（等授权卡片上的「重置授权」按钮）。
///
/// 场景：adhoc 签名每版 cdhash 都变，升级后旧条目会变成"勾着但无效"的僵尸
/// 记录 —— 用户在系统设置里反复点开关都没用，`AXIsProcessTrusted()` 恒 false。
/// `tccutil reset Accessibility` 是唯一能清掉它的手段。
///
/// 清完只把设置面板拉起来（**不再调 prompt FFI**，见
/// `prepare_accessibility_grant` 的说明）：列表里重建条目要靠用户点「+」，
/// 卡片文案已经写明这一步。
#[tauri::command]
pub fn accessibility_reset(app: AppHandle) -> Result<(), String> {
    let id = app.config().identifier.clone();
    crate::logln!("[snappin] reset accessibility only: id={id}");
    snappin_core_platform::reset_accessibility_permission(&id)?;
    crate::logln!("[snappin] reset accessibility: done, opening pane");
    crate::open_external(crate::ACCESSIBILITY_SETTINGS_URL);
    Ok(())
}

fn mac_capture_gate(app: &AppHandle) -> Result<(), String> {
    use std::sync::atomic::{AtomicBool, Ordering};

    // 本进程是否已经请求过系统授权框（系统只弹一次，重复调用没有意义）
    static REQUESTED: AtomicBool = AtomicBool::new(false);

    if snappin_core_platform::has_screen_capture_permission() {
        return Ok(());
    }
    // 第一次没权限：先让系统自己弹授权框（用户在系统框里点"允许"后需重启本应用）
    if !REQUESTED.swap(true, Ordering::SeqCst)
        && snappin_core_platform::request_screen_capture_permission()
    {
        return Ok(());
    }

    // 走到这里说明 preflight 说没权限，但**它并不总是真话**。
    //
    // 2026-10-26 真机（macOS 13.6.6 + adhoc 签名）实测：preflight 恒为 false，
    // 而截图内容完全正常（能 OCR 出别的窗口里的文字）。说明这个 API 在
    // "签名变过/授权记录与当前二进制不完全匹配"时会误报。
    //
    // 因此门禁**从不阻断**：误报时阻断会让用户完全不可用且无自救路径。
    // 真没权限时截出来只有壁纸，肉眼立刻能分辨 —— 信息量比"打不开"更大。
    // 提示只弹一次（靠下面这个标记），而且只要用户成功截过一次就不再弹。
    if preflight_false_observed(app) {
        crate::logln!("[snappin] capture allowed (preflight=false，此前已确认能正常截图)");
        return Ok(());
    }

    crate::logln!("[snappin] capture allowed with preflight=false（仅提示，不阻断）");
    use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind};
    let app_for_cb = app.clone();
    app.dialog()
        .message(
            "系统报告 SnapPin 没有「屏幕录制」权限。\n\n             如果你截出来的图只有桌面壁纸、看不到任何窗口，就是真没权限：\n             ① 菜单栏图标右键 →「重置屏幕录制授权…」→ 按提示退出重启；\n             ② 系统弹出「SnapPin 想要录制屏幕」时点允许，再重启一次。\n\n             如果截图内容正常（能看到窗口），说明这只是系统的误报，忽略即可 ——              本次之后不会再提示。",
        )
        .title("屏幕录制权限检查")
        .kind(MessageDialogKind::Warning)
        .buttons(MessageDialogButtons::OkCancelCustom(
            "重置授权".to_string(),
            "继续截图".to_string(),
        ))
        .show(move |reset| {
            if reset {
                // dialog 的回调要求 'static：借用 &AppHandle 会逃逸，必须 clone
                crate::commands::reset_screen_permission_and_hint(&app_for_cb);
            }
        });
    Ok(())
}

/// "preflight 误报"的标记文件：只要用户在本机成功截过一次图，就写下它。
///
/// 目的是**只烦一次**。macOS 上 preflight 一旦误报就是每次启动都误报，
/// 若每次都弹提示，用户会被一个假警报反复打扰；而只弹一次又怕真有问题的
/// 用户看不到说明 —— 用"是否成功截过图"来区分这两类情况最省事。
#[cfg(target_os = "macos")]
fn preflight_marker(app: &AppHandle) -> Option<std::path::PathBuf> {
    app.path()
        .app_data_dir()
        .ok()
        .map(|d| d.join(".preflight-false-but-capture-works"))
}

#[cfg(not(target_os = "macos"))]
fn preflight_marker(_app: &AppHandle) -> Option<std::path::PathBuf> {
    None
}

fn preflight_false_observed(app: &AppHandle) -> bool {
    preflight_marker(app).map(|p| p.exists()).unwrap_or(false)
}

/// 成功截到图后调用：如果 preflight 仍在误报，落一个标记，之后不再提示。
pub(crate) fn note_capture_succeeded(app: &AppHandle) {
    #[cfg(target_os = "macos")]
    {
        if snappin_core_platform::has_screen_capture_permission() {
            return;
        }
        if let Some(p) = preflight_marker(app) {
            if let Some(dir) = p.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            let _ = std::fs::write(&p, b"1");
        }
    }
    #[cfg(not(target_os = "macos"))]
    let _ = app;
}

pub fn do_capture(app: &AppHandle, state: &AppState, src: &str) -> Result<CapturePayload, String> {
    // 权限闸必须在互斥锁与采集之前：否则会先截出一张"没有窗口的桌面"，
    // 用户完全看不出这是权限问题（见 mac_capture_gate 文档）。
    mac_capture_gate(app)?;
    let t0 = std::time::Instant::now();
    // 0. 互斥：热键/球点击/前端命令可能在同一瞬间重复触发。没有这道闸，
    // 第二遍采集会复用第一遍预创建的浮层（页面不重载→前端状态陈旧），
    // 并把浮层自身截进冻结图，表现为"整屏发灰 + 截图采集超时"。
    if state
        .capturing
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        eprintln!("[snappin] capture skipped (src={src}, session={} busy)", state.session.load(Ordering::SeqCst));
        return Err("capture already in progress".to_string());
    }
    let _guard = CaptureGuard(&state.capturing);
    // 新会话代号：上一轮迟到的 cancel/confirm 请求据此判定为陈旧并忽略
    let session_id = state.session.fetch_add(1, Ordering::SeqCst) + 1;
    state.cancel_pending.store(false, Ordering::SeqCst);
    eprintln!("[snappin] capture start (session {session_id}, src={src})");

    // 1. 采集前隐藏悬浮球（否则球体会被截进冻结图），并等待合成器刷新
    let ball_hidden = windows::hide_ball_for_capture(app);
    if ball_hidden {
        // 300ms 不是保守值，是实测下限：Windows 对 hide() 会走一个约 200ms 的
        // 淡出动画（"动画显示窗口"开启时）。只等 100ms 抓到的帧里球还在，
        // 表现为冻结图上有一块很淡的圆角方块残影（用户报"截图会把图标也截出来"）。
        std::thread::sleep(Duration::from_millis(300));
    }
    let after_ball = std::time::Instant::now();

    // 2. 并行预创建隐藏浮层（与采集重叠；失败无妨，第 4 步会兜底创建）
    let precreate_app = app.clone();
    let precreate = std::thread::spawn(move || {
        windows::precreate_overlays_hidden(&precreate_app);
    });

    // 3. 采集（并行 + fast 压缩，双屏约 0.9s）
    let capture_start = std::time::Instant::now();
    let capture = snappin_core_platform::capture_all_monitors();
    let capture_ms = capture_start.elapsed().as_millis();

    // 等预创建线程结束（采集已完成或接近完成）
    let join_start = std::time::Instant::now();
    let _ = precreate.join();
    let join_ms = join_start.elapsed().as_millis();

    let shots = match capture {
        Ok(s) => s,
        Err(e) => {
            // 采集失败：关掉隐藏浮层、清空会话、恢复悬浮球，返回错误
            eprintln!("[snappin] xcap capture error (session {session_id}): {e}");
            windows::end_capture_session(app, state, "xcap-error");
            return Err(e.to_string());
        }
    };
    eprintln!(
        "[snappin] captured {} monitor(s) in {}ms (session {session_id})",
        shots.len(),
        t0.elapsed().as_millis()
    );
    // 成功截到图 = 权限实际可用。若 preflight 仍在误报，落一个标记文件，
    // 之后启动就不再弹那条"没有屏幕录制权限"的提示（详见 mac_capture_gate）。
    crate::commands::note_capture_succeeded(app);
    // 采集期间用户已按取消（前端此时还没有会话代号）：在显示浮层前放弃，
    // 避免"Escl 了却又弹出浮层"。
    if state.cancel_pending.swap(false, Ordering::SeqCst) {
        windows::end_capture_session(app, state, "cancel-pending");
        eprintln!("[snappin] capture canceled before show (session {session_id})");
        return Err("capture canceled".to_string());
    }

    {
        let mut guard = state.monitors.lock().unwrap();
        *guard = shots.clone();
    }

    // 4. 显示浮层（主线程；几何用 xcap 精确值校正）
    let show_start = std::time::Instant::now();
    windows::show_overlays(app, &shots);
    crate::logln!(
        "[perf] capture session={session_id} ball={}ms capture={}ms overlay_wait={}ms overlay_show={}ms total={}ms",
        after_ball.duration_since(t0).as_millis(),
        capture_ms,
        join_ms,
        show_start.elapsed().as_millis(),
        t0.elapsed().as_millis()
    );
    eprintln!(
        "[snappin] session {session_id} ready, {}ms total",
        t0.elapsed().as_millis()
    );

    Ok(build_capture_payload(&shots))
}

#[tauri::command]
pub async fn start_capture(app: AppHandle) -> Result<CapturePayload, String> {
    // Windows 已知问题：同步命令/主线程内创建 webview 会死锁或导致 webview 空白，
    // 必须放到阻塞线程执行（见 WebviewWindowBuilder 文档的 Known issues）。
    let app2 = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let state = app2.state::<AppState>();
        do_capture(&app2, &state, "cmd")
    })
    .await
    .map_err(|e| format!("capture task failed: {e}"))?
}

#[tauri::command]
pub fn overlay_context(
    window: Window,
    state: State<'_, AppState>,
) -> Result<OverlayContextPayload, String> {
    let idx: usize = window
        .label()
        .strip_prefix(windows::OVERLAY_PREFIX)
        .ok_or_else(|| "not an overlay window".to_string())?
        .parse()
        .map_err(|_| "invalid overlay window label".to_string())?;
    let monitors = state.monitors.lock().unwrap();
    // 采集中（state.monitors 为空或尚未覆盖到本显示器）：
    // 返回 ready=false 的占位载荷，前端轮询等待，不视为错误。
    let Some(m) = monitors.get(idx) else {
        // 诊断：越界说明"浮层索引 > 采集结果数"，该浮层会永远等不到 ready。
        // 轮询很频繁，用静态计数限流，只打前几条。
        static WAIT_LOGS: AtomicUsize = AtomicUsize::new(0);
        let n = WAIT_LOGS.fetch_add(1, Ordering::Relaxed);
        if n < 3 {
            eprintln!(
                "[snappin] overlay {} waits: idx={} captured={}",
                window.label(),
                idx,
                monitors.len()
            );
        }
        return Ok(OverlayContextPayload {
            monitor_id: String::new(),
            image_path: String::new(),
            natural_width: 0,
            natural_height: 0,
            scale_factor: 1.0,
            ready: false,
            session_id: state.session.load(Ordering::SeqCst),
        });
    };
    Ok(OverlayContextPayload {
        monitor_id: m.id.clone(),
        image_path: m.path.to_string_lossy().to_string(),
        natural_width: m.width,
        natural_height: m.height,
        scale_factor: m.scale_factor,
        ready: true,
        session_id: state.session.load(Ordering::SeqCst),
    })
}

#[tauri::command]
pub async fn confirm_selection(
    app: AppHandle,
    state: State<'_, AppState>,
    monitor_id: String,
    rect: RectPayload,
    action: String,
    shapes: Option<Vec<ShapePayload>>,
    session_id: Option<u64>,
) -> Result<ConfirmPayload, String> {
    // 陈旧确认（上一轮会话迟到的请求）：直接丢弃，避免误操作新会话
    if let Some(id) = session_id {
        let current = state.session.load(Ordering::SeqCst);
        if id != current {
            return Err(format!("stale session {id} != {current}"));
        }
    }
    // await 之前取好配置，避免跨 await 持有 State
    let save_dir = { state.config.lock().unwrap().save_dir.clone() };

    let (cropped, scale_factor, monitor_x, monitor_y) = {
        let monitors = state.monitors.lock().unwrap();
        let m = monitors
            .iter()
            .find(|m| m.id == monitor_id)
            .ok_or_else(|| "no active capture session".to_string())?;
        let sf = m.scale_factor as f64;
        // 前端选区为逻辑像素，换算成物理像素后再裁剪
        let algo_rect = AlgoRect {
            x: (rect.x * sf).round() as i32,
            y: (rect.y * sf).round() as i32,
            w: (rect.w * sf).round().max(1.0) as u32,
            h: (rect.h * sf).round().max(1.0) as u32,
        };
        let mut cropped = m.image.crop(&algo_rect).map_err(|e| e.to_string())?;
        // 标注光栅化到裁剪图上（复制/贴图/保存三个出口共用同一份像素，
        // 保证所见即所得）。标注坐标同为逻辑像素，这里一起换算。
        if let Some(payloads) = &shapes {
            if !payloads.is_empty() {
                // base = 画任何标注之前的原始裁剪图。橡皮擦（还原原图）、
                // 放大镜（放大原图内容）、智能擦除（从原图边界扩散）三个
                // 变体都要从它取像素；普通图形不受影响。
                let base = cropped.clone();
                snappin_core_algo::draw::draw_shapes_with_base(
                    &mut cropped,
                    &base,
                    &to_shapes(payloads, sf)?,
                );
            }
        }
        let (ox, oy) = m.logical_origin();
        (cropped, sf, ox, oy)
    };

    let path = snappin_core_platform::new_temp_png_path();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    snappin_core_platform::save_png(&cropped, &path).map_err(|e| e.to_string())?;
    let path_str = path.to_string_lossy().to_string();

    match action.as_str() {
        "copy" => {
            snappin_core_platform::clipboard_copy_image(&cropped).map_err(|e| e.to_string())?;
        }
        "pin" => {
            let label = pin_label(&path);
            let disp_w = (cropped.width as f64 / scale_factor).max(8.0);
            let disp_h = (cropped.height as f64 / scale_factor).max(8.0);
            let x = monitor_x + rect.x;
            let y = monitor_y + rect.y;
            // 建窗线程约束因平台而异（见 windows::create_pin_window 文档）：
            // Windows 主线程同步建窗会死锁 → 丢到阻塞线程；macOS 必须主线程 → 内部投递。
            let app2 = app.clone();
            let label2 = label.clone();
            #[cfg(windows)]
            tauri::async_runtime::spawn_blocking(move || {
                windows::create_pin_window(&app2, &label2, disp_w, disp_h, x, y)
            })
            .await
            .map_err(|e| format!("pin window task failed: {e}"))??;
            #[cfg(not(windows))]
            windows::create_pin_window(&app2, &label2, disp_w, disp_h, x, y)?;
            register_pin(&state, &label, &path_str, disp_w, disp_h);
        }
        "save" => {
            // 先结束会话再弹对话框：① always-on-top 全屏浮层会盖住系统保存
            // 对话框（无主窗口，z-order 低于置顶浮层）；② 提前 return，
            // 避免对话框停留期间用户开启新截图后，被函数尾部的清理误杀
            // （防重入只看浮层是否存在，此时浮层已关）。
            windows::end_capture_session(&app, &state, "save-before-dialog");
            let dest = save_dialog(
                &app,
                &snappin_core_platform::default_save_name(),
                save_dir,
            )
            .await;
            if let Some(dest) = dest {
                // 临时文件固定为 PNG；按用户选择的扩展名重新编码落盘
                let buffer = snappin_core_platform::load_rgba(Path::new(&path_str))
                    .map_err(|e| e.to_string())?;
                snappin_core_platform::save_image(&buffer, &dest).map_err(|e| e.to_string())?;
                // 记住此次目录，下次保存对话框默认打开这里
                if let Some(parent) = dest.parent() {
                    let dir_str = parent.to_string_lossy().to_string();
                    let mut guard = state.config.lock().unwrap();
                    if guard.save_dir.as_deref() != Some(dir_str.as_str()) {
                        guard.save_dir = Some(dir_str.clone());
                        let snapshot = guard.clone();
                        drop(guard);
                        if let Ok(dir) = app.path().app_config_dir() {
                            let _ = crate::config::save(&dir, &snapshot);
                        }
                    }
                }
            }
            return Ok(ConfirmPayload { path: path_str });
        }
        other => return Err(format!("unknown action: {}", other)),
    }

    // 复制/贴图成功：结束会话（关闭全部浮层 + 清空数据）。
    // 前端每块浮层还会自关 + 轮询兜底，三级保障"完全退出截图模式"。
    windows::end_capture_session(&app, &state, "confirm-ok");
    Ok(ConfirmPayload { path: path_str })
}

#[tauri::command]
pub fn cancel_capture(
    app: AppHandle,
    state: State<'_, AppState>,
    session_id: Option<u64>,
) -> Result<(), String> {
    let current = state.session.load(Ordering::SeqCst);
    eprintln!(
        "[snappin] cancel_capture requested (arg={:?}, current={current}, capturing={})",
        session_id,
        state.capturing.load(Ordering::SeqCst)
    );
    if let Some(id) = session_id {
        if id != current {
            // 上一轮会话的迟到取消：新会话已经在进行，忽略它，
            // 否则会关掉新浮层并清空新采集数据（表现为采集超时）。
            eprintln!("[snappin] cancel ignored: stale session {id} != {current}");
            return Ok(());
        }
    } else if state.capturing.load(Ordering::SeqCst) {
        // 采集还没完成，前端尚无会话代号：登记取消，由 do_capture 在
        // 显示浮层之前消费（此刻直接关窗会与预创建流程冲突）。
        state.cancel_pending.store(true, Ordering::SeqCst);
        return Ok(());
    }
    windows::end_capture_session(&app, &state, "cancel-capture");
    Ok(())
}

/// 前端诊断日志：release 构建里 DevTools 控制台不可用，
/// 统一汇到 stderr，便于与 Rust 侧日志按同一时间线排查。
#[tauri::command]
pub fn ui_log(window: Window, msg: String) {
    eprintln!("[ui:{}] {msg}", window.label());
}

/// 复制纯文本到剪贴板（取色器的 HEX、二维码识别结果都走这里）。
///
/// 不用前端的 `navigator.clipboard`：WebView2 里写入剪贴板需要文档焦点与
/// 用户手势校验，而截图工具的场景常常是"窗口刚抢走焦点"（点完工具条
/// 立刻要写剪贴板），浏览器侧容易静默失败。arboard 走系统 API 最稳。
#[tauri::command]
pub fn copy_text(text: String) -> Result<(), String> {
    snappin_core_platform::clipboard_copy_text(&text).map_err(|e| e.to_string())
}

/// 用系统默认浏览器打开链接（二维码识别结果的"打开链接"按钮）。
///
/// **只放行 http/https**：二维码内容是用户不可控的外部数据，绝不能把
/// `file:`/`ms-msapp:`/任意协议喂给系统的"打开"处理器——一个 `file:` 链接
/// 就能读到本地文件，属于把工具变成攻击面。
#[tauri::command]
pub fn open_url(url: String) -> Result<(), String> {
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return Err("only http/https links can be opened".to_string());
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // 不弹黑色控制台窗口：start 是 cmd 内建命令，借道 cmd 必然带窗口，
        // 用 CREATE_NO_WINDOW 压掉（0x08000000）。
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        std::process::Command::new("cmd")
            .arg("/c")
            .arg("start")
            .arg("")
            .arg(&url)
            .creation_flags(CREATE_NO_WINDOW)
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(&url)
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    #[cfg(all(not(windows), not(target_os = "macos")))]
    {
        std::process::Command::new("xdg-open")
            .arg(&url)
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

// ---------- 二维码 / 条码识别 ----------

/// 一个识别到的码。
///
/// 坐标约定：定位点相对**选区左上角**、单位为**逻辑像素**——与前端 overlay
/// 里选区对象的坐标系一致，前端拿到即可直接画高亮框，不需要再换算。
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DecodedCodePayload {
    pub text: String,
    /// `QR_CODE` / `EAN_13` / `DATA_MATRIX` …
    pub format: String,
    pub points: Vec<(f32, f32)>,
}

/// 识别选区内的二维码 / 条码（支持一图多码）。
///
/// 走 `spawn_blocking`：rxing 的多码识别最坏情况要做象限递归，
/// 在 4K 全屏上是百毫秒到秒级，不能占住主线程。
///
/// 没有码时返回空数组（不是错误）。
#[tauri::command]
pub async fn decode_qr(
    app: AppHandle,
    monitor_id: String,
    rect: RectPayload,
    session_id: Option<u64>,
) -> Result<Vec<DecodedCodePayload>, String> {
    if let Some(id) = session_id {
        let current = app.state::<AppState>().session.load(Ordering::SeqCst);
        if id != current {
            return Err(format!("stale session {id} != {current}"));
        }
    }
    let app2 = app.clone();
    let monitor_id2 = monitor_id.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let state = app2.state::<AppState>();
        let monitors = state.monitors.lock().unwrap();
        let m = monitors
            .iter()
            .find(|m| m.id == monitor_id2)
            .ok_or_else(|| "no active capture session".to_string())?;
        let sf = m.scale_factor as f64;
        // 与 confirm_selection 完全相同的裁剪口径：逻辑像素 → 物理像素
        let algo_rect = AlgoRect {
            x: (rect.x * sf).round() as i32,
            y: (rect.y * sf).round() as i32,
            w: (rect.w * sf).round().max(1.0) as u32,
            h: (rect.h * sf).round().max(1.0) as u32,
        };
        let cropped = m.image.crop(&algo_rect).map_err(|e| e.to_string())?;
        let codes = snappin_core_platform::qr::decode_codes(&cropped)
            .map_err(|e| e.to_string())?;
        // 在锁内一起换算：定位点从物理像素换成逻辑像素，与前端选区坐标系对齐
        Ok(to_decoded_payloads(codes, sf))
    })
    .await
    .map_err(|e| format!("decode task failed: {e}"))?
}

/// 识别结果转前端载荷：定位点从物理像素换算成逻辑像素。
///
/// 单独抽成函数是为了可测——换算错一点，前端的高亮框就会整体偏移，
/// 而这种偏移在"恰好 1.0 缩放"的显示器上看不出来。
fn to_decoded_payloads(
    codes: Vec<snappin_core_platform::qr::DecodedCode>,
    sf: f64,
) -> Vec<DecodedCodePayload> {
    codes
        .into_iter()
        .map(|c| DecodedCodePayload {
            text: c.text,
            format: c.format,
            points: c
                .points
                .into_iter()
                .map(|(x, y)| ((x as f64 / sf) as f32, (y as f64 / sf) as f32))
                .collect(),
        })
        .collect()
}

// ---------- 文字识别（OCR） ----------

/// 一行识别结果（前端契约）。
///
/// 坐标约定：框相对**选区左上角**、单位**逻辑像素**——与二维码定位点同一
/// 口径，前端拿到即可直接画高亮框。
#[derive(Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct OcrLinePayload {
    pub text: String,
    /// 识别置信度 0..1
    pub score: f32,
    pub min_x: f32,
    pub min_y: f32,
    pub max_x: f32,
    pub max_y: f32,
}

/// OCR 模型状态。前端据此把工具按钮标成"需下载模型"，
/// 而不是等用户点下去才弹推理错误。
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OcrModelsPayload {
    pub available: bool,
    /// 模型目录（缺失时告诉用户该往哪放）
    pub dir: String,
}

/// 下载 OCR 模型三件套到固定目录（首次用 OCR 时按需触发）。
///
/// 15.6MB，走阻塞线程，避免卡住主线程/UI。已存在且 SHA256 校验通过的
/// 文件会跳过，所以重复调用是幂等的（点到一半断网也能续上）。
#[tauri::command]
pub async fn ocr_download_models(app: AppHandle) -> Result<OcrModelsPayload, String> {
    let dir = resolve_models_dir(&app);
    let dir_bg = dir.clone();
    tauri::async_runtime::spawn_blocking(move || {
        snappin_core_platform::ocr::download_models(&dir_bg, |line| crate::logln!("{line}"))
    })
    .await
    .map_err(|e| format!("模型下载任务失败：{e}"))??;
    Ok(OcrModelsPayload {
        available: snappin_core_platform::ocr::models_present(&dir),
        dir: dir.to_string_lossy().to_string(),
    })
}

/// 解析 OCR 模型目录。优先级：
///
/// 1. `SNAPPIN_MODELS_DIR` 环境变量（显式覆盖，测试/特殊部署用）；
/// 2. debug 构建：仓库根 `models/ocr`（开发期免配置，文件在就走这里）；
/// 3. release：`{app_data_dir}/models/ocr`——按需下载的固定目录，
///    与"模型不进安装包"的约定一致（见 models/README.md）。
fn resolve_models_dir(app: &AppHandle) -> PathBuf {
    if let Some(dir) = std::env::var_os("SNAPPIN_MODELS_DIR") {
        return PathBuf::from(dir);
    }
    // 1) **随安装包分发的模型**（0.1.7 起）：装完即可用 OCR，无需联网。
    //    优先级最高——包里有就不用下载，避免用户卡在"模型缺失"上。
    //    macOS 落在 SnapPin.app/Contents/Resources/models/ocr，
    //    Windows 落在安装目录的 models\ocr（见 tauri.conf.json 的 bundle.resources）。
    if let Ok(res) = app.path().resource_dir() {
        let bundled = res.join("models").join("ocr");
        if snappin_core_platform::ocr::models_present(&bundled) {
            return bundled;
        }
    }
    #[cfg(debug_assertions)]
    {
        let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("models/ocr")
            .canonicalize()
            .unwrap_or_default();
        if snappin_core_platform::ocr::models_present(&repo) {
            return repo;
        }
    }
    match app.path().app_data_dir() {
        Ok(base) => base.join("models").join("ocr"),
        Err(_) => PathBuf::from("models/ocr"),
    }
}

/// 查询 OCR 模型是否就绪（不触发加载）。
#[tauri::command]
pub fn ocr_models_status(app: AppHandle) -> OcrModelsPayload {
    let dir = resolve_models_dir(&app);
    OcrModelsPayload {
        available: snappin_core_platform::ocr::models_present(&dir),
        dir: dir.to_string_lossy().to_string(),
    }
}

/// 识别选区内的文字（M4 OCR：PP-OCRv4 mobile + rten）。
///
/// 走 `spawn_blocking`：det 推理在全高清区域约 0.8s、4K 约 3s，
/// 不能占住主线程。模型首次调用时懒加载并缓存（约 60ms），
/// 之后每次识别只付推理的钱。
///
/// 模型缺失时返回可操作的错误信息（告诉用户目录在哪），不是panic。
#[tauri::command]
pub async fn ocr_recognize(
    app: AppHandle,
    monitor_id: String,
    rect: RectPayload,
    session_id: Option<u64>,
) -> Result<Vec<OcrLinePayload>, String> {
    if let Some(id) = session_id {
        let current = app.state::<AppState>().session.load(Ordering::SeqCst);
        if id != current {
            return Err(format!("stale session {id} != {current}"));
        }
    }
    let app2 = app.clone();
    let monitor_id2 = monitor_id.clone();
    let t0 = std::time::Instant::now();
    crate::logln!("ocr: start monitor={monitor_id2} rect={:?}", (rect.x, rect.y, rect.w, rect.h));
    tauri::async_runtime::spawn_blocking(move || {
        let state = app2.state::<AppState>();
        // **锁只包住裁剪**（毫秒级）就地释放。
        // 曾经的 bug：推理（秒级）期间一直攥着 monitors 锁，而 cancel_capture
        // 是同步命令、跑在主线程上，也要拿这把锁 → 主线程被锁死 → 用户按 Esc /
        // 点关闭全都没反应（表现为"截图模式卡死"）。
        let (cropped, sf, ox, oy) = {
            let monitors = state.monitors.lock().unwrap();
            let m = monitors
                .iter()
                .find(|m| m.id == monitor_id2)
                .ok_or_else(|| "no active capture session".to_string())?;
            let sf = m.scale_factor as f64;
            // 与 confirm_selection / decode_qr 完全相同的裁剪口径
            let algo_rect = AlgoRect {
                x: (rect.x * sf).round() as i32,
                y: (rect.y * sf).round() as i32,
                w: (rect.w * sf).round().max(1.0) as u32,
                h: (rect.h * sf).round().max(1.0) as u32,
            };
            let cropped = m.image.crop(&algo_rect).map_err(|e| e.to_string())?;
            (cropped, sf, algo_rect.x, algo_rect.y)
        };
        crate::logln!("ocr: cropped {}x{} in {}ms", cropped.width, cropped.height, t0.elapsed().as_millis());
        // 裁剪图落盘：结果窗左栏要显示"原图"（对标参考图的左图右文布局）。
        // 存路径不存字节——前端走 asset 协议读，避免大 base64 过 IPC。
        //
        // 先把旧的清空再写：万一这次落盘失败，宁可左栏显示"没有原图"，
        // 也不能留着**上一次**的图配**这一次**的识别文字（那样的错配
        // 比空着更难发现）。
        {
            *state.ocr_image.lock().unwrap() = None;
            let path = snappin_core_platform::new_temp_png_path();
            match snappin_core_platform::save_png(&cropped, &path) {
                Ok(()) => {
                    *state.ocr_image.lock().unwrap() = Some(path.display().to_string());
                }
                Err(e) => crate::logln!("ocr: save cropped image failed: {e}"),
            }
        }

        // 引擎懒加载：持锁加载，加载期间其它 OCR 请求排队等待
        let mut guard = state.ocr.lock().unwrap();
        if guard.is_none() {
            let dir = resolve_models_dir(&app2);
            if !snappin_core_platform::ocr::models_present(&dir) {
                return Err(format!(
                    "OCR 模型缺失：请把 det.onnx / rec.onnx / ppocr_keys_v1.txt \
                     下载到 {}（来源与校验值见仓库 models/README.md）",
                    dir.display()
                ));
            }
            match snappin_core_platform::ocr::OcrEngine::load(&dir) {
                Ok(engine) => *guard = Some(engine),
                Err(e) => return Err(format!("OCR 模型加载失败：{e}")),
            }
        }
        let engine = guard.as_ref().expect("engine just loaded");
        let lines = engine.recognize(&cropped).map_err(|e| e.to_string())?;
        // 推理期间用户可能已取消截图：会话号变了就别再返回结果，
        // 免得"取消了还弹一个结果窗出来"。
        if let Some(id) = session_id {
            let current = state.session.load(Ordering::SeqCst);
            if id != current {
                return Err(format!("stale session {id} != {current}"));
            }
        }
        // 检测框从裁剪图物理像素换成选区内逻辑像素，与前端标注层坐标系对齐
        let payloads = to_ocr_payloads(lines, sf, ox, oy);
        crate::logln!("ocr: done {} lines in {}ms", payloads.len(), t0.elapsed().as_millis());
        Ok(payloads)
    })
    .await
    .map_err(|e| format!("ocr task failed: {e}"))?
}

/// 识别结果转前端载荷：框从裁剪图物理像素换算成**选区内逻辑像素**。
///
/// 单独抽成函数是为了可测——换算错一点，前端的高亮框就会整体偏移，
/// 而这种偏移在"恰好 1.0 缩放"的显示器上看不出来。
fn to_ocr_payloads(
    lines: Vec<snappin_core_platform::ocr::OcrLine>,
    sf: f64,
    origin_x: i32,
    origin_y: i32,
) -> Vec<OcrLinePayload> {
    lines
        .into_iter()
        .map(|l| OcrLinePayload {
            text: l.text,
            score: l.score,
            min_x: to_logical(l.box_.min_x, origin_x, sf),
            min_y: to_logical(l.box_.min_y, origin_y, sf),
            max_x: to_logical(l.box_.max_x, origin_x, sf),
            max_y: to_logical(l.box_.max_y, origin_y, sf),
        })
        .collect()
}

/// 物理像素 → 选区内逻辑像素（相对选区左上角）。
fn to_logical(v: f32, origin: i32, sf: f64) -> f32 {
    ((v - origin as f32) / sf as f32).max(0.0)
}

// ---------- 贴图 ----------

/// 剪贴板贴图的实际逻辑（供命令与托盘/快捷键路径共用）。
/// 贴图在主屏居中（原"Ctrl+V 跟随鼠标"的定位分支已随该功能一并移除）。
/// 内部会创建贴图 webview 窗口，调用方须确保不在主线程同步执行。
pub fn do_pin_from_clipboard(app: &AppHandle, state: &AppState) -> Result<String, String> {
    let buffer = snappin_core_platform::clipboard_get_image()
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "剪贴板中没有图片".to_string())?;
    let path = snappin_core_platform::new_temp_png_path();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    snappin_core_platform::save_png_fast(&buffer, &path).map_err(|e| e.to_string())?;
    let path_str = path.to_string_lossy().to_string();
    let label = pin_label(&path);

    // 定位：主屏居中（按主显示器缩放因子把物理像素换算成逻辑像素）
    let (disp_w, disp_h, x, y) = {
        let sf = app
            .primary_monitor()
            .ok()
            .flatten()
            .map(|m| m.scale_factor())
            .unwrap_or(1.0);
        let dw = (buffer.width as f64 / sf).max(8.0);
        let dh = (buffer.height as f64 / sf).max(8.0);
        let (cx, cy) = windows::place_centered_on_primary(app, dw, dh);
        (dw, dh, cx, cy)
    };
    windows::create_pin_window(app, &label, disp_w, disp_h, x, y)?;
    register_pin(state, &label, &path_str, disp_w, disp_h);
    Ok(label)
}

#[tauri::command]
pub async fn pin_from_clipboard(app: AppHandle) -> Result<String, String> {
    let app2 = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let state = app2.state::<AppState>();
        do_pin_from_clipboard(&app2, &state)
    })
    .await
    .map_err(|e| format!("pin task failed: {e}"))?
}

/// 读取图片原始字节（PNG），以**二进制**经 IPC 返回（JS 侧拿到 ArrayBuffer）。
///
/// 用途：截图浮层要把冻结图变成 ImageBitmap 才能**读像素**（取色/放大镜
/// 颜色读数）。`convertFileSrc` 的 asset 图片在 WebView2 里被当作跨域资源，
/// `getImageData` 会抛 SecurityError；fetch 是否放行取决于响应头是否被
/// 正确识别。走 Rust 读文件则完全绕开 CORS，行为确定。
///
/// 只在会话开始时读一次（约 1-3MB，一次性开销），不做逐像素 IPC。
#[tauri::command]
pub fn read_image_bytes(path: String) -> Result<tauri::ipc::Response, String> {
    let bytes = std::fs::read(&path).map_err(|e| format!("read image failed: {e}"))?;
    Ok(tauri::ipc::Response::new(bytes))
}

/// 贴图窗口取回自己的上下文（按窗口标签查状态表）。
#[tauri::command]
pub fn pin_context(
    window: Window,
    state: State<'_, AppState>,
) -> Result<PinContextPayload, String> {
    let label = window.label().to_string();
    let pins = state.pins.lock().unwrap();
    let meta = pins
        .get(&label)
        .ok_or_else(|| "pin metadata not found".to_string())?;
    Ok(PinContextPayload {
        label,
        src: meta.src.clone(),
        width: meta.width,
        height: meta.height,
    })
}

#[tauri::command]
pub fn pin_close(app: AppHandle, state: State<'_, AppState>, label: String) -> Result<(), String> {
    windows::close_pin(&app, &label);
    let meta = state.pins.lock().unwrap().remove(&label);
    // 顺手清掉菜单勾选快照，避免 label 复用后读到旧状态
    state.pin_flags.lock().unwrap().remove(&label);
    // 图像处理产生的临时文件：窗口没了就没人读它了，删掉免得临时目录越积越多。
    // 原图（edited = None）不归这里管——它是截图/剪贴板自己的文件。
    if let Some(meta) = meta {
        if let Some(edited) = meta.edited {
            let _ = std::fs::remove_file(edited);
        }
    }
    Ok(())
}

/// 切换贴图窗的置顶状态（工具条"置顶"按钮）。
/// always_on_top 的 setter 须在主线程执行（PostMessage 窗口句柄）。
#[tauri::command]
pub async fn pin_set_topmost(
    app: AppHandle,
    state: State<'_, AppState>,
    label: String,
    topmost: bool,
) -> Result<(), String> {
    // 仅允许操作已登记的贴图窗（防止恶意 label 操作其他窗口）
    if !state.pins.lock().unwrap().contains_key(&label) {
        return Err("pin window not found".to_string());
    }
    windows::set_window_topmost(&app, &label, topmost);
    Ok(())
}

/// 切换贴图窗的鼠标穿透（工具条"穿透"按钮）。
///
/// 穿透 = 窗口对鼠标事件透明（SetWindowLong WS_EX_TRANSPARENT），
/// 用于临时查看/操作被贴图遮住的内容。穿透后本窗工具条也点不到，
/// 故前端必须提供自动恢复（定时）与提示；恢复方式见 PinWindow 注释。
#[tauri::command]
pub async fn pin_set_ignore_cursor_events(
    app: AppHandle,
    state: State<'_, AppState>,
    label: String,
    ignore: bool,
) -> Result<(), String> {
    if !state.pins.lock().unwrap().contains_key(&label) {
        return Err("pin window not found".to_string());
    }
    windows::set_window_ignore_cursor_events(&app, &label, ignore);
    Ok(())
}

#[tauri::command]
pub fn pin_copy(app: AppHandle, state: State<'_, AppState>, label: String) -> Result<(), String> {
    let src = pin_src(&state, &label)?;
    let buffer = snappin_core_platform::load_rgba(Path::new(&src)).map_err(|e| e.to_string())?;
    snappin_core_platform::clipboard_copy_image(&buffer).map_err(|e| e.to_string())?;
    // 关闭路径上的窗口不存在也没关系
    let _ = app;
    Ok(())
}

/// 应用一次图像处理（旋转 / 翻转 / 灰度 / 反色 / 透明度 / 亮度）。
///
/// 分工：**像素处理在前端 canvas 完成**，这里只做"落盘 + 换源 + 调窗口"。
/// 理由有两条：
/// 1. 这些操作在 canvas 上是 `filter` 一行的事，塞进 core 层只会让核心算法
///    依赖编解码与图形 API（`core-algo` 保持零依赖的约定）；
/// 2. `pin_copy` / `pin_save` 读的正是本命令更新后的 `src`，因此**贴图所见
///    即所得**——不需要再给复制/保存各实现一套处理。
///
/// 参数 `width/height` 是处理后图片的**逻辑**显示尺寸（旋转 90° 时宽高互换），
/// 窗口按它重设大小并保持中心不动。返回新的上下文，前端据此更新 `<img>`。
///
/// async 原因：内部要调 `resize_pin_keep_center`，它会 `run_on_main_thread`
/// 投递闭包并**等**完成；同步命令本身就在主线程跑，会自等待死锁
/// （同 update_config / ball_menu 的注释）。
/// 贴图默认分组名
pub const DEFAULT_PIN_GROUP: &str = "默认";

/// 可选分组（右键菜单里切换归属）
pub const PIN_GROUPS: [&str; 4] = ["默认", "工作", "收藏", "重要"];

// ---- 分组的核心逻辑（自由函数）----
// 为什么不用 `State<'_, AppState>` 参数：`menu_action` 里也要调它们，
// 而 Tauri 的 `State` 只能在命令签名里出现、无法手工构造。所以这里拆成
// 自由函数，命令壳只负责转发。

fn pin_groups_impl(state: &AppState) -> Vec<String> {
    let guard = state.pins.lock().unwrap();
    let mut out: Vec<String> = PIN_GROUPS.iter().map(|g| g.to_string()).collect();
    for m in guard.values() {
        if !m.group.is_empty() && !out.contains(&m.group) {
            out.push(m.group.clone());
        }
    }
    out
}

fn pin_group_of_impl(state: &AppState, label: &str) -> String {
    state
        .pins
        .lock()
        .unwrap()
        .get(label)
        .map(|m| m.group.clone())
        .unwrap_or_default()
}

fn pin_set_group_impl(state: &AppState, label: &str, group: &str) {
    let mut guard = state.pins.lock().unwrap();
    if let Some(m) = guard.get_mut(label) {
        // 分组名去首尾空白；空值回退默认（否则"空组"在菜单里没法看）
        let g = group.trim();
        m.group = if g.is_empty() {
            DEFAULT_PIN_GROUP.to_string()
        } else {
            g.to_string()
        };
    }
}

/// 批量设置贴图可见性（窗口操作走主线程，与其它 pin 命令一致）
fn set_pins_visible(app: &AppHandle, items: Vec<(String, bool)>) {
    let app_cb = app.clone();
    let _ = app.run_on_main_thread(move || {
        for (label, visible) in items {
            if let Some(w) = app_cb.get_webview_window(&label) {
                if visible {
                    let _ = w.show();
                } else {
                    let _ = w.hide();
                }
            }
        }
    });
}

fn pin_show_group_only_impl(app: &AppHandle, state: &AppState, group: &str) {
    let items: Vec<(String, bool)> = state
        .pins
        .lock()
        .unwrap()
        .iter()
        .map(|(l, m)| (l.clone(), m.group == group))
        .collect();
    set_pins_visible(app, items);
}

fn pin_hide_group_impl(app: &AppHandle, state: &AppState, group: &str) {
    let items: Vec<(String, bool)> = state
        .pins
        .lock()
        .unwrap()
        .iter()
        .map(|(l, m)| (l.clone(), m.group != group))
        .collect();
    set_pins_visible(app, items);
}

fn pin_show_all_impl(app: &AppHandle, state: &AppState) {
    let items: Vec<(String, bool)> = state
        .pins
        .lock()
        .unwrap()
        .keys()
        .map(|l| (l.clone(), true))
        .collect();
    set_pins_visible(app, items);
}

// ---- 命令壳 ----

/// 当前存活贴图的分组清单（去重，含预设四项）。
/// 右键菜单的「分组」子菜单靠它渲染。
#[tauri::command]
pub fn pin_groups(state: State<'_, AppState>) -> Vec<String> {
    pin_groups_impl(&state)
}

/// 某个贴图当前所在分组
#[tauri::command]
pub fn pin_group_of(state: State<'_, AppState>, label: String) -> String {
    pin_group_of_impl(&state, &label)
}

/// 把贴图移到指定分组；返回刷新后的分组清单（菜单据此重画）。
#[tauri::command]
pub fn pin_set_group(state: State<'_, AppState>, label: String, group: String) -> Vec<String> {
    pin_set_group_impl(&state, &label, &group);
    pin_groups_impl(&state)
}

/// 只看本组：隐藏其它分组的贴图（贴图一多，屏幕上一片叠加时最有用）
#[tauri::command]
pub fn pin_show_group_only(
    app: AppHandle,
    state: State<'_, AppState>,
    group: String,
) -> Result<(), String> {
    pin_show_group_only_impl(&app, &state, &group);
    Ok(())
}

/// 隐藏某一分组的全部贴图
#[tauri::command]
pub fn pin_hide_group(
    app: AppHandle,
    state: State<'_, AppState>,
    group: String,
) -> Result<(), String> {
    pin_hide_group_impl(&app, &state, &group);
    Ok(())
}

/// 显示全部贴图（所有分组都放出来）
#[tauri::command]
pub fn pin_show_all(app: AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    pin_show_all_impl(&app, &state);
    Ok(())
}

/// 只改贴图的**显示尺寸**（滚轮缩放），不重渲染图片。
///
/// 为什么单独一个命令：`pin_apply_edit` 要传新 PNG（旋转/翻转才会变像素），
/// 而缩放只是"同一张图放大显示" —— 每滚一格都重新编码 PNG 纯属浪费
/// （大图上一次编码几十毫秒，滚轮会卡）。窗口尺寸变化本身由
/// `resize_pin_keep_center` 保证中心不动。
///
/// async 原因同 `pin_apply_edit`：内部要 `run_on_main_thread`。
#[tauri::command]
pub async fn pin_set_display_size(
    app: AppHandle,
    state: State<'_, AppState>,
    label: String,
    width: f64,
    height: f64,
) -> Result<(), String> {
    // 与其它 pin 命令一致：只认登记表里的窗口，防止前端拿任意 label 改尺寸
    if !state.pins.lock().unwrap().contains_key(&label) {
        return Err("pin window not found".to_string());
    }
    windows::resize_pin_keep_center(&app, &label, width, height)
}

#[tauri::command]
pub async fn pin_apply_edit(
    app: AppHandle,
    state: State<'_, AppState>,
    label: String,
    png_base64: String,
    width: f64,
    height: f64,
) -> Result<PinContextPayload, String> {
    if !state.pins.lock().unwrap().contains_key(&label) {
        return Err("pin window not found".to_string());
    }
    let raw = decode_base64_bytes(&png_base64)?;
    validate_png_header(&raw)?;

    let path = snappin_core_platform::new_temp_png_path();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    std::fs::write(&path, &raw).map_err(|e| format!("write edited png failed: {e}"))?;
    let path_str = path.to_string_lossy().to_string();

    // 换源 + 记住这份临时文件；上一份处理结果已无人引用，顺手删掉
    let prev = {
        let mut pins = state.pins.lock().unwrap();
        let meta = pins
            .get_mut(&label)
            .ok_or_else(|| "pin window not found".to_string())?;
        meta.src = path_str.clone();
        meta.width = width;
        meta.height = height;
        meta.edited.replace(path_str.clone())
    };
    if let Some(prev) = prev {
        if prev != path_str {
            let _ = std::fs::remove_file(prev);
        }
    }

    windows::resize_pin_keep_center(&app, &label, width, height)?;
    Ok(PinContextPayload {
        label,
        src: path_str,
        width,
        height,
    })
}

#[tauri::command]
pub async fn pin_save(
    app: AppHandle,
    state: State<'_, AppState>,
    _window: Window,
    label: String,
) -> Result<bool, String> {
    let save_dir = { state.config.lock().unwrap().save_dir.clone() };
    let src = pin_src(&state, &label)?;
    // 贴图窗是 always-on-top，会盖住系统保存对话框：临时取消置顶
    windows::set_window_topmost(&app, &label, false);
    let picked = save_dialog(
        &app,
        &snappin_core_platform::default_save_name(),
        save_dir,
    )
    .await;
    // 恢复置顶（窗口可能已被关闭，忽略错误）
    windows::set_window_topmost(&app, &label, true);
    if let Some(dest) = picked {
        let buffer =
            snappin_core_platform::load_rgba(Path::new(&src)).map_err(|e| e.to_string())?;
        snappin_core_platform::save_image(&buffer, &dest).map_err(|e| e.to_string())?;
        if let Some(parent) = dest.parent() {
            let dir_str = parent.to_string_lossy().to_string();
            let mut guard = state.config.lock().unwrap();
            if guard.save_dir.as_deref() != Some(dir_str.as_str()) {
                guard.save_dir = Some(dir_str.clone());
                let snapshot = guard.clone();
                drop(guard);
                if let Ok(dir) = app.path().app_config_dir() {
                    let _ = crate::config::save(&dir, &snapshot);
                }
            }
        }
        Ok(true)
    } else {
        Ok(false)
    }
}

// ---------- 悬浮球 / 窗口 ----------

#[tauri::command]
pub async fn ball_command(app: AppHandle, cmd: String) -> Result<(), String> {
    match cmd.as_str() {
        "capture" => {
            // 防重入：浮层已存在（含采集中 loading 态）时忽略重复触发
            if app
                .webview_windows()
                .keys()
                .any(|l| l.starts_with(windows::OVERLAY_PREFIX))
            {
                return Ok(());
            }
            let app2 = app.clone();
            tauri::async_runtime::spawn_blocking(move || {
                let state = app2.state::<AppState>();
                do_capture(&app2, &state, "ball")
            })
            .await
            .map_err(|e| format!("capture task failed: {e}"))?
            // 已有一轮采集在跑（含热键/托盘同时触发）：视为成功，
            // 不把 "capture already in progress" 当错误弹给用户。
            .map(|_| ())
            .or_else(|e| {
                if e.contains("already in progress") {
                    Ok(())
                } else {
                    Err(e)
                }
            })?;
        }
        "pin-clipboard" => {
            let app2 = app.clone();
            let r = tauri::async_runtime::spawn_blocking(move || {
                let state = app2.state::<AppState>();
                do_pin_from_clipboard(&app2, &state)
            })
            .await
            .map_err(|e| format!("pin task failed: {e}"))?;
            r.map(|_| ())?;
        }
        "hide" => {
            if let Some(w) = app.get_webview_window(windows::BALL_LABEL) {
                w.hide().map_err(|e| e.to_string())?;
            }
        }
        "quit" => app.exit(0),
        other => return Err(format!("unknown ball command: {}", other)),
    }
    Ok(())
}

/// 悬浮球右键菜单：在光标处打开**自绘菜单窗**。
///
/// 背景：wry 0.57 默认开启 WebView2 浏览器右键菜单（含"打印"等），
/// 已由 webview 初始化脚本抑制；本命令提供应用自己的右键菜单。
///
/// 不再用 `popup_menu`（原生 TrackPopupMenu）：样式完全由系统决定，
/// 做不出"浅色圆角 + 勾选列 + 右侧快捷键"的面板（见 menuwin.rs）。
///
/// async 的原因：建 webview 不能在主线程同步做（会死锁/白屏）。
#[tauri::command]
pub async fn ball_menu(app: AppHandle) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || crate::menuwin::open_menu_at_cursor(&app))
        .await
        .map_err(|e| format!("menu task failed: {e}"))?
}

/// 贴图右键菜单：在光标处打开自绘菜单窗，目标为本调用窗口。
///
/// 与 ball_menu 共用同一个菜单窗（单例），Rust 记下 kind=Pin + 本窗 label，
/// 前端据此渲染贴图菜单（浅色面板 + 勾选项），点击后动作经
/// `pin-menu-action` 事件转发回本窗执行。
#[tauri::command]
pub async fn pin_menu_open(app: AppHandle, window: Window) -> Result<(), String> {
    let label = window.label().to_string();
    tauri::async_runtime::spawn_blocking(move || crate::menuwin::open_pin_menu_at_cursor(&app, &label))
        .await
        .map_err(|e| format!("pin menu task failed: {e}"))?
}

/// 贴图窗发布自己的状态快照（界面开关 + 图像处理参数）。
///
/// 为什么要发：菜单窗与贴图窗是两个独立 webview，菜单要画勾选标记
/// （✓ 窗口置顶 / ✓ 灰度化 / 不透明度当前档位）就必须知道贴图的当前状态；
/// 状态的"事实来源"在贴图窗的 React 里，故由它在挂载时和每次切换后
/// 主动上报，Rust 只缓存。
#[tauri::command]
pub fn pin_publish_flags(state: State<'_, AppState>, label: String, flags: PinFlags) {
    state.pin_flags.lock().unwrap().insert(label, flags);
}

/// 读取贴图界面开关快照（无记录 = 默认值：置顶开、未锁、有阴影）。
#[tauri::command]
pub fn pin_flags(state: State<'_, AppState>, label: String) -> PinFlags {
    state
        .pin_flags
        .lock()
        .unwrap()
        .get(&label)
        .copied()
        .unwrap_or_default()
}

/// `pin-set-opacity` 事件载荷（Rust → 贴图窗）。
#[derive(Clone, serde::Serialize)]
struct PinOpacityPayload {
    value: f64,
}

/// 菜单里的「不透明度」滑块：把实时值转给目标贴图窗。
///
/// 为什么不让菜单窗直接改：贴图窗的透明度是它自己 CSS 上的 `opacity`
/// （`<img>` 的 style），跨 webview 改不到；所以走
/// 菜单 → Rust → `pin-set-opacity` 事件 → 贴图窗 setState 这一跳。
///
/// 顺手把值写进 `pin_flags` 快照：菜单关掉再打开时滑块要停在原处，
/// 否则会"跳回 100%"（快照只能由贴图窗发布，拖动期间它还没发）。
#[tauri::command]
pub fn pin_set_opacity(
    app: AppHandle,
    state: State<'_, AppState>,
    label: String,
    value: f64,
) -> Result<(), String> {
    // 与工具条滑条 / Alt+滚轮同一个下限，避免拖到 0 后贴图"消失找不回来"
    let v = value.clamp(0.15, 1.0);
    state
        .pin_flags
        .lock()
        .unwrap()
        .entry(label.clone())
        .or_default()
        .opacity = v;
    match app.get_webview_window(&label) {
        Some(w) => w
            .emit("pin-set-opacity", PinOpacityPayload { value: v })
            .map_err(|e| format!("emit pin-set-opacity failed: {e}")),
        // 窗口已关（菜单还没收）时静默：拖动最后几帧撞上关闭属正常
        None => Ok(()),
    }
}

/// 前端量出菜单尺寸后调用：按锚点定位、resize、显示、抢焦点。
#[tauri::command]
pub async fn menu_fit(app: AppHandle, width: f64, height: f64) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || crate::menuwin::fit_and_show(&app, width, height))
        .await
        .map_err(|e| format!("menu fit task failed: {e}"))?
}

/// 关闭菜单（Esc / 点击菜单项后）。同步命令：只 hide，不建窗。
#[tauri::command]
pub fn menu_close(app: AppHandle) {
    crate::menuwin::hide_menu(&app);
}

/// 菜单初始状态。
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MenuStatePayload {
    /// 是否处于"打开流程中"（窗口刚建好时前端收不到 emit，靠它补一刀）
    pub open: bool,
    /// 注册失败的内置快捷键名（空 = 正常）
    pub shortcut_failed: Vec<String>,
    /// 当前菜单种类：`app`（托盘/悬浮球）或 `pin`（贴图右键）。
    /// 首次创建菜单窗时 `menu-open` 事件会丢（前端还没注册监听），
    /// 靠它补上正确的菜单内容，否则第一次右键贴图会弹出托盘菜单。
    pub kind: &'static str,
    /// 贴图菜单的目标窗口 label（kind = pin 时有值）
    pub pin_label: Option<String>,
}

#[tauri::command]
pub fn menu_state(app: AppHandle, state: State<'_, AppState>) -> MenuStatePayload {
    let (kind, pin_label) = {
        let guard = state.menu_anchor.lock().unwrap();
        match guard.as_ref() {
            Some(a) => (a.kind, a.pin_label.clone()),
            None => (MenuKind::App, None),
        }
    };
    MenuStatePayload {
        open: crate::menuwin::menu_is_open(&app),
        shortcut_failed: state.shortcut_failed.lock().unwrap().clone(),
        kind: match kind {
            MenuKind::App => "app",
            MenuKind::Pin => "pin",
        },
        pin_label,
    }
}

/// `pin-menu-action` 事件载荷（Rust → 贴图窗）。
#[derive(Clone, serde::Serialize)]
struct PinMenuActionPayload {
    id: String,
}

/// 菜单项点击：先关菜单再分发动作。
///
/// 复用 `handle_menu_event`——与托盘菜单共用同一套动作，避免两处行为漂移。
/// **贴图菜单例外**：动作转发给对应贴图窗（`pin-menu-action` 事件），
/// 由它的 React 执行——置顶/锁定/阴影等界面状态的事实来源在那里。
#[tauri::command]
pub fn menu_action(app: AppHandle, id: String) -> Result<(), String> {
    // 先取锚点再 hide（hide 会清空锚点）
    let route = {
        let state = app.state::<AppState>();
        let guard = state.menu_anchor.lock().unwrap();
        guard
            .as_ref()
            .map(|a| (a.kind, a.pin_label.clone()))
    };
    crate::menuwin::hide_menu(&app);
    match route {
        Some((MenuKind::Pin, Some(label))) => {
            // 分组动作是**全局**操作（影响所有贴图），在 Rust 侧直接做完，
            // 不转发给单个贴图窗 —— 转发过去那个窗也没法操作别人的窗口。
            if let Some(rest) = id.strip_prefix("pin-group:") {
                let state = app.state::<AppState>();
                pin_set_group_impl(&state, &label, rest);
                return Ok(());
            }
            match id.as_str() {
                "pin-group-show-all" => {
                    pin_show_all_impl(&app, &app.state::<AppState>());
                    return Ok(());
                }
                "pin-group-only" => {
                    let state = app.state::<AppState>();
                    let g = pin_group_of_impl(&state, &label);
                    pin_show_group_only_impl(&app, &state, &g);
                    return Ok(());
                }
                "pin-group-hide" => {
                    let state = app.state::<AppState>();
                    let g = pin_group_of_impl(&state, &label);
                    pin_hide_group_impl(&app, &state, &g);
                    return Ok(());
                }
                _ => {}
            }
            if let Some(w) = app.get_webview_window(&label) {
                w.emit("pin-menu-action", PinMenuActionPayload { id })
                    .map_err(|e| format!("emit pin-menu-action failed: {e}"))?;
            } else {
                crate::logln!("pin menu action but window gone: {label}");
            }
        }
        _ => crate::handle_menu_event(&app, &id),
    }
    Ok(())
}

/// 悬浮球**实时预览**：拖动尺寸/不透明度滑条时调用，改完不落盘。
///
/// 为什么单独一条命令而不是直接 update_config：
/// 1) 拖滑条每帧存盘会疯狂写 config.json；
/// 2) 预览要允许"取消"——用户松手不保存时，下次启动仍是旧值。
/// 落盘仍走 update_config，这里只负责把当前值推给球窗口。
#[tauri::command]
pub async fn ball_preview(app: AppHandle, size: u32, opacity: f64) -> Result<(), String> {
    let size = size.clamp(24, 160) as f64;
    let opacity = opacity.clamp(0.15, 1.0);
    let app_cb = app.clone();
    windows::run_on_main(&app, move |app| {
        windows::resize_ball_window(app, size);
        if let Some(w) = app_cb.get_webview_window(windows::BALL_LABEL) {
            let _ = w.emit(
                "ball-preview",
                serde_json::json!({ "size": size, "opacity": opacity }),
            );
        }
    });
    Ok(())
}

/// 对调用方窗口启动 OS 原生拖拽循环。
#[tauri::command]
pub fn drag_window(window: Window) -> Result<(), String> {
    window.start_dragging().map_err(|e| e.to_string())
}

// ---------- 配置 / 信息 ----------

#[tauri::command]
pub fn get_config(state: State<'_, AppState>) -> Result<AppConfig, String> {
    Ok(state.config.lock().unwrap().clone())
}

/// 更新配置并落盘；悬浮球尺寸变化立即生效。
///
/// async 原因：内部 run_on_main 向主线程投递闭包并等待完成，
/// 同步命令在主线程执行会自等待死锁（见 ball_menu 注释）。
#[tauri::command]
pub async fn update_config(
    app: AppHandle,
    state: State<'_, AppState>,
    config: AppConfig,
) -> Result<AppConfig, String> {
    let dir = app.path().app_config_dir().map_err(|e| e.to_string())?;
    crate::config::save(&dir, &config).map_err(|e| e.to_string())?;
    {
        let mut guard = state.config.lock().unwrap();
        *guard = config.clone();
    }
    // 悬浮球尺寸变化立即生效（窗口操作须在主线程执行）。
    //
    // ★ 窗口尺寸必须是 `球径 + 2×留白`（见 windows::ball_window_size）：
    //   早先这里直接 set_size(球径, 球径)，窗口正好等于球 —— 投影、hover 放大、
    //   1px 描边全被窗口边界裁掉，表现为"尺寸调小后浮球显示不完整"。
    //   同时要重新贴右上角：窗口变小后旧位置会把球推离原位。
    let new_size = config.ball_size as f64;
    let opacity = config.ball_opacity;
    windows::run_on_main(&app, move |app| {
        windows::resize_ball_window(app, new_size);
        if let Some(w) = app.get_webview_window(windows::BALL_LABEL) {
            // 不透明度交给前端 CSS（Windows 下 WebView2 窗口没有 set_opacity）
            let _ = w.emit("ball-preview", serde_json::json!({ "size": new_size, "opacity": opacity }));
        }
    });
    // 快捷键变化：热重注册（M7）。注册是全局的，换键要先注销旧的；
    // 失败不阻断保存，把失败名带回前端提示（与启动时的 sc_failed 同思路）。
    let mut failed: Vec<String> = Vec::new();
    // 闭包必须 'static：这里持有**拥有版**字符串，不借 config
    let targets = [
        (
            "截图",
            config.shortcut_capture.clone(),
            "Ctrl+1".to_string(),
        ),
        (
            "剪贴板贴图",
            config.shortcut_pin_clipboard.clone(),
            "Ctrl+2".to_string(),
        ),
    ];
    for (label, spec, default) in targets {
        let Some(sc) = parse_shortcut(&spec) else {
            failed.push(format!("{label}：无法识别“{spec}”"));
            continue;
        };
        let old = parse_shortcut(&default).expect("builtin default shortcut");
        let label_cb = label.to_string();
        let spec_cb = spec.clone();
        windows::post_to_main(&app, move |app| {
            let gs = app.global_shortcut();
            let _ = gs.unregister(old);
            if let Err(e) = gs.register(sc) {
                eprintln!("[snappin] re-register {label_cb} ({spec_cb}) failed: {e}");
            }
        });
        // 同步更新内存里的"当前生效键"：热键处理器靠它比对，
        // 不更新的话用户改了键，按新键没反应、按旧键反而还有（注销是异步的）。
        let mut hot = state.hotkeys.lock().unwrap();
        if label == "截图" {
            hot.0 = sc;
        } else {
            hot.1 = sc;
        }
    }
    if !failed.is_empty() {
        return Err(format!("快捷键未生效：{}", failed.join("；")));
    }
    Ok(config)
}

/// 快捷键录制期间**临时注销全部全局热键**。
///
/// 为什么必须这样：录制时用户按下的组合键，如果正好等于当前已注册的键
/// （例如要改「开始截图」，照着原值按 Ctrl+1），全局热键会立刻触发截图，
/// 浮层铺满全屏把设置页盖住 —— 用户看到的就是"设置快捷键时突然截图了，
/// 根本设置不了"（实测反馈）。录制结束（拿到组合键或按 Esc）再恢复。
#[tauri::command]
pub fn hotkeys_suspend(app: AppHandle) -> Result<(), String> {
    app.global_shortcut()
        .unregister_all()
        .map_err(|e| e.to_string())?;
    crate::logln!("[snappin] hotkeys suspended (recording)");
    Ok(())
}

/// 录制结束后按**已保存的配置**重新注册全局热键。
///
/// 注意用的是 config 里的值而不是前端草稿：用户录完没点「保存设置」的话，
/// 生效的仍应是旧键（与 update_config 的语义一致）。
#[tauri::command]
pub fn hotkeys_resume(app: AppHandle) -> Result<(), String> {
    // 先全部注销再注册：让本函数**幂等**。它既会被前端的录制结束调用，
    // 也会被窗口销毁兜底调用；不先注销的话重复注册同一个键会报错。
    let _ = app.global_shortcut().unregister_all();
    let cfg = {
        let state = app.state::<AppState>();
        let c = state.config.lock().unwrap().clone();
        c
    };
    let capture_sc =
        parse_shortcut(&cfg.shortcut_capture).unwrap_or_else(crate::default_capture_shortcut);
    let pin_sc = parse_shortcut(&cfg.shortcut_pin_clipboard)
        .unwrap_or_else(crate::default_pin_shortcut);
    {
        let state = app.state::<AppState>();
        *state.hotkeys.lock().unwrap() = (capture_sc.clone(), pin_sc.clone());
    }
    let gs = app.global_shortcut();
    let mut errs: Vec<String> = Vec::new();
    let mut failed_names: Vec<String> = Vec::new();
    if let Err(e) = gs.register(capture_sc) {
        errs.push(format!("{}: {e}", cfg.shortcut_capture));
        failed_names.push("截图快捷键".to_string());
    }
    if let Err(e) = gs.register(pin_sc) {
        errs.push(format!("{}: {e}", cfg.shortcut_pin_clipboard));
        failed_names.push("贴图快捷键".to_string());
    }
    // 挂起再恢复之间，键可能被别的程序抢走了（RegisterHotKey 先到先得）。
    // 恢复失败必须让用户看得见：写进 shortcut_failed，自绘菜单顶部会显示告警
    // ——否则表现为"改了个快捷键之后热键全没了"，用户完全找不到线索。
    {
        let state = app.state::<AppState>();
        *state.shortcut_failed.lock().unwrap() = failed_names;
    }
    crate::logln!(
        "[snappin] hotkeys resumed: {} / {} {:?}",
        cfg.shortcut_capture,
        cfg.shortcut_pin_clipboard,
        errs
    );
    if errs.is_empty() {
        Ok(())
    } else {
        Err(format!("热键恢复失败：{}", errs.join("；")))
    }
}

/// 把 "Ctrl+1" / "Ctrl+Shift+A" 这类配置字符串解析成 `Shortcut`。
///
/// 支持的修饰键：Ctrl / Shift / Alt / Super（Win 键 / ⌘）；
/// 主键：0-9、A-Z、F1-F12。解析不了返回 None——调用方必须给出可见错误，
/// 不能静默回退到默认值（用户会以为改成功了）。
pub fn parse_shortcut(spec: &str) -> Option<Shortcut> {
    use tauri_plugin_global_shortcut::{Code, Modifiers};
    let mut mods: Option<Modifiers> = None;
    let mut code: Option<Code> = None;
    for part in spec.split('+').map(|p| p.trim()).filter(|p| !p.is_empty()) {
        let m = match part.to_ascii_lowercase().as_str() {
            "ctrl" | "control" => Some(Modifiers::CONTROL),
            "shift" => Some(Modifiers::SHIFT),
            "alt" | "option" => Some(Modifiers::ALT),
            "super" | "cmd" | "command" | "win" => Some(Modifiers::SUPER),
            _ => None,
        };
        if let Some(m) = m {
            mods = Some(match mods {
                Some(cur) => cur | m,
                None => m,
            });
            continue;
        }
        let up = part.to_ascii_uppercase();
        code = parse_code(&up).or(code);
    }
    Some(Shortcut::new(mods, code?))
}

/// 单个主键字符串 → `Code`（0-9 / A-Z / F1-F12）。
fn parse_code(up: &str) -> Option<tauri_plugin_global_shortcut::Code> {
    use tauri_plugin_global_shortcut::Code;
    if up.len() > 1 {
        // F1..F12（长度 >1 才可能是功能键；单字母 F 走字母分支）
        let n: u8 = up.strip_prefix('F')?.parse().ok()?;
        return match n {
            1 => Some(Code::F1),
            2 => Some(Code::F2),
            3 => Some(Code::F3),
            4 => Some(Code::F4),
            5 => Some(Code::F5),
            6 => Some(Code::F6),
            7 => Some(Code::F7),
            8 => Some(Code::F8),
            9 => Some(Code::F9),
            10 => Some(Code::F10),
            11 => Some(Code::F11),
            12 => Some(Code::F12),
            _ => None,
        };
    }
    let ch = up.chars().next()?;
    match ch {
        '0' => Some(Code::Digit0),
        '1' => Some(Code::Digit1),
        '2' => Some(Code::Digit2),
        '3' => Some(Code::Digit3),
        '4' => Some(Code::Digit4),
        '5' => Some(Code::Digit5),
        '6' => Some(Code::Digit6),
        '7' => Some(Code::Digit7),
        '8' => Some(Code::Digit8),
        '9' => Some(Code::Digit9),
        'A' => Some(Code::KeyA),
        'B' => Some(Code::KeyB),
        'C' => Some(Code::KeyC),
        'D' => Some(Code::KeyD),
        'E' => Some(Code::KeyE),
        'F' => Some(Code::KeyF),
        'G' => Some(Code::KeyG),
        'H' => Some(Code::KeyH),
        'I' => Some(Code::KeyI),
        'J' => Some(Code::KeyJ),
        'K' => Some(Code::KeyK),
        'L' => Some(Code::KeyL),
        'M' => Some(Code::KeyM),
        'N' => Some(Code::KeyN),
        'O' => Some(Code::KeyO),
        'P' => Some(Code::KeyP),
        'Q' => Some(Code::KeyQ),
        'R' => Some(Code::KeyR),
        'S' => Some(Code::KeyS),
        'T' => Some(Code::KeyT),
        'U' => Some(Code::KeyU),
        'V' => Some(Code::KeyV),
        'W' => Some(Code::KeyW),
        'X' => Some(Code::KeyX),
        'Y' => Some(Code::KeyY),
        'Z' => Some(Code::KeyZ),
        _ => None,
    }
}

#[tauri::command]
pub fn app_info(app: AppHandle) -> Result<AppInfoPayload, String> {
    Ok(AppInfoPayload {
        version: app.package_info().version.to_string(),
        platform: std::env::consts::OS.to_string(),
        arch: std::env::consts::ARCH.to_string(),
    })
}

// ---------- 内部工具 ----------

fn build_capture_payload(shots: &[snappin_core_platform::MonitorShot]) -> CapturePayload {
    CapturePayload {
        monitors: shots
            .iter()
            .map(|m| MonitorInfoPayload {
                id: m.id.clone(),
                name: m.name.clone(),
                x: m.x,
                y: m.y,
                width: m.width,
                height: m.height,
                scale_factor: m.scale_factor,
            })
            .collect(),
        shots: shots
            .iter()
            .map(|m| ShotAssetPayload {
                monitor_id: m.id.clone(),
                path: m.path.to_string_lossy().to_string(),
            })
            .collect(),
    }
}

/// 由临时文件路径生成贴图窗口标签：pin-snap-<uuid8>。
fn pin_label(path: &Path) -> String {
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("snap")
        .to_string();
    format!("{}{}", windows::PIN_PREFIX, stem)
}

/// 登记贴图元数据。
fn register_pin(state: &AppState, label: &str, src: &str, width: f64, height: f64) {
    state.pins.lock().unwrap().insert(
        label.to_string(),
        PinMeta {
            src: src.to_string(),
            width,
            height,
            group: DEFAULT_PIN_GROUP.to_string(),
            edited: None,
        },
    );
}

/// 查询贴图元数据中的图片路径。
fn pin_src(state: &AppState, label: &str) -> Result<String, String> {
    let pins = state.pins.lock().unwrap();
    pins.get(label)
        .map(|m| m.src.clone())
        .ok_or_else(|| "pin window not found".to_string())
}

/// 弹出保存对话框并等待用户选择（取消返回 None）。
async fn save_dialog(
    app: &AppHandle,
    default_name: &str,
    save_dir: Option<String>,
) -> Option<std::path::PathBuf> {
    use tauri_plugin_dialog::DialogExt;

    let (tx, rx) = mpsc::channel::<Option<std::path::PathBuf>>();
    let mut builder = app
        .dialog()
        .file()
        .set_file_name(default_name)
        .add_filter("PNG 图片", &["png"])
        .add_filter("JPEG 图片", &["jpg", "jpeg"]);
    if let Some(dir) = save_dir {
        builder = builder.set_directory(dir);
    }
    builder.save_file(move |picked| {
        let path = match picked {
            Some(tauri_plugin_dialog::FilePath::Path(p)) => Some(p),
            _ => None,
        };
        let _ = tx.send(path);
    });

    // 对话框回调走主线程事件循环；async 命令让出线程后回调才能送达
    tauri::async_runtime::spawn_blocking(move || {
        rx.recv_timeout(Duration::from_secs(600)).ok().flatten()
    })
    .await
    .ok()
    .flatten()
}

// ---------- 长截图（M5） ----------

/// 丢弃长截图会话（不产出图）：会话结束 / 用户取消截图时调用。
///
/// 由 `windows::end_capture_session` 在每条退出路径上统一调用——
/// 包括用户没点结果面板按钮就直接取消截图的情形（会话不能滞留，
/// 否则下次长截图会撞上 "already in progress"）。
/// 驱动循环（若还在跑）下一步发现会话没了会自行退出，不做收尾。
pub fn discard_long_session(app: &AppHandle, state: &AppState, why: &str) {
    // 结果窗已经接手（拼接完成 → 退出截图模式）时不要丢弃会话，
    // 否则结果窗的保存/复制/贴图全部报 "session not found"。
    if state.long_keep.load(Ordering::SeqCst) {
        eprintln!("[snappin] keep long capture for result window (why={why})");
        return;
    }
    let session = { state.long.lock().unwrap().take() };
    let Some(session) = session else {
        return;
    };
    let _ = std::fs::remove_file(&session.thumb);
    state.long_keep.store(false, Ordering::SeqCst);
    eprintln!(
        "[snappin] discard long capture (why={why}, height={}px, steps={})",
        session.capture.height(),
        session.capture.steps()
    );
    // 先叫停再还原：驱动循环可能在当前步的稳态等待里
    state.long_stop.store(true, Ordering::SeqCst);
    restore_long_overlay(
        app,
        session.overlay_idx,
        session.overlay_hwnd,
        session.used_fallback,
    );
    state.long_active.store(false, Ordering::SeqCst);
    unregister_long_esc(app);
    windows::close_long_preview(app);
}

/// 会话显示器 ID `"m{idx}"` → `xcap::Monitor::all()` 下标。
///
/// ID 由 `capture_all_monitors` 按枚举顺序生成（见 core-platform `capture_shot`），
/// 长截图据此重新取同一枚显示器。解析必须严格：错一位就会拼错屏幕的内容。
fn monitor_index(monitor_id: &str) -> Result<usize, String> {
    monitor_id
        .strip_prefix('m')
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| format!("invalid monitor id: {monitor_id}"))
}

/// 长截图进度事件名（前端 `listen("long-capture-progress")`）。
pub const LONG_EVENT: &str = "long-capture-progress";

/// 手动采样步数上限，防止用户长时间不结束会话。
const LONG_MAX_STEPS: u32 = 3600;
/// 贴图封顶比例：长图逻辑尺寸相对主屏幅面的上限（0.7 = 70%）。
const LONG_PIN_MAX_RATIO: f64 = 0.7;

/// 长截图进度载荷（`long-capture-progress` 事件）。
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct LongProgressPayload {
    /// "scrolling" 滚动拼接中 | "done" 正常结束 | "error" 出错
    pub phase: String,
    /// 当前累积高度（物理像素）
    pub height: u32,
    /// 累计步数
    pub steps: u32,
    /// 最近一次匹配置信度 0..1（Still 步为帧相似度）
    pub score: f32,
    /// 缩略图临时 PNG 路径（前端 convertFileSrc 加载）
    pub thumb: String,
    /// 状态条文案
    pub message: String,
}

/// 长截图：复用当前截图会话的浮层，把选区交给驱动循环自动滚动拼接。
///
/// 流程（重活在 `begin_long_session`，放在阻塞线程里做）：
/// 1. 校验会话代号 / 防重入 / 逻辑→物理选区换算
/// 2. 浮层改为光标穿透（滚轮事件落到下层页面）
/// 3. Windows 给浮层设 WDA 采集排除；失败则退化为"采集前后隐藏浮层"
/// 4. 注册全局 Esc（长截图期间专用，结束即注销）
/// 5. 采第一帧建会话 → 启动驱动循环
#[tauri::command]
pub async fn long_capture_start(
    app: AppHandle,
    state: State<'_, AppState>,
    monitor_id: String,
    rect: RectPayload,
    session_id: Option<u64>,
) -> Result<(), String> {
    // 陈旧会话防护（与 confirm_selection 同口径）
    if let Some(id) = session_id {
        let current = state.session.load(Ordering::SeqCst);
        if id != current {
            return Err(format!("stale session {id} != {current}"));
        }
    }
    // 防重入（驱动循环持锁期间这里会立刻失败）
    //
    // 例外：会话槽位有值但 `long_active=false` 只可能是**上一轮泄漏**
    // （建会话成功后、置 active 之前被打断）。不清理的话此后每次长截图都报
    // "already in progress"，用户只能重启应用。这里直接丢弃重建。
    {
        let mut guard = state.long.lock().unwrap();
        if guard.is_some() {
            if state.long_active.load(Ordering::SeqCst) {
                return Err("long capture already in progress".to_string());
            }
            crate::logln!("[snappin] long: 丢弃上一轮泄漏的会话（long_active=false）");
            *guard = None;
            state.long_keep.store(false, Ordering::SeqCst);
        }
    }
    // monitor_id "m0" → xcap::Monitor::all()[0]（id 即采集下标，见 capture_shot）
    let idx = monitor_index(&monitor_id)?;
    // 选区坐标 → 采集坐标系。
    //
    // **两个平台的单位不同，不能一刀切乘 sf**：
    // - Windows：xcap 走 BitBlt，`Monitor.x()/y()`、`capture_region`、光标定位
    //   全是**物理像素** → 必须乘 sf；
    // - macOS：xcap 走 CGDisplayCreateImageForRect，`Monitor.x()/y()`、
    //   `capture_region`、`CGWarpMouseCursorPosition` 全是**逻辑点** → 不能乘。
    //
    // 乘错的后果（2026-10-26 真机）：直接把逻辑值翻倍当物理传进去，
    //   `Region (0, 50, 3360, 1850) is outside monitor bounds (0, 0, 1680, 1050)`
    //   —— 长截图在 Retina 屏上 100% 启动失败。
    //
    // 为什么必须整段统一：驱动循环里 `scroll_point = monitor 原点 + 选区中心`、
    // 以及光标残影修复的 `cx = px - rect.x`，都默认 rect 与光标同单位。
    // 只有两种单位各自内部自洽，混用就会滚到屏幕外。
    let cap_rect = {
        let monitors = state.monitors.lock().unwrap();
        let m = monitors
            .iter()
            .find(|m| m.id == monitor_id)
            .ok_or_else(|| "no active capture session".to_string())?;
        #[cfg(target_os = "macos")]
        let k = 1.0_f64;
        #[cfg(not(target_os = "macos"))]
        let k = m.scale_factor as f64;
        AlgoRect {
            x: (rect.x * k).round() as i32,
            y: (rect.y * k).round() as i32,
            w: (rect.w * k).round().max(1.0) as u32,
            h: (rect.h * k).round().max(1.0) as u32,
        }
    };
    let app2 = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        if let Err(e) = begin_long_session(&app2, idx, cap_rect) {
            eprintln!("[snappin] long capture start failed: {e}");
            emit_long(&app2, "error", 0, 0, 0.0, "", format!("长截图启动失败：{e}"));
        }
    })
    .await
    .map_err(|e| format!("long capture task failed: {e}"))?;
    Ok(())
}

/// 建会话 + 改窗口状态 + 启动驱动循环（阻塞线程）。
fn begin_long_session(
    app: &AppHandle,
    idx: usize,
    rect: AlgoRect,
) -> Result<(), String> {
    let state = app.state::<AppState>();
    // 防重入（命令体已查过一次；await 之间可能有并发启动）
    if state.long.lock().unwrap().is_some() {
        return Err("long capture already in progress".to_string());
    }
    // 选区太小拼不动（重叠要求最低 24 行，再算上边距留量）
    if rect.h < 80 {
        return Err(format!("选区太矮（{}px），长截图至少需要 80px 高", rect.h));
    }
    let overlay_label = format!("{}{}", windows::OVERLAY_PREFIX, idx);
    let overlay_hwnd = overlay_window_hwnd(app, &overlay_label);

    let excluded = overlay_hwnd != 0
        && snappin_core_platform::stitch::set_hwnd_capture_excluded(overlay_hwnd, true);
    #[cfg(windows)]
    if !excluded {
        return Err("cannot exclude selection border from capture".to_string());
    }

    // ★ 整个拼接期间**不显示浮层**，因此也不需要"抓帧前后隐藏/显示"的回调。
    //
    // 这里原来在 WDA 不可用时（macOS 永远走这条）装配了 hide_ui/show_ui 兜底：
    // 抓帧前藏、抓帧后立刻显示回来。问题在于紧接着下面又整体隐藏了浮层，
    // 于是 show_ui 每次都把浮层**重新显示**出来 —— 结果是整段会话里一个全屏
    // WebView 一直盖在网页上方。macOS 上滚轮事件被这个窗口吃掉，
    // 页面一步都不滚：真机日志表现为 `long done: 页面不再滚动 (1850px, 5 steps)`，
    // 高度恒等于选区高度、视频里网页完全没动。
    //
    // 现在改为：拼接期间浮层全程隐藏（抓帧自然干净，也无需排除），
    // 结束时再 restore_long_overlay 显示结果面板。
    let (hide_ui, show_ui): (Option<UiCallback>, Option<UiCallback>) = (None, None);
    // 光标穿透：滚轮与点击都要落到下层的目标窗口
    windows::set_window_ignore_cursor_events(app, &overlay_label, true);
    windows::hide_all_overlays(app);
    #[cfg(target_os = "macos")]
    std::thread::sleep(std::time::Duration::from_millis(120));

    // 停止标志复位（上次异常退出可能留着 true），克隆进会话
    let stop = state.long_stop.clone();
    stop.store(false, Ordering::SeqCst);
    let thumb = snappin_core_platform::new_temp_png_path();

    let monitor = snappin_core_platform::monitor_by_index(idx).map_err(|e| e.to_string())?;
    // 建会话前把坐标打出来：macOS 上"滚不动"若是因为滚动点算错（落在别的窗口
    // 甚至屏幕外），日志里这一行就是唯一的证据。
    {
        let (ox, oy) = (
            monitor.x().unwrap_or(-1),
            monitor.y().unwrap_or(-1),
        );
        crate::logln!(
            "[snappin] long manual session: rect=({},{},{}x{}) monitor_origin=({},{})",
            rect.x,
            rect.y,
            rect.w,
            rect.h,
            ox,
            oy,
        );
    }
    let capture = LongCapture::new(
        monitor,
        rect,
        StitchOptions::for_frame(rect.h),
        0,
        hide_ui,
        show_ui,
        Some(stop),
        true,
    );
    let capture = match capture {
        Ok(c) => c,
        Err(e) => {
            // 第一帧就失败：还原已改的窗口状态，别把浮层卡在穿透/排除态
            restore_long_overlay(app, idx, overlay_hwnd, !excluded);
            return Err(e);
        }
    };

    if let Err(e) = write_thumbnail(capture.canvas(), &thumb) {
        restore_long_overlay(app, idx, overlay_hwnd, !excluded);
        return Err(e);
    }
    let first_height = capture.height();
    let first_steps = capture.steps();
    let thumb_str = thumb.to_string_lossy().to_string();
    *state.long.lock().unwrap() = Some(LongSession {
        capture,
        overlay_idx: idx,
        overlay_hwnd,
        used_fallback: !excluded,
        preview: None,
        thumb,
    });
    if let Err(e) = windows::show_only_long_overlay(app, idx) {
        discard_long_session(app, &state, "selection-border-open-failed");
        return Err(e);
    }
    #[cfg(target_os = "macos")]
    {
        let id = app
            .get_webview_window(&overlay_label)
            .map(|window| overlay_window_number(&window))
            .and_then(|id| u32::try_from(id).ok())
            .filter(|id| *id != 0);
        let Some(id) = id else {
            discard_long_session(app, &state, "selection-border-id-failed");
            return Err("selection border window number unavailable".to_string());
        };
        if let Some(session) = state.long.lock().unwrap().as_mut() {
            session.capture.set_excluded_window(id);
        }
        crate::logln!("[snappin] long capture below selection border id={id}");
    }
    if let Err(e) = windows::show_long_preview(app) {
        discard_long_session(app, &state, "preview-open-failed");
        return Err(e);
    }
    state.long_active.store(true, Ordering::SeqCst);
    register_long_esc(app);
    emit_long(
        app,
        "scrolling",
        first_height,
        first_steps,
        1.0,
        &thumb_str,
        "滚动页面继续截图",
    );

    // 手动滚轮驱动循环：独立线程采集页面变化，不接管滚轮。
    let app2 = app.clone();
    tauri::async_runtime::spawn_blocking(move || run_long_driver(&app2));
    Ok(())
}

/// 停止长截图（Esc / 前端停止按钮）：置位标志即返回，
/// 驱动循环在稳态等待间隙（~100ms）发现后收尾。
///
/// 同步命令在主线程执行，**不能取 `long` 锁**——驱动循环一步要持锁约 1s，
/// 取了会冻结 UI。改用 `long_active` 原子快照判断会话是否存在。
#[tauri::command]
pub fn long_capture_stop(state: State<'_, AppState>) -> Result<(), String> {
    if !state.long_active.load(Ordering::SeqCst) {
        return Err("long capture session not found".to_string());
    }
    state.long_stop.store(true, Ordering::SeqCst);
    Ok(())
}

#[tauri::command]
pub fn long_preview_info(state: State<'_, AppState>) -> Result<LongProgressPayload, String> {
    let guard = state.long.lock().unwrap();
    let session = guard.as_ref().ok_or_else(|| "long capture session not found".to_string())?;
    Ok(LongProgressPayload {
        phase: "scrolling".to_string(),
        height: session.capture.height(),
        steps: session.capture.steps(),
        score: 1.0,
        thumb: session.thumb.to_string_lossy().to_string(),
        message: "滚动页面继续截图".to_string(),
    })
}

/// 长截图收尾出口：copy=复制到剪贴板 / pin=贴图 / save=保存。
///
/// 取出会话 → 还原浮层 → 落盘最终长图 → 执行动作 → 结束截图会话。
/// 驱动循环应已在 done 事件前退出；若仍在滚动（异常路径），
/// 拿锁后它会自行发现会话被取走而退出。
/// 长截图结果窗的展示载荷。
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LongResultPayload {
    /// 全分辨率结果的临时 PNG 路径
    pub path: String,
    pub width: u32,
    pub height: u32,
    pub steps: u32,
}

/// 拼接完成 → 把会话交给结果窗，并**退出截图模式**。
///
/// 前端在收到 `done` 事件后调用：先是这里（生成全图、置位"保留会话"标记、
/// 打开结果窗），再调 `exitCapture` 收掉浮层。顺序不能反：浮层一关就会
/// `end_capture_session` → `discard_long_session`，所以标记必须在此之前置位。
#[tauri::command]
pub async fn long_result_open(app: AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    // 1) 生成（或复用）全分辨率 PNG
    let path = {
        let mut guard = state.long.lock().unwrap();
        let session = guard.as_mut().ok_or_else(|| "no long session".to_string())?;
        match &session.preview {
            Some(p) => p.clone(),
            None => {
                let canvas = session.capture.canvas();
                let path = snappin_core_platform::new_temp_png_path();
                if let Some(dir) = path.parent() {
                    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
                }
                snappin_core_platform::save_png_fast(canvas, &path).map_err(|e| e.to_string())?;
                session.preview = Some(path.clone());
                // 驱动循环已经结束，这里顺手把停止标志复位，避免影响下一次
                path
            }
        }
    };
    crate::logln!("[snappin] long result → {}", path.display());
    if let Some(session) = state.long.lock().unwrap().as_ref() {
        let _ = std::fs::remove_file(&session.thumb);
    }

    // 2) 置位"保留会话"：紧随其后的 exitCapture 会走 discard_long_session
    state.long_keep.store(true, Ordering::SeqCst);

    // 3) 开结果窗（必须主线程，与其它 doc 窗一致）
    let app2 = app.clone();
    tauri::async_runtime::spawn_blocking(move || windows::show_long_result(&app2))
        .await
        .map_err(|e| format!("open result window failed: {e}"))??;
    Ok(())
}

/// 结果窗挂载时取展示信息（图片路径 + 尺寸 + 步数）。
#[tauri::command]
pub async fn long_result_info(state: State<'_, AppState>) -> Result<LongResultPayload, String> {
    let guard = state.long.lock().unwrap();
    let session = guard.as_ref().ok_or_else(|| "no long session".to_string())?;
    let path = session
        .preview
        .as_ref()
        .ok_or_else(|| "long result image not ready".to_string())?;
    let canvas = session.capture.canvas();
    Ok(LongResultPayload {
        path: path.to_string_lossy().to_string(),
        width: canvas.width,
        height: canvas.height,
        steps: session.capture.steps(),
    })
}

/// 关掉结果窗并丢弃会话（用户点「完成」或直接关闭窗口）。
#[tauri::command]
pub async fn long_result_close(app: AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    state.long_stop.store(true, Ordering::SeqCst);
    state.long_active.store(false, Ordering::SeqCst);
    // 这里要真正丢弃（覆盖可能残留的保留标记）
    state.long_keep.store(false, Ordering::SeqCst);
    let dropped = state.long.lock().unwrap().take();
    match dropped {
        Some(session) => {
            let _ = std::fs::remove_file(&session.thumb);
            // 直接丢弃即可：LongCapture 的 Drop 会恢复系统光标、还原浮层状态
            drop(session);
            crate::logln!("[snappin] long result closed, session dropped");
        }
        // commit 已经先把会话取走了，这里没有可丢的东西（正常路径）
        None => crate::logln!("[snappin] long result closed (session already consumed)"),
    }
    windows::close_long_result(&app);
    Ok(())
}

#[tauri::command]
pub async fn long_capture_commit(
    app: AppHandle,
    state: State<'_, AppState>,
    action: String,
) -> Result<ConfirmPayload, String> {
    // 先叫停再取：驱动循环可能在当前步的稳态等待里
    state.long_stop.store(true, Ordering::SeqCst);
    let session = { state.long.lock().unwrap().take() };
    let Some(session) = session else {
        crate::logln!("[snappin] long commit({action}) failed: session not found");
        return Err("long capture session not found".to_string());
    };
    let _ = std::fs::remove_file(&session.thumb);
    state.long_keep.store(false, Ordering::SeqCst);
    crate::logln!(
        "[snappin] long commit({action}) start: {}x{}, {} steps",
        session.capture.canvas().width,
        session.capture.canvas().height,
        session.capture.steps()
    );
    let canvas = session.capture.into_canvas();
    restore_long_overlay(
        &app,
        session.overlay_idx,
        session.overlay_hwnd,
        session.used_fallback,
    );
    state.long_active.store(false, Ordering::SeqCst);
    unregister_long_esc(&app);

    let save_dir = { state.config.lock().unwrap().save_dir.clone() };
    let path = snappin_core_platform::new_temp_png_path();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    snappin_core_platform::save_png(&canvas, &path).map_err(|e| e.to_string())?;
    let path_str = path.to_string_lossy().to_string();

    match action.as_str() {
        "copy" => {
            snappin_core_platform::clipboard_copy_image(&canvas).map_err(|e| e.to_string())?;
        }
        "pin" => {
            let label = pin_label(&path);
            // 长图按 70% 屏幅封顶缩放，主屏居中（长图经常远超屏幕）
            let (disp_w, disp_h, x, y) = long_pin_geometry(&app, canvas.width, canvas.height);
            let app2 = app.clone();
            let label2 = label.clone();
            #[cfg(windows)]
            tauri::async_runtime::spawn_blocking(move || {
                windows::create_pin_window(&app2, &label2, disp_w, disp_h, x, y)
            })
            .await
            .map_err(|e| format!("pin window task failed: {e}"))??;
            #[cfg(not(windows))]
            windows::create_pin_window(&app2, &label2, disp_w, disp_h, x, y)?;
            register_pin(&state, &label, &path_str, disp_w, disp_h);
        }
        "save" => {
            // 先结束会话再弹对话框（always-on-top 浮层会盖住系统保存框，
            // 同 confirm_selection 的 save 分支）
            windows::end_capture_session(&app, &state, "long-save-before-dialog");
            let dest = save_dialog(
                &app,
                &snappin_core_platform::default_save_name(),
                save_dir,
            )
            .await;
            if let Some(dest) = dest {
                let buffer =
                    snappin_core_platform::load_rgba(Path::new(&path_str)).map_err(|e| e.to_string())?;
                snappin_core_platform::save_image(&buffer, &dest).map_err(|e| e.to_string())?;
                // 记住此次目录，下次保存对话框默认打开这里
                if let Some(parent) = dest.parent() {
                    let dir_str = parent.to_string_lossy().to_string();
                    let mut guard = state.config.lock().unwrap();
                    if guard.save_dir.as_deref() != Some(dir_str.as_str()) {
                        guard.save_dir = Some(dir_str.clone());
                        let snapshot = guard.clone();
                        drop(guard);
                        if let Ok(dir) = app.path().app_config_dir() {
                            let _ = crate::config::save(&dir, &snapshot);
                        }
                    }
                }
            }
            return Ok(ConfirmPayload { path: path_str });
        }
        other => return Err(format!("unknown action: {}", other)),
    }

    windows::end_capture_session(&app, &state, "long-confirm-ok");
    Ok(ConfirmPayload { path: path_str })
}

/// 手动长截图驱动循环：轮询采集页面，用户停止后收尾。
fn run_long_driver(app: &AppHandle) {
    let state = app.state::<AppState>();
    let mut last_score = 0.0f32;
    let mut last_steps = 0u32;
    let mut error: Option<String> = None;
    let mut reason = "done";

    'outer: for _ in 0..LONG_MAX_STEPS {
        if state.long_stop.load(Ordering::SeqCst) {
            reason = "stopped";
            break;
        }
        // 一步可能耗时 1s 上下（稳态等待 + 两次全屏采集），全程持锁；
        // commit 只在 done 之后发生，不会与这里并发。
        let stepped = {
            let mut guard = state.long.lock().unwrap();
            let Some(session) = guard.as_mut() else {
                // 会话已被 commit 取走（异常路径）：直接退出，收尾由 commit 负责
                return;
            };
            let lc = &mut session.capture;
            let r = lc.step_manual();
            match r {
                Ok(StepOutcome::Stopped) => {
                    reason = "stopped";
                    break 'outer;
                }
                Ok(StepOutcome::Stepped(step)) => {
                    if step.kind == StitchStepKind::Appended {
                        let _ = write_thumbnail(lc.canvas(), &session.thumb);
                    }
                    (step, session.thumb.to_string_lossy().to_string())
                }
                Err(e) => {
                    error = Some(e);
                    break 'outer;
                }
            }
        };
        let (step, thumb) = stepped;
        if step.kind != StitchStepKind::Still {
            crate::logln!(
                "[snappin] long manual step {}: kind={:?} score={:.3} diff={:.2} best_shift={} best_mean={:.4} height={}px",
                step.steps, step.kind, step.score, step.diff, step.best_shift,
                step.best_mean, step.height
            );
        }
        last_score = step.score;
        last_steps = step.steps;
        match step.kind {
            StitchStepKind::Appended => {
                emit_long(
                    app,
                    "scrolling",
                    step.height,
                    step.steps,
                    step.score,
                    &thumb,
                    // 超长模式：降采样后高度上限翻倍，但清晰度下降，必须让用户看到
                    format!(
                        "滚动页面继续截图 · 已 {} px{}",
                        step.height,
                        if step.scale > 1 {
                            format!("（超长模式 · 清晰度 1/{}）", step.scale)
                        } else {
                            String::new()
                        }
                    ),
                );
            }
            StitchStepKind::NoMatch => {
                emit_long(
                    app,
                    "scrolling",
                    step.height,
                    step.steps,
                    step.score,
                    &thumb,
                    "滚动过快，找不到重叠；请向上滚回已截区域",
                );
            }
            StitchStepKind::Rewound => {
                emit_long(
                    app,
                    "scrolling",
                    step.height,
                    step.steps,
                    step.score,
                    &thumb,
                    "当前内容已截取；继续滚动可扩展长图",
                );
            }
            StitchStepKind::Still => {
                // 手动模式由用户明确结束；静止只降低采样频率。
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(
            if step.kind == StitchStepKind::Still { 180 } else { 35 },
        ));
    }

    // 会话可能已被 commit 取走：那就由它收尾，这里什么都不做
    let final_info = {
        let mut guard = state.long.lock().unwrap();
        guard
            .as_mut()
            .map(|s| {
                s.capture.restore_cursor();
                (s.capture.height(), s.thumb.to_string_lossy().to_string())
            })
    };
    let Some((height, thumb)) = final_info else {
        state.long_active.store(false, Ordering::SeqCst);
        // 这条路径（会话已被 commit 取走）同样要注销 Esc，否则会一直占着全局 Esc
        unregister_long_esc(app);
        return;
    };

    let (phase, message) = match &error {
        Some(e) => ("error", format!("长截图出错：{e}")),
        None => (
            "done",
            match reason {
                "stopped" => "已停止".to_string(),
                _ => format!("已完成，共 {height} px"),
            },
        ),
    };
    emit_long(
        app,
        phase,
        height,
        last_steps,
        last_score,
        &thumb,
        &message,
    );
    windows::close_long_preview(app);
    // 诊断：结果面板出现时把屏幕上所有中小窗口打一遍（排查"多出来的灰框"）
    #[cfg(windows)]
    crate::winsnap::log_window_inventory("long-result-shown");

    // 还原浮层 + 注销 Esc（幂等；commit 也会做一遍，双保险）
    let anchor = {
        let guard = state.long.lock().unwrap();
        guard
            .as_ref()
            .map(|s| (s.overlay_idx, s.overlay_hwnd, s.used_fallback))
    };
    if let Some((idx, hwnd, used_fallback)) = anchor {
        restore_long_overlay(app, idx, hwnd, used_fallback);
    }
    state.long_active.store(false, Ordering::SeqCst);
    // **无条件**注销 Esc。
    //
    // 这里原来是"state.long 为空才注销"，而 done/error 之后会话**仍留在 state 里**
    // （要供结果面板的保存/复制/贴图用），条件永远不成立 → 全局 Esc 一直被本程序
    // 注册着。Windows 会把该键交给全局热键、不再传给前台窗口，于是：
    //   浮层的 keydown 收不到 Esc（前端那句 exitCapture 永远不执行）
    //   热键处理器看 long_active=false 也什么都不做
    // 用户实测：结果面板上连按 30+ 次 Esc 全无反应（日志 30+ 条 hotkey pressed）。
    unregister_long_esc(app);
    // 结果面板期间就把悬浮球还回来：万一前端渲染出问题，球是用户唯一的逃生口
    // （点球可重新截图、右键有菜单）。原来只在 end session 时恢复，而结果面板
    // 期间浮层不关，球就一直藏着 —— 用户反馈"悬浮球看不见"。
    windows::restore_ball_after_capture(app);
}

/// 发一条长截图进度事件。
fn emit_long(
    app: &AppHandle,
    phase: &str,
    height: u32,
    steps: u32,
    score: f32,
    thumb: &str,
    message: impl Into<String>,
) {
    let payload = LongProgressPayload {
        phase: phase.to_string(),
        height,
        steps,
        score,
        thumb: thumb.to_string(),
        message: message.into(),
    };
    let _ = app.emit(LONG_EVENT, payload);
}

/// 还原长截图期间改过的浮层状态：光标穿透解除、采集排除解除、
/// 兜底路径隐藏过的浮层显式唤回。
fn restore_long_overlay(app: &AppHandle, idx: usize, hwnd: isize, ensure_visible: bool) {
    let label = format!("{}{}", windows::OVERLAY_PREFIX, idx);
    windows::set_window_ignore_cursor_events(app, &label, false);
    if hwnd != 0 {
        snappin_core_platform::stitch::set_hwnd_capture_excluded(hwnd, false);
    }
    // **无条件**重新显示：拼接期间浮层被 hide_all_overlays 整体收起了，
    // 而结果面板（复制/贴图/保存）就画在浮层里，不重新显示用户就取不到图。
    let _ = ensure_visible; // 参数保留：老调用点仍在传，语义已并入无条件显示
    let _ = overlay_set_visible(app, &label, true);
}

/// 取 NSWindow 的 CGWindowNumber（非 macOS / 失败返回 0）。
///
/// `CGWindowList` 里标识窗口用的就是这个号，跳过"浮层自己"要靠它。
#[cfg(target_os = "macos")]
fn overlay_window_number(w: &tauri::WebviewWindow) -> isize {
    match w.ns_window() {
        Ok(ns) => crate::winsnap::ns_window_number(ns),
        Err(e) => {
            eprintln!("[snappin] ns_window failed for {}: {e}", w.label());
            0
        }
    }
}

/// 取浮层顶层窗口的 Win32 HWND（非 Windows / 失败返回 0）。
#[cfg(windows)]
fn overlay_window_hwnd(app: &AppHandle, label: &str) -> isize {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    let Some(w) = app.get_webview_window(label) else {
        return 0;
    };
    match w.window_handle() {
        Ok(h) => match h.as_raw() {
            RawWindowHandle::Win32(h32) => h32.hwnd.get(),
            _ => 0,
        },
        Err(e) => {
            eprintln!("[snappin] window_handle failed for {label}: {e}");
            0
        }
    }
}

#[cfg(not(windows))]
fn overlay_window_hwnd(_app: &AppHandle, _label: &str) -> isize {
    0
}

/// 在主线程显示/隐藏浮层（同步等待结果）。
fn overlay_set_visible(app: &AppHandle, label: &str, visible: bool) -> Result<(), String> {
    let (tx, rx) = mpsc::channel::<Result<(), String>>();
    let label = label.to_string();
    let app_cb = app.clone();
    let _ = app.run_on_main_thread(move || {
        let r = match app_cb.get_webview_window(&label) {
            Some(w) => {
                if visible {
                    w.show().map_err(|e| e.to_string())
                } else {
                    w.hide().map_err(|e| e.to_string())
                }
            }
            None => Ok(()),
        };
        let _ = tx.send(r);
    });
    rx.recv_timeout(Duration::from_secs(2))
        .unwrap_or_else(|_| Err("overlay visibility: main thread timeout".to_string()))
}

/// 长截图期间注册全局 Esc（拦截系统级按键；结束必须注销）。
/// 与其它动态热键同一模式：动态注册、按时注销，不常驻——
/// 后台工具常驻拦截无修饰键 Esc 会打扰其它程序。
fn register_long_esc(app: &AppHandle) {
    let esc = crate::long_esc_shortcut();
    windows::post_to_main(app, move |app| {
        if let Err(e) = app.global_shortcut().register(esc) {
            eprintln!("[snappin] register Esc for long capture failed: {e}");
        }
    });
}

/// 注销长截图的全局 Esc（幂等，未注册时返回错误，忽略）。
fn unregister_long_esc(app: &AppHandle) {
    let esc = crate::long_esc_shortcut();
    windows::post_to_main(app, move |app| {
        let _ = app.global_shortcut().unregister(esc);
    });
}

/// 贴图几何：长图逻辑尺寸按 70% 主屏幅面封顶缩放，主屏居中。
fn long_pin_geometry(app: &AppHandle, w: u32, h: u32) -> (f64, f64, f64, f64) {
    let (mon_w, mon_h, sf) = app
        .primary_monitor()
        .ok()
        .flatten()
        .map(|m| {
            let s = m.scale_factor();
            let sz = m.size();
            (sz.width as f64 / s, sz.height as f64 / s, s)
        })
        .unwrap_or((1920.0, 1080.0, 1.0));
    let dw = (w as f64 / sf).max(8.0);
    let dh = (h as f64 / sf).max(8.0);
    let scale = ((mon_w * LONG_PIN_MAX_RATIO) / dw)
        .min((mon_h * LONG_PIN_MAX_RATIO) / dh)
        .min(1.0);
    let dw = dw * scale;
    let dh = dh * scale;
    let (x, y) = windows::place_centered_on_primary(app, dw, dh);
    (dw, dh, x, y)
}


// ---------- OCR 结果窗 / 翻译窗 / 设置窗 ----------

/// 打开 OCR 结果窗（识别成功后由浮层调用）。
///
/// 行数据先存进 state，再建/前置窗口；窗口（含已存在的旧窗）通过
/// `ocr-open` 事件重新拉取，避免"第二次识别后窗口内容不刷新"。
///
/// 线程模型（Windows 实测教训，别再改坏）：
/// - 命令本身 **async**：跑在 async runtime，不在主线程的 webview IPC 回调里
///   （曾经的 bug：同步命令在主线程 IPC 回调里建窗 → 整个应用卡死）
/// - **所有窗口操作投递主线程**并同步等结果：建窗要枚举显示器、关窗要走
///   消息循环，非主线程做会挂；而主线程此时是空闲的（IPC 回调已返回），
///   所以不会堵死它
/// - 顺序：先关浮层再开结果窗。反序会让结果窗盖住浮层工具条并抢焦点，
///   用户看到的是"点了识别没反应、X/ESC 也没反应"
#[tauri::command]
pub async fn ocr_show(app: AppHandle, lines: Vec<OcrLinePayload>) -> Result<(), String> {
    crate::logln!("ocr_show: {} lines", lines.len());
    *app.state::<AppState>().ocr_result.lock().unwrap() = lines;
    let (tx, rx) = std::sync::mpsc::channel::<Result<(), String>>();
    let app_cb = app.clone();
    let _ = app.run_on_main_thread(move || {
        let state = app_cb.state::<AppState>();
        // 失败路径不走这里（recognize 直接 reject），浮层保持原样供重试
        windows::end_capture_session(&app_cb, &state, "ocr-done");
        let r = windows::show_ocr_window(&app_cb);
        let _ = tx.send(r);
    });
    let r = rx
        .recv_timeout(std::time::Duration::from_secs(10))
        .map_err(|_| "ocr window: main thread timeout (10s)".to_string())?;
    r?;
    let _ = app.emit("ocr-open", ());
    Ok(())
}

/// OCR 结果窗的一次完整内容：识别行 + 原图路径（左栏显示用）。
#[derive(Clone, serde::Serialize)]
pub struct OcrResultPayload {
    pub lines: Vec<OcrLinePayload>,
    /// 裁剪后 PNG 的绝对路径；None = 还没识别过
    pub image: Option<String>,
}

/// 读取最近一次 OCR 结果（窗口挂载 / 收到 ocr-open 时调用）。
#[tauri::command]
pub fn ocr_result(app: AppHandle) -> OcrResultPayload {
    // 先取句柄再分别 lock：`app.state()` 的临时 State 不能跨字段借用
    let state = app.state::<AppState>();
    let lines = state.ocr_result.lock().unwrap().clone();
    let image = state.ocr_image.lock().unwrap().clone();
    OcrResultPayload { lines, image }
}

/// 悬停吸附：返回光标处应高亮的窗口或子区域（逻辑坐标 + 标题）。
///
/// 前端在"还没出选区"时按 ~15fps 调它，用于高亮鼠标下的窗口；
/// 单击即选中高亮区域（对标 QQ 截图 / 微信截图的识窗行为）。
/// 返回 None 表示光标下只有我们自己的窗口（浮层/悬浮球/菜单），不高亮。
///
/// 实现细节与坑见 `winsnap` 模块：浮层自己是全屏窗口，`WindowFromPoint`
/// 只会命中浮层，必须沿 z 序往下找第一个"别人家的、可见的、包含该点"的窗。
#[tauri::command]
pub fn window_snap(
    app: AppHandle,
    state: State<'_, AppState>,
    monitor_id: String,
    x: f64,
    y: f64,
) -> Option<crate::winsnap::WindowSnap> {
    // 只跳过**截图浮层自己**，不按进程过滤。
    // 早期按"本进程"过滤 → 光标停在设置窗上时，设置窗被跳过，
    // 扫描继续往下找到它背后的程序，高亮就跑偏了（用户实测反馈）。
    #[cfg(windows)]
    let skip: Vec<isize> = app
        .webview_windows()
        .keys()
        .filter(|label| label.starts_with(crate::windows::OVERLAY_PREFIX))
        .map(|label| overlay_window_hwnd(&app, label))
        .filter(|h| *h != 0)
        .collect();
    // macOS 用 CGWindowNumber 标识窗口（见 winsnap_mac.rs）
    #[cfg(target_os = "macos")]
    let skip: Vec<isize> = app
        .webview_windows()
        .iter()
        .filter(|(label, _)| label.starts_with(crate::windows::OVERLAY_PREFIX))
        .map(|(_, w)| overlay_window_number(w))
        .filter(|n| *n != 0)
        .collect();
    #[cfg(not(any(windows, target_os = "macos")))]
    let skip: Vec<isize> = Vec::new();

    // 只在"吸附结果变了"时打日志（含变成 none）：前端 ~15fps 调用，
    // 每次都打会淹没日志；但只记"找到了"又分不清"没调用"和"没命中"。
    static LAST: std::sync::Mutex<Option<Option<String>>> = std::sync::Mutex::new(None);
    let snap = (|| {
        let monitors = state.monitors.lock().unwrap();
        let m = monitors.iter().find(|m| m.id == monitor_id)?;
        let sf = m.scale_factor as f64;
        // `m.x/m.y` 的**单位随平台不同，各平台实现自己解释**（见 winsnap.rs）：
        // - Windows：物理像素，platform::snap_at 内部按 `mon + local * sf` 换算
        // - macOS：点（CGDisplayBounds），winsnap_mac 直接 `mon + local`，不乘 sf
        // 所以这里原样传 `m.x`，不要提前除/乘 scale_factor。
        let mut s = crate::winsnap::snap_at(x, y, m.x as f64, m.y as f64, sf, &skip)?;
        // 窗口可能跨屏：钳进当前显示器（浮层逻辑坐标以本显示器左上为原点）
        let lw = m.width as f64 / sf;
        let lh = m.height as f64 / sf;
        crate::winsnap::clamp_to_monitor(&mut s, 0.0, 0.0, lw, lh);
        Some(s)
    })();
    {
        let cur = snap.as_ref().map(|s| {
            format!("{}:{:.0},{:.0},{:.0},{:.0}", s.title, s.x, s.y, s.w, s.h)
        });
        let mut last = LAST.lock().unwrap();
        let changed = match last.as_ref() {
            None => true,
            Some(prev) => prev != &cur,
        };
        if changed {
            match (&cur, &snap) {
                (Some(_), Some(s)) => crate::logln!(
                    "snap target: {:?} {}x{} at ({}, {})",
                    s.title,
                    s.w.round(),
                    s.h.round(),
                    s.x.round(),
                    s.y.round()
                ),
                _ => crate::logln!("snap target: none (cursor={x:.0},{y:.0})"),
            }
            *last = Some(cur);
        }
    }
    snap
}

/// 关闭 OCR 结果窗（内容保留，下次打开还在）。
#[tauri::command]
pub fn ocr_close(app: AppHandle) {
    windows::close_doc_window(&app, windows::OCR_LABEL);
}

/// 开始翻译：先开窗（用户立刻看到"翻译中"），再到阻塞线程里调 API。
///
/// 设计要点：
/// - **同步返回、异步出结果**：翻译是秒级网络操作，不能卡住调用方
///   （OCR 窗点「翻译」后要能继续操作）；结果走 `trans-update` 事件推送。
/// - **状态集中在 state.trans**：窗口销毁不丢上下文，重开窗能接着看。
/// - 重复调用会取消上一次吗？不会——但 `busy` 期间前端禁用按钮，
///   且后到的结果会覆盖先到的（正常单用户场景不会并发）。
#[tauri::command]
pub async fn trans_start(app: AppHandle, text: String) -> Result<(), String> {
    {
        let state = app.state::<AppState>();
        let mut t = state.trans.lock().unwrap();
        t.original = text;
        t.translated.clear();
        t.error.clear();
        t.busy = true;
        let tr = state.config.lock().unwrap().translation.clone();
        t.target_lang = tr.target_lang.clone();
        t.model_name = tr
            .active()
            .map(|m| if m.name.is_empty() { m.model.clone() } else { m.name.clone() })
            .unwrap_or_default();
    }
    // 窗口操作投递主线程（Windows 上非主线程建窗会挂，见 ocr_show 的线程模型说明）
    let (tx, rx) = std::sync::mpsc::channel::<Result<(), String>>();
    let app_cb = app.clone();
    let _ = app.run_on_main_thread(move || {
        let _ = tx.send(windows::show_trans_window(&app_cb));
    });
    rx.recv_timeout(std::time::Duration::from_secs(10))
        .map_err(|_| "trans window: main thread timeout (10s)".to_string())?
        ?;
    let _ = app.emit("trans-update", snapshot_trans(&app));

    let app2 = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let state = app2.state::<AppState>();
        // 配置快照：翻译期间用户改设置不影响本次请求（也避免持锁跨网络调用）
        let cfg = state.config.lock().unwrap().translation.clone();
        let outcome = snappin_core_platform::translate::translate(&cfg, &{
            let t = state.trans.lock().unwrap();
            t.original.clone()
        });
        {
            let mut t = state.trans.lock().unwrap();
            t.busy = false;
            if outcome.ok {
                t.translated = outcome.translated;
                t.error.clear();
            } else {
                t.translated.clear();
                t.error = outcome.error;
            }
        }
        let _ = app2.emit("trans-update", snapshot_trans(&app2));
    });
    Ok(())
}

/// 测试某个模型的连通性（设置窗「模型」页的「测试连接」）。
///
/// 网络操作同样放阻塞线程；成功/失败都转成中文短文案，前端在行内显示。
#[tauri::command]
pub async fn trans_test(app: AppHandle, id: String) -> Result<(), String> {
    let app2 = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let cfg = app2.state::<AppState>().config.lock().unwrap().translation.clone();
        snappin_core_platform::translate::test_connection(&cfg, &id)
    })
    .await
    .map_err(|e| format!("测试连接任务失败：{e}"))?
}

/// 翻译窗当前状态（前端挂载时拉一次，之后靠 trans-update 事件）。
#[tauri::command]
pub fn trans_state(app: AppHandle) -> crate::state::TransState {
    snapshot_trans(&app)
}

/// 关闭翻译窗（状态保留，下次打开接着看）。
#[tauri::command]
pub fn trans_close(app: AppHandle) {
    windows::close_doc_window(&app, windows::TRANS_LABEL);
}

/// 翻译状态快照（加锁克隆，避免把 MutexGuard 传出去）。
fn snapshot_trans(app: &AppHandle) -> crate::state::TransState {
    app.state::<AppState>().trans.lock().unwrap().clone()
}

/// 打开设置窗（托盘菜单 / 翻译失败时的"去设置"都用它）。
///
/// 同 `ocr_show`：必须 async，避免在主线程的 webview IPC 回调里建窗死锁。
/// 把某张贴图置前（管理面板点缩略图）。
#[tauri::command]
pub async fn pin_focus(app: AppHandle, state: State<'_, AppState>, label: String) -> Result<(), String> {
    if !state.pins.lock().unwrap().contains_key(&label) {
        return Err("pin window not found".to_string());
    }
    let (tx, rx) = std::sync::mpsc::channel::<Result<(), String>>();
    let app_cb = app.clone();
    let label_cb = label.clone();
    let _ = app.run_on_main_thread(move || {
        let r = (|| -> Result<(), String> {
            let w = app_cb
                .get_webview_window(&label_cb)
                .ok_or_else(|| "pin window not found".to_string())?;
            let _ = w.show();
            let _ = w.unminimize();
            let _ = w.set_focus();
            Ok(())
        })();
        let _ = tx.send(r);
    });
    rx.recv_timeout(std::time::Duration::from_secs(5))
        .map_err(|_| "pin focus: main thread timeout (5s)".to_string())?
}

/// 一条贴图记录（贴图管理面板用）。
#[derive(Debug, Clone, serde::Serialize)]
pub struct PinInfoPayload {
    pub label: String,
    /// 当前显示的图片路径（可能指向处理后的临时文件）
    pub src: String,
    pub width: f64,
    pub height: f64,
    pub group: String,
}

/// 列出所有贴图（按加入顺序稳定排序，面板不会每次刷新跳位）。
#[tauri::command]
pub async fn pin_list(state: State<'_, AppState>) -> Result<Vec<PinInfoPayload>, String> {
    let pins = state.pins.lock().unwrap();
    let mut out: Vec<PinInfoPayload> = pins
        .iter()
        .map(|(label, m)| PinInfoPayload {
            label: label.clone(),
            src: m.src.clone(),
            width: m.width,
            height: m.height,
            group: m.group.clone(),
        })
        .collect();
    out.sort_by(|a, b| a.label.cmp(&b.label));
    Ok(out)
}

/// 打开贴图管理面板。
///
/// 与设置窗同一条铁律：菜单/托盘事件里**不能同步建窗**（Windows 会死锁），
/// 必须投递主线程并同步等结果。
#[tauri::command]
pub async fn open_pin_manager(app: AppHandle) -> Result<(), String> {
    let (tx, rx) = std::sync::mpsc::channel::<Result<(), String>>();
    let app_cb = app.clone();
    let _ = app.run_on_main_thread(move || {
        let _ = tx.send(windows::show_pin_manager(&app_cb));
    });
    rx.recv_timeout(std::time::Duration::from_secs(10))
        .map_err(|_| "pin manager: main thread timeout (10s)".to_string())?
}

#[tauri::command]
pub async fn open_settings_window(app: AppHandle) -> Result<(), String> {
    let (tx, rx) = std::sync::mpsc::channel::<Result<(), String>>();
    let app_cb = app.clone();
    let _ = app.run_on_main_thread(move || {
        let _ = tx.send(windows::show_main_window(&app_cb));
    });
    rx.recv_timeout(std::time::Duration::from_secs(10))
        .map_err(|_| "settings window: main thread timeout (10s)".to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine as _;

    /// 选区载荷必须接受**小数**逻辑坐标。
    ///
    /// 回归：macOS Retina（scale=2）与 Windows 125%/150% 缩放下，逻辑坐标
    /// 天然带小数。`RectPayload` 曾是 `i32/u32`，serde 在反序列化阶段就拒收
    /// （`invalid type: floating point '484.71484375', expected u32`），
    /// 用户点确认后只看到"复制失败"，且**只在高缩放的机器上**才复现。
    #[test]
    fn rect_payload_accepts_fractional_coords() {
        let json = r#"{"x":484.71484375,"y":120.5,"w":1415.25,"h":969.75}"#;
        let r: RectPayload = serde_json::from_str(json).expect("fractional rect must parse");
        assert!((r.x - 484.71484375).abs() < 1e-9);
        assert!((r.h - 969.75).abs() < 1e-9);
        // 整数形态（100% 缩放）向后兼容，不能因为改成浮点就收不了
        let r2: RectPayload = serde_json::from_str(r#"{"x":0,"y":0,"w":800,"h":600}"#).unwrap();
        assert_eq!((r2.x, r2.y, r2.w, r2.h), (0.0, 0.0, 800.0, 600.0));
    }

    /// 逻辑坐标 → 物理像素的舍入口径：2x 屏上 0.5 逻辑像素 = 1 物理像素，
    /// 四舍五入后不能出现 0 宽/0 高（否则裁剪直接 panic）。
    #[test]
    fn fractional_rect_scales_to_valid_pixels() {
        let sf = 2.0_f64;
        let (x, y, w, h) = (484.71484375, 120.5, 1415.25, 969.75);
        assert_eq!((x * sf).round() as i32, 969);
        assert_eq!((y * sf).round() as i32, 241);
        assert_eq!((w * sf).round().max(1.0) as u32, 2831);
        assert_eq!((h * sf).round().max(1.0) as u32, 1940);
        // 极小选区（0.3 逻辑像素）在 1x 屏上仍必须 >= 1
        let tiny = (0.3_f64 * 1.0).round().max(1.0) as u32;
        assert_eq!(tiny, 1);
    }

    /// 显示器 ID 解析：错一位就会拼错屏幕的内容，必须严格。
    #[test]
    fn monitor_id_parses_to_index() {
        assert_eq!(monitor_index("m0").unwrap(), 0);
        assert_eq!(monitor_index("m1").unwrap(), 1);
        assert_eq!(monitor_index("m10").unwrap(), 10);
        assert!(monitor_index("x0").is_err());
        assert!(monitor_index("m").is_err());
        assert!(monitor_index("m-1").is_err());
        assert!(monitor_index("m1.5").is_err());
        assert!(monitor_index("").is_err());
    }

    /// **IPC 契约回归**：`confirm_selection` 的 `shapes` 直接来自前端
    /// `src/annotate/tools.ts` 的 `ShapeDraft`（camelCase 字段 + 各工具的 `type` 判别值）。
    ///
    /// 这类不匹配只在运行时才炸，用户看到的是
    /// `invalid args shapes for command confirm_selection: missing field step_x`
    /// 这种完全指不出根因的错误，所以只有单测能兜住。已踩过两次：
    /// 1. 枚举只写了 `rename_all`（作用变体名），漏了 `rename_all_fields`
    ///    （作用字段名）——单词字段恰好同名掩盖了问题，水印的 `stepX/stepY` 一用就炸；
    /// 2. `ZoomRegion` 变体名按 camelCase 是 `zoomRegion`，而前端发的是 `zoom`。
    #[test]
    fn shape_payload_matches_frontend_tool_payloads() {
        let png = sample_png_b64();
        let cases: Vec<(&str, String)> = vec![
            (
                "rect",
                r#"{"type":"rect","x":1,"y":2,"w":3,"h":4,"color":[255,0,0,255],"width":2}"#
                    .into(),
            ),
            (
                "ellipse",
                r#"{"type":"ellipse","x":1,"y":2,"w":3,"h":4,"color":[255,0,0,255],"width":2}"#
                    .into(),
            ),
            (
                "arrow",
                r#"{"type":"arrow","x1":1,"y1":2,"x2":3,"y2":4,"color":[255,0,0,255],"width":2}"#
                    .into(),
            ),
            (
                "pen",
                r#"{"type":"pen","points":[[1,2],[3,4]],"color":[255,0,0,255],"width":2}"#.into(),
            ),
            (
                "highlighter",
                r#"{"type":"highlighter","points":[[1,2]],"color":[255,0,0,255],"width":2}"#.into(),
            ),
            (
                "mosaic",
                r#"{"type":"mosaic","x":1,"y":2,"w":3,"h":4,"block":8}"#.into(),
            ),
            (
                "spotlight",
                r#"{"type":"spotlight","x":1,"y":2,"w":3,"h":4,"radius":6,"dim":128}"#.into(),
            ),
            (
                "bitmap",
                // 前端还会带 w/h（预览用），Rust 侧忽略未知字段
                format!(r#"{{"type":"bitmap","x":1,"y":2,"w":3,"h":4,"png":"{png}"}}"#),
            ),
            (
                "eraser",
                r#"{"type":"eraser","points":[[1,2]],"radius":3}"#.into(),
            ),
            (
                "watermark",
                format!(
                    r#"{{"type":"watermark","x":1,"y":2,"w":30,"h":40,"stepX":25,"stepY":35,"png":"{png}"}}"#
                ),
            ),
            (
                "zoom",
                r#"{"type":"zoom","x":1,"y":2,"w":3,"h":4,"zoom":2,"color":[0,0,0,255],"width":2}"#
                    .into(),
            ),
            (
                "smartErase",
                r#"{"type":"smartErase","x":1,"y":2,"w":3,"h":4,"iterations":0}"#.into(),
            ),
        ];

        for (name, json) in cases {
            let parsed: ShapePayload = serde_json::from_str(&json)
                .unwrap_or_else(|e| panic!("[{name}] 前端载荷解析失败：{e}\n载荷：{json}"));

            // 回写方向同样要求 camelCase（将来前端若"回读"不必猜字段名）
            let value = serde_json::to_value(&parsed).expect("serialize");
            if let Some(obj) = value.as_object() {
                for key in obj.keys() {
                    assert!(
                        !key.contains('_'),
                        "[{name}] 字段名须为 camelCase，出现了 {key}"
                    );
                }
            }
        }
    }

    /// 2x1 的 PNG（左红右绿），base64 编码——模拟前端 `toDataURL` 的尾巴
    fn sample_png_b64() -> String {
        let mut img = image::RgbaImage::new(2, 1);
        img.put_pixel(0, 0, image::Rgba([255, 0, 0, 255]));
        img.put_pixel(1, 0, image::Rgba([0, 255, 0, 255]));
        let mut buf = Vec::new();
        image::DynamicImage::ImageRgba8(img)
            .write_to(&mut std::io::Cursor::new(&mut buf), image::ImageFormat::Png)
            .expect("encode png");
        base64::engine::general_purpose::STANDARD.encode(buf)
    }

    #[test]
    fn decodes_base64_png() {
        let (w, h, rgba) = decode_png_base64(&sample_png_b64()).expect("decode");
        assert_eq!((w, h), (2, 1));
        assert_eq!(&rgba[0..4], &[255, 0, 0, 255]);
        assert_eq!(&rgba[4..8], &[0, 255, 0, 255]);
    }

    #[test]
    fn decoded_points_scale_back_to_logical_pixels() {
        let codes = vec![snappin_core_platform::qr::DecodedCode {
            text: "https://example.com".to_string(),
            format: "QR_CODE".to_string(),
            // 物理像素：1.75 缩放的 4K 屏上，逻辑 (100,200) → 物理 (175,350)
            points: vec![(175.0, 350.0), (525.0, 350.0)],
        }];
        let out = to_decoded_payloads(codes, 1.75);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].text, "https://example.com");
        assert_eq!(out[0].format, "QR_CODE");
        assert_eq!(out[0].points, vec![(100.0, 200.0), (300.0, 200.0)]);
    }

    #[test]
    fn decoded_points_unchanged_at_unity_scale() {
        // 100% 缩放的显示器：换算必须是恒等，不能因为浮点误差抖出 0.9999
        let codes = vec![snappin_core_platform::qr::DecodedCode {
            text: "x".to_string(),
            format: "QR_CODE".to_string(),
            points: vec![(10.5, 20.25)],
        }];
        let out = to_decoded_payloads(codes, 1.0);
        assert_eq!(out[0].points, vec![(10.5, 20.25)]);
    }

    fn sample_ocr_line(min_x: f32, min_y: f32, max_x: f32, max_y: f32) -> snappin_core_platform::ocr::OcrLine {
        snappin_core_platform::ocr::OcrLine {
            text: "测试文字".to_string(),
            score: 0.95,
            box_: snappin_core_algo::ocr::DetBox {
                min_x,
                min_y,
                max_x,
                max_y,
                score: 0.9,
            },
        }
    }

    #[test]
    fn ocr_boxes_are_selection_relative_logical_pixels() {
        // 裁剪图原点是显示器物理像素 (350, 700)，sf=1.75；
        // 框在裁剪图物理坐标 (525, 1050)-(875, 1050) → 选区内逻辑 (100, 200)-(300, 200)
        let lines = vec![sample_ocr_line(525.0, 1050.0, 875.0, 1050.0)];
        let out = to_ocr_payloads(lines, 1.75, 350, 700);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].text, "测试文字");
        assert_eq!(out[0].min_x, 100.0);
        assert_eq!(out[0].min_y, 200.0);
        assert_eq!(out[0].max_x, 300.0);
        assert_eq!(out[0].max_y, 200.0);
    }

    #[test]
    fn ocr_boxes_unchanged_at_unity_scale_and_zero_origin() {
        let lines = vec![sample_ocr_line(10.0, 20.0, 110.0, 45.0)];
        let out = to_ocr_payloads(lines, 1.0, 0, 0);
        assert_eq!((out[0].min_x, out[0].min_y), (10.0, 20.0));
        assert_eq!((out[0].max_x, out[0].max_y), (110.0, 45.0));
    }

    #[test]
    fn ocr_boxes_never_go_negative() {
        // 选区原点不一定与框原点严格对齐（取整误差），负值夹到 0，
        // 否则前端会把高亮框画到选区外（被 overflow 裁掉）
        let lines = vec![sample_ocr_line(-3.0, -1.0, 10.0, 10.0)];
        let out = to_ocr_payloads(lines, 1.0, 5, 5);
        assert_eq!((out[0].min_x, out[0].min_y), (0.0, 0.0));
    }

    #[test]
    fn accepts_data_url_prefix_and_rejects_garbage() {
        let with_prefix = format!("data:image/png;base64,{}", sample_png_b64());
        assert!(decode_png_base64(&with_prefix).is_ok());
        assert!(decode_png_base64("").is_err());
        assert!(decode_png_base64("!!!not-base64!!!").is_err());
        // 合法 base64 但不是 PNG
        assert!(decode_png_base64("aGVsbG8=").is_err());
    }

    #[test]
    fn bitmap_payload_scales_anchor_but_keeps_pixels() {
        let p = ShapePayload::Bitmap { x: 10.0, y: 20.0, png: sample_png_b64() };
        match p.to_shape(2.0).expect("to_shape") {
            snappin_core_algo::shapes::Shape::Bitmap { x, y, w, h, rgba } => {
                assert_eq!((x, y), (20, 40), "锚点应乘 DPR");
                assert_eq!((w, h), (2, 1), "位图像素不应再缩放");
                assert_eq!(rgba.len(), 8);
            }
            other => panic!("expect bitmap, got {other:?}"),
        }
    }

    #[test]
    fn bad_bitmap_payload_fails_instead_of_silently_dropping() {
        let p = ShapePayload::Bitmap { x: 0.0, y: 0.0, png: "broken".into() };
        assert!(p.to_shape(1.0).is_err());
    }

    #[test]
    fn spotlight_payload_scales_geometry_but_not_dim() {        let p = ShapePayload::Spotlight {
            x: 1.0,
            y: 2.0,
            w: 30.0,
            h: 10.0,
            radius: 6.0,
            dim: 150,
        };
        match p.to_shape(2.0).expect("to_shape") {
            snappin_core_algo::shapes::Shape::Spotlight { x, y, w, h, radius, dim } => {
                // 几何量乘 DPR，压暗强度是颜色语义、绝不能乘
                assert_eq!((x, y, w, h, radius, dim), (2, 4, 60, 20, 12, 150));
            }
            other => panic!("expect spotlight, got {other:?}"),
        }
    }

    /// 端到端：载荷 → to_shapes → draw_shapes 后，像素必须真的被压暗。
    /// （单测覆盖过 spotlight_region 本身，这里锁的是"从 IPC 载荷到最终图"
    ///   这条链路——也就是用户保存出来的那张图。）
    #[test]
    fn spotlight_payload_actually_dims_saved_pixels() {
        let (w, h) = (120u32, 80u32);
        let mut data = vec![255u8; w as usize * h as usize * 4];
        for px in data.chunks_exact_mut(4) {
            px[3] = 255;
        }
        let mut buf = snappin_core_algo::RgbaBuffer::new(w, h, data).expect("buffer");

        let payloads = vec![ShapePayload::Spotlight {
            x: 40.0,
            y: 25.0,
            w: 40.0,
            h: 30.0,
            radius: 8.0,
            dim: 171,
        }];
        let shapes = to_shapes(&payloads, 1.0).expect("to_shapes");
        snappin_core_algo::draw::draw_shapes(&mut buf, &shapes);

        // 洞内保持纯白
        assert_eq!(buf.pixel(60, 40), Some([255, 255, 255, 255]), "洞内不该被压暗");
        // 角落压到 255 × (1 - 171/255) ≈ 84
        let corner = buf.pixel(1, 1).expect("corner");
        assert!(
            (corner[0] as i32 - 84).abs() <= 2,
            "角落应压到约 84，实际 {corner:?}"
        );
        assert_eq!(corner[3], 255, "alpha 不该被改动");
    }

    // ---- M3 新增载荷：橡皮擦 / 水印 / 放大镜 / 智能擦除 ----

    #[test]
    fn eraser_payload_scales_points_and_radius() {
        let p = ShapePayload::Eraser {
            points: vec![[10.0, 20.0], [30.0, 40.0]],
            radius: 3.0,
        };
        match p.to_shape(2.0).expect("to_shape") {
            snappin_core_algo::shapes::Shape::Eraser { points, radius } => {
                assert_eq!(points, vec![(20.0, 40.0), (60.0, 80.0)]);
                assert_eq!(radius, 6.0);
            }
            other => panic!("expect eraser, got {other:?}"),
        }
    }

    #[test]
    fn watermark_payload_scales_geometry_and_step() {
        let p = ShapePayload::Watermark {
            x: 5.0,
            y: 6.0,
            w: 20.0,
            h: 10.0,
            step_x: 7.0,
            step_y: 9.0,
            png: sample_png_b64(),
        };
        match p.to_shape(2.0).expect("to_shape") {
            snappin_core_algo::shapes::Shape::Watermark {
                x, y, w, h, step_x, step_y, tile_w, tile_h, tile,
            } => {
                // 几何量与步进都乘 DPR（步进不乘，高 DPI 下水印会挤成一团）
                assert_eq!((x, y, w, h), (10, 12, 40, 20));
                assert_eq!((step_x, step_y), (14, 18));
                // tile 尺寸与像素来自 PNG 解码，不再缩放
                assert_eq!((tile_w, tile_h), (2, 1));
                assert_eq!(tile.len(), 8);
            }
            other => panic!("expect watermark, got {other:?}"),
        }
    }

    #[test]
    fn zoom_payload_keeps_multiplier_and_scales_geometry() {
        let p = ShapePayload::ZoomRegion {
            x: 1.0,
            y: 2.0,
            w: 8.0,
            h: 6.0,
            zoom: 3.0,
            color: [0, 0, 0, 255],
            width: 2.0,
        };
        match p.to_shape(2.0).expect("to_shape") {
            snappin_core_algo::shapes::Shape::ZoomRegion { x, y, w, h, zoom, style } => {
                assert_eq!((x, y, w, h), (2, 4, 16, 12));
                // 倍率是"看起来放大几倍"，与 DPI 无关，绝不能乘
                assert_eq!(zoom, 3.0);
                // 边框线宽是几何量，要乘
                assert_eq!(style.width, 4.0);
            }
            other => panic!("expect zoom, got {other:?}"),
        }
    }

    #[test]
    fn smart_erase_payload_scales_geometry_only() {
        let p = ShapePayload::SmartErase {
            x: 3.0,
            y: 4.0,
            w: 10.0,
            h: 8.0,
            iterations: 5,
        };
        match p.to_shape(2.0).expect("to_shape") {
            snappin_core_algo::shapes::Shape::SmartErase { x, y, w, h, iterations } => {
                assert_eq!((x, y, w, h), (6, 8, 20, 16));
                // 轮数上限是算法参数，不是几何量
                assert_eq!(iterations, 5);
            }
            other => panic!("expect smartErase, got {other:?}"),
        }
    }

    /// 端到端：四个新载荷 → to_shapes → draw_shapes_with_base 后，
    /// 保存图上的像素必须真的按各自语义变化（而不是被静默忽略）。
    /// 单测覆盖过算法本身，这里锁的是"从 IPC 载荷到最终图"这条链路。
    #[test]
    fn new_payloads_actually_change_saved_pixels() {
        // base：水平灰度渐变（灰度 = x*6），左右可区分、放大后可验证取样
        let (w, h) = (40u32, 40u32);
        let mut base_data = vec![0u8; w as usize * h as usize * 4];
        for y in 0..h {
            for x in 0..w {
                let i = (y * w + x) as usize * 4;
                let v = (x as u8).wrapping_mul(6);
                base_data[i] = v;
                base_data[i + 1] = v;
                base_data[i + 2] = v;
                base_data[i + 3] = 255;
            }
        }
        let base = snappin_core_algo::RgbaBuffer::new(w, h, base_data).expect("base");

        // ① 橡皮擦：先画盖住 (10,10)-(20,20) 的红矩形，再擦中心 → 露出 base
        let mut buf = base.clone();
        let payloads = vec![
            ShapePayload::Rect {
                x: 10.0,
                y: 10.0,
                w: 10.0,
                h: 10.0,
                color: [255, 0, 0, 255],
                width: 2.0,
            },
            ShapePayload::Eraser {
                points: vec![[15.0, 15.0]],
                radius: 3.0,
            },
        ];
        let shapes = to_shapes(&payloads, 1.0).expect("to_shapes");
        snappin_core_algo::draw::draw_shapes_with_base(&mut buf, &base, &shapes);
        assert_eq!(buf.pixel(15, 15), Some([90, 90, 90, 255]), "擦除处应还原为 base 渐变");
        assert_eq!(buf.pixel(10, 10), Some([255, 0, 0, 255]), "矩形其余部分应保留");

        // ② 放大镜：放大 (2,2,12,12) zoom=2 → 内部取样 base（黑画布上应出现渐变灰）
        let mut buf2 = snappin_core_algo::RgbaBuffer::new(
            w,
            h,
            {
                let mut d = vec![0u8; w as usize * h as usize * 4];
                for px in d.chunks_exact_mut(4) {
                    px[3] = 255;
                }
                d
            },
        )
        .expect("buf");
        let payloads = vec![ShapePayload::ZoomRegion {
            x: 2.0,
            y: 2.0,
            w: 12.0,
            h: 12.0,
            zoom: 2.0,
            color: [255, 0, 0, 255],
            width: 2.0,
        }];
        let shapes = to_shapes(&payloads, 1.0).expect("to_shapes");
        snappin_core_algo::draw::draw_shapes_with_base(&mut buf2, &base, &shapes);
        // (8,8) 在描边之内：源坐标 = 2 + (8-2)/2 = 5 → base 灰度 5*6 = 30
        assert_eq!(buf2.pixel(8, 8), Some([30, 30, 30, 255]), "放大镜应取样 base 内容");
        assert_eq!(buf2.pixel(0, 0), Some([0, 0, 0, 255]), "区域外不该被动到");

        // ③ 智能擦除：白底中间 8x8 涂黑 → 擦除后还原为白
        let mut white = vec![255u8; w as usize * h as usize * 4];
        for px in white.chunks_exact_mut(4) {
            px[3] = 255;
        }
        let flat = snappin_core_algo::RgbaBuffer::new(w, h, white).expect("flat");
        let mut buf3 = flat.clone();
        for y in 16..24 {
            for x in 16..24 {
                let i = (y * w + x) as usize * 4;
                buf3.data[i] = 0;
                buf3.data[i + 1] = 0;
                buf3.data[i + 2] = 0;
            }
        }
        let payloads = vec![ShapePayload::SmartErase {
            x: 16.0,
            y: 16.0,
            w: 8.0,
            h: 8.0,
            iterations: 0,
        }];
        let shapes = to_shapes(&payloads, 1.0).expect("to_shapes");
        snappin_core_algo::draw::draw_shapes_with_base(&mut buf3, &flat, &shapes);
        assert_eq!(buf3.pixel(20, 20), Some([255, 255, 255, 255]), "智能擦除应补全为白");

        // ④ 水印：2x1（左红右绿）tile 按步进 2 平铺进 (0,0,6,4) → 网格落位
        let mut buf4 = snappin_core_algo::RgbaBuffer::new(
            w,
            h,
            {
                let mut d = vec![0u8; w as usize * h as usize * 4];
                for px in d.chunks_exact_mut(4) {
                    px[3] = 255;
                }
                d
            },
        )
        .expect("buf");
        let payloads = vec![ShapePayload::Watermark {
            x: 0.0,
            y: 0.0,
            w: 6.0,
            h: 4.0,
            step_x: 2.0,
            step_y: 2.0,
            png: sample_png_b64(),
        }];
        let shapes = to_shapes(&payloads, 1.0).expect("to_shapes");
        snappin_core_algo::draw::draw_shapes(&mut buf4, &shapes);
        assert_eq!(buf4.pixel(0, 0), Some([255, 0, 0, 255]), "第 1 行第 1 块 = 红");
        assert_eq!(buf4.pixel(1, 0), Some([0, 255, 0, 255]), "第 1 行第 1 块 = 绿");
        assert_eq!(buf4.pixel(4, 0), Some([255, 0, 0, 255]), "第 3 块平铺到位");
        assert_eq!(buf4.pixel(0, 2), Some([255, 0, 0, 255]), "第 2 行平铺到位");
    }
}
