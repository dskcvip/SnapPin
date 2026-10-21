use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;

/// 应用配置（前端契约见 docs/CONTRACTS.md）。
/// serde 字段名统一 camelCase，与前端 TS 契约一致。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AppConfig {
    pub theme_color: String,
    pub ball_size: u32,
    /// 悬浮球**球心**位置（物理像素；创建窗口时用 set_position 精确还原）
    pub ball_position: Option<BallPosition>,
    /// `ball_position` 是否为球心语义。
    /// 旧版本存的是窗口左上角，为 false；新版一律 true。
    /// 留这个标记是为了让旧配置只换算一次，不会每次启动都把球往前推。
    pub ball_pos_center: bool,
    pub shortcut_capture: String,
    pub shortcut_pin_clipboard: String,
    pub save_dir: Option<String>,
    /// 翻译配置（在线 API，OpenAI 兼容协议）。Key 由用户自己填，
    /// 只落本地 config.json，不进安装包、不进 git。
    pub translation: snappin_core_platform::translate::TranslateConfig,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BallPosition {
    pub x: f64,
    pub y: f64,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            theme_color: "#12A5D8".to_string(),
            ball_size: 56,
            ball_position: None,
            ball_pos_center: false,
            shortcut_capture: "Ctrl+1".to_string(),
            shortcut_pin_clipboard: "Ctrl+2".to_string(),
            save_dir: None,
            translation: Default::default(),
        }
    }
}

/// 从磁盘加载配置；文件不存在或损坏时回退默认值。
pub fn load(dir: &Path) -> AppConfig {
    let path = dir.join("config.json");
    match fs::read_to_string(&path) {
        Ok(text) => serde_json::from_str(&text).unwrap_or_default(),
        Err(_) => AppConfig::default(),
    }
}

/// 写回配置。
pub fn save(dir: &Path, config: &AppConfig) -> anyhow::Result<()> {
    fs::create_dir_all(dir)?;
    let text = serde_json::to_string_pretty(config)?;
    fs::write(dir.join("config.json"), text)?;
    Ok(())
}
