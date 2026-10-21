use crate::config::AppConfig;
use snappin_core_platform::MonitorShot;
use snappin_core_platform::ocr::OcrEngine;
use snappin_core_platform::stitch::LongCapture;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64};
use std::sync::{Arc, Mutex};

/// 贴图窗口元数据（label → 信息）。
/// 前端通过 pin_context 命令按窗口标签取回，避免 URL 传参的编码问题。
#[derive(Debug, Clone)]
pub struct PinMeta {
    /// 贴图图片的本地路径（PNG）——**当前显示的内容**。
    /// 做过图像处理（旋转/翻转/灰度…）后指向处理后的临时文件。
    pub src: String,
    /// 逻辑像素显示宽
    pub width: f64,
    /// 逻辑像素显示高
    pub height: f64,
    /// 图像处理产生的临时文件（None = 原图，未处理过）。
    /// 关窗/再次处理时删除，避免临时目录无限增长。
    pub edited: Option<String>,
}

/// 长截图会话（拼接器 + 结束时要还原的浮层状态）。
pub struct LongSession {
    pub capture: LongCapture,
    /// 浮层窗口索引（label = OVERLAY_PREFIX + idx）
    pub overlay_idx: usize,
    /// 浮层顶层窗口句柄（Windows 采集排除用；非 Windows / 未取到为 0）
    pub overlay_hwnd: isize,
    /// WDA 不可用时的"采集前后隐藏浮层"兜底是否生效
    ///（决定收尾时要不要显式 show 回来）。
    pub used_fallback: bool,
    /// 缩略图临时路径（驱动循环每步覆盖写，前端状态条预览用）
    pub thumb: PathBuf,
}

/// 自绘菜单的种类：决定菜单内容与点击后的动作路由。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuKind {
    /// 托盘 / 悬浮球菜单（应用级动作，见 lib.rs handle_menu_event）
    App,
    /// 贴图右键菜单（动作转发给对应贴图窗，见 commands.rs menu_action）
    Pin,
}

/// 自绘菜单锚点（**物理**屏幕坐标 + 目标）。
///
/// `Some` = 菜单正处于"打开流程中"（含前端测量尺寸的窗口期）；
/// `None` = 已关闭。菜单窗是隐藏预建的，前端挂载后靠这个判断
/// 该不该测量并显示（首次创建窗口时会丢掉 emit 的事件）。
pub struct MenuAnchor {
    pub x: f64,
    pub y: f64,
    pub kind: MenuKind,
    /// 贴图菜单的目标窗口 label（kind = Pin 时有效）
    pub pin_label: Option<String>,
}

/// 贴图窗的状态快照（界面开关 + 图像处理参数）。
///
/// 状态的"事实来源"在贴图窗自己的 React 里，Rust 只缓存一份最新快照，
/// 供右键菜单渲染勾选标记与当前档位（菜单窗与贴图窗是两个 webview，
/// 无法直接互访）。前端在挂载时和每次变化后调 `pin_publish_flags` 刷新。
#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PinFlags {
    // ---- 界面开关 ----
    pub topmost: bool,
    pub locked: bool,
    pub shadow: bool,
    /// 悬停是否显示工具条（菜单"工具栏"开关 / H 键）。
    /// 关掉后贴图只剩图片 + 边框，右键菜单仍可用（不然就没法再打开）。
    pub toolbar: bool,
    /// 窗口整体不透明度（顶部滑块 / 工具条"透明度"滑条 / Alt+滚轮）
    pub opacity: f64,
    // ---- 图像处理参数（非累积：每次都从原图重算） ----
    /// 顺时针旋转角度：0 / 90 / 180 / 270
    pub rotate: u16,
    pub flip_h: bool,
    pub flip_v: bool,
    pub gray: bool,
    pub invert: bool,
    /// 图像自身 alpha（菜单"图像处理 → 不透明度"），与窗口不透明度是两回事
    pub alpha: f64,
    /// 亮度系数：1.0 = 原始
    pub brightness: f64,
}

impl Default for PinFlags {
    /// 与贴图窗的初始状态一致（置顶开、未锁定、有阴影、工具条显示、未做任何处理）
    fn default() -> Self {
        Self {
            topmost: true,
            locked: false,
            shadow: true,
            toolbar: true,
            opacity: 1.0,
            rotate: 0,
            flip_h: false,
            flip_v: false,
            gray: false,
            invert: false,
            alpha: 1.0,
            brightness: 1.0,
        }
    }
}

