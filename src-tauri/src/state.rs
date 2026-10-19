use crate::config::AppConfig;
use snappin_core_platform::MonitorShot;
use std::collections::HashMap;
use std::sync::Mutex;

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
}

impl AppState {
    pub fn new() -> Self {
        Self {
            config: Mutex::new(AppConfig::default()),
            monitors: Mutex::new(Vec::new()),
            pins: Mutex::new(HashMap::new()),
        }
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}
