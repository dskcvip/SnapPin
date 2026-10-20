use crate::config::AppConfig;
use snappin_core_platform::MonitorShot;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64};
use std::sync::Mutex;
use std::time::Instant;

/// 贴图窗口元数据（label → 信息）。
/// 前端通过 pin_context 命令按窗口标签取回，避免 URL 传参的编码问题。
#[derive(Debug, Clone)]
pub struct PinMeta {
    /// 贴图图片的本地路径（PNG）
    pub src: String,
    /// 逻辑像素显示宽
    pub width: f64,
    /// 逻辑像素显示高
    pub height: f64,
}

/// 全局应用状态。
pub struct AppState {
    /// 当前配置（磁盘 config.json 的内存镜像）
    pub config: Mutex<AppConfig>,
    /// 当前截图会话：浮层打开期间有效，confirm_selection 据此裁剪
    pub monitors: Mutex<Vec<MonitorShot>>,
    /// 存活贴图窗口的元数据
    pub pins: Mutex<HashMap<String, PinMeta>>,
    /// Ctrl+V 贴图武装窗口：截图"复制"成功后一段时间内，
    /// 全局 Ctrl+V 被 SnapPin 拦截用于弹出贴图（PixPin 标志性交互）。
    /// 过期/贴图后立即解除武装并注销热键，恢复正常粘贴行为。
    /// Some((generation, deadline))：generation 用于废弃过期的解除任务。
    pub paste_armed: Mutex<Option<(u64, Instant)>>,
    /// 武装代数（每次 arm/disarm 递增，用于幂等解除）
    pub paste_gen: AtomicU64,
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
}

impl AppState {
    pub fn new() -> Self {
        Self {
            config: Mutex::new(AppConfig::default()),
            monitors: Mutex::new(Vec::new()),
            pins: Mutex::new(HashMap::new()),
            paste_armed: Mutex::new(None),
            paste_gen: AtomicU64::new(0),
            capturing: AtomicBool::new(false),
            session: AtomicU64::new(0),
            cancel_pending: AtomicBool::new(false),
        }
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}