/// 全局应用状态。
pub struct AppState {    /// 当前配置（磁盘 config.json 的内存镜像）
    pub config: Mutex<AppConfig>,
    /// 当前截图会话：浮层打开期间有效，confirm_selection 据此裁剪
    pub monitors: Mutex<Vec<MonitorShot>>,
    /// 存活贴图窗口的元数据
    pub pins: Mutex<HashMap<String, PinMeta>>,
    /// 采集互斥：同一时刻只允许一个采集流程。
    ///
    /// 不能靠"浮层窗口是否已存在"来防重入——第一次采集在窗口创建前有
    /// 上百毫秒空窗（隐藏浮层是并行预创建的），此期间重复触发会启动
    /// 第二遍采集：它复用第一遍的浮层窗口（页面不重载 → 前端状态陈旧），
    /// 并把浮层自身截进冻结图，表现为"界面发灰 + 超时"。
    pub capturing: AtomicBool,
    /// 截图会话代号：每开始一次采集递增。
    ///
    /// 取消/确认请求携带自己所属的代号，Rust 只处理匹配的请求。
    /// 场景：用户按 Esc 取消后立刻又点悬浮球截图，上一轮迟到的
    /// cancel IPC 会在这个空窗里执行，把新会话的浮层关掉、把
    /// 新会话的采集数据清空 → 新浮层永远拿不到数据 → "截图采集超时"。
    pub session: AtomicU64,
    /// 采集尚未完成时收到的取消请求（此时前端还没有会话代号）。
    /// 由 do_capture 在显示浮层之前消费。
    pub cancel_pending: AtomicBool,
    /// OCR 引擎（懒加载，首次识别时初始化；模型缺失时为 None 且不缓存失败）。
    ///
    /// 推理是秒级操作（det 全高清约 0.8s），必须放 `spawn_blocking`；
    /// 引擎本身（两个 rten 模型 + 字典）加载一次约 60ms，缓存避免重复解析。
    pub ocr: Mutex<Option<OcrEngine>>,
    /// 长截图会话（None = 未进行）。由驱动循环持锁逐步 `step()`。
    ///
    /// 与 `capturing` 互斥的关系：长截图复用同一次截图会话的浮层（用户在
    /// 浮层上框选后点"长截图"），因此不占用 `capturing`；但会话期间
    /// 前端会阻止新的截图触发（工具条只露停止按钮）。
    pub long: Mutex<Option<LongSession>>,
    /// 长截图停止标志：Esc（全局热键 / 浮层键盘）置位，驱动循环在
    /// 每步稳态等待期检查（~100ms 响应），随后收尾并发出 done 事件。
    /// 同时持有在 `LongCapture` 内部（克隆同一枚 Arc）。
    pub long_stop: Arc<AtomicBool>,
    /// 长截图是否进行中（不取锁的快照，供全局热键处理器查询——
    /// 主线程不能阻塞在驱动循环持的 `long` 锁上）。
    pub long_active: AtomicBool,
    /// 自绘菜单锚点（见 MenuAnchor）。`None` = 菜单已关闭。
    pub menu_anchor: Mutex<Option<MenuAnchor>>,
    /// 贴图界面开关快照（label → flags）。贴图右键菜单据此画勾。
    pub pin_flags: Mutex<HashMap<String, PinFlags>>,
    /// 当前生效的全局快捷键（截图, 剪贴板贴图）。
    ///
    /// 热键处理器与设置窗共用这一份事实来源：用户在设置里改键后
    /// `update_config` 会同步更新它，处理器据此比对，改键即时生效。
    pub hotkeys: Mutex<(tauri_plugin_global_shortcut::Shortcut, tauri_plugin_global_shortcut::Shortcut)>,
    /// 注册失败的内置快捷键名（空 = 全部成功）。
    /// 自绘菜单据此在顶部显示一条告警——热键注册失败时用户唯一的可见线索。
    pub shortcut_failed: Mutex<Vec<String>>,
    /// OCR 结果窗内容（最近一次识别）。窗口是单例，前端挂载/再次打开时
    /// 用 `ocr_result` 取；重复识别走 `ocr-open` 事件通知已存在的窗口重取。
    pub ocr_result: Mutex<Vec<crate::commands::OcrLinePayload>>,
    /// 翻译窗状态（原文 / 译文 / 错误 / 进行中）。
    /// 翻译是秒级网络操作，放阻塞线程里跑，结果写这里 + 发 `trans-update`
    /// 事件，前端据此渲染，不轮询。
    pub trans: Mutex<TransState>,
}

/// 翻译窗的完整状态（前端 `trans_state` 的返回体 + `trans-update` 事件载荷）。
#[derive(Debug, Clone, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TransState {
    /// 原文（用户点「翻译」时的 OCR 全文）
    pub original: String,
    /// 译文（成功后有值）
    pub translated: String,
    /// 失败原因（中文可操作文案；空 = 没出错）
    pub error: String,
    /// 是否正在翻译
    pub busy: bool,
    /// 目标语言（来自配置，窗内标题展示用）
    pub target_lang: String,
    /// 产出本次译文的模型名（空 = 还没翻译）
    pub model_name: String,
}

impl AppState {
    pub fn new() -> Self {
        Self {
            config: Mutex::new(AppConfig::default()),
            monitors: Mutex::new(Vec::new()),
            pins: Mutex::new(HashMap::new()),
            capturing: AtomicBool::new(false),
            session: AtomicU64::new(0),
            cancel_pending: AtomicBool::new(false),
            ocr: Mutex::new(None),
            long: Mutex::new(None),
            long_stop: Arc::new(AtomicBool::new(false)),
            long_active: AtomicBool::new(false),
            menu_anchor: Mutex::new(None),
            pin_flags: Mutex::new(HashMap::new()),
            hotkeys: Mutex::new((
                crate::default_capture_shortcut(),
                crate::default_pin_shortcut(),
            )),
            shortcut_failed: Mutex::new(Vec::new()),
            ocr_result: Mutex::new(Vec::new()),
            trans: Mutex::new(TransState::default()),
        }
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}
