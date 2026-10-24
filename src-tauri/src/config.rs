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
    pub shortcut_capture: String,
    pub shortcut_pin_clipboard: String,
    pub save_dir: Option<String>,
    /// 翻译配置（在线 API，OpenAI 兼容协议）。Key 由用户自己填，
    /// 只落本地 config.json，不进安装包、不进 git。
    pub translation: snappin_core_platform::translate::TranslateConfig,
}

/// 默认快捷键字符串。
///
/// **平台差异**：macOS 习惯是 ⌘（Command），Windows/Linux 是 Ctrl。
/// 默认给 Ctrl+1 在 mac 上"能用但不对劲"——用户肌肉记忆是按 ⌘1。
/// 已保存过配置的老用户不受影响（读的是自己 config.json 里的值）。
const DEFAULT_CAPTURE: &str = if cfg!(target_os = "macos") {
    "Super+1"
} else {
    "Ctrl+1"
};
const DEFAULT_PIN: &str = if cfg!(target_os = "macos") {
    "Super+2"
} else {
    "Ctrl+2"
};

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            theme_color: "#12A5D8".to_string(),
            ball_size: 56,
            shortcut_capture: DEFAULT_CAPTURE.to_string(),
            shortcut_pin_clipboard: DEFAULT_PIN.to_string(),
            save_dir: None,
            translation: Default::default(),
        }
    }
}

/// 从磁盘加载配置；文件不存在或损坏时回退默认值。
pub fn load(dir: &Path) -> AppConfig {
    let path = dir.join("config.json");
    let mut cfg: AppConfig = match fs::read_to_string(&path) {
        Ok(text) => serde_json::from_str(&text).unwrap_or_default(),
        Err(_) => AppConfig::default(),
    };
    // 历史配置自愈（空 id/name 的模型记录，见 TranslateConfig::normalize）
    cfg.translation.normalize();
    cfg
}

/// 写回配置。
pub fn save(dir: &Path, config: &AppConfig) -> anyhow::Result<()> {
    fs::create_dir_all(dir)?;
    let text = serde_json::to_string_pretty(config)?;
    fs::write(dir.join("config.json"), text)?;
    Ok(())
}
