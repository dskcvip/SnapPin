//! 在线翻译（OCR 结果窗「翻译」按钮 / 独立翻译窗共用）。
//!
//! # 为什么是在线 API 而不是本地模型
//! 本地翻译（Opus-MT / llama.cpp，路线图 M8）动辄几百 MB 模型，与"模型小、
//! 能进 git"的现状冲突；用户环境里也已经有可用的 LLM API。因此这里做
//! **协议兼容层**：任何 OpenAI 兼容的 `/chat/completions` 端点都能用，
//! 配置是「模型列表 + 全局目标语言」，Key 由用户自己填、只存本地 config.json。
//!
//! # 错误处理约定
//! 所有失败都转成**中文可操作错误**（没配模型 / 没填 Key / 401 / 402 / 404 /
//! 429 / 超时分别给不同文案），因为翻译失败时用户唯一能看到的就是这句话——
//! 返回 "reqwest error: ..." 等于没说。

use serde::{Deserialize, Serialize};
use std::time::Duration;

/// 一个自定义模型（翻译用的在线端点）。
///
/// 对应设置窗「模型」页列表里的一行：名称 + 备注 + 三个必填项
/// （Base URL / API Key / 模型 ID）+ 一个可选的「获取 Key 网址」。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct TranslateModel {
    /// 稳定标识（新建时生成，编辑/删除/测试连接都靠它指认）
    pub id: String,
    /// 显示名（如 "DeepSeek-V4 Pro"）
    pub name: String,
    /// 副标题（如 "深度搜索" / "Kimi 中国版"）
    pub note: String,
    /// OpenAI 兼容端点根地址（不带 /chat/completions）
    pub base_url: String,
    /// API Key；空 = 未配置（测试连接与翻译都会被拦下并引导去填写）
    pub api_key: String,
    /// 模型 ID（默认 "deepseek-flash"）
    pub model: String,
    /// 「获取 Key」的网址（列表行内链接图标打开它）
    pub key_url: String,
}

impl Default for TranslateModel {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: String::new(),
            note: String::new(),
            base_url: "https://api.deepseek.com".to_string(),
            api_key: String::new(),
            model: "deepseek-flash".to_string(),
            key_url: "https://platform.deepseek.com/api_keys".to_string(),
        }
    }
}

/// 翻译配置（持久化在 config.json 的 `translation` 字段）。
///
/// `models` **第一个 = 当前启用的模型**（设置窗列表里标"使用中"）——
/// 不做单选状态机：顺序即优先级，简单，也不会出现"选了不存在的模型"。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct TranslateConfig {
    pub models: Vec<TranslateModel>,
    /// 目标语言（写进 system prompt，如 "英文" / "日文" / "繁体中文"）
    pub target_lang: String,
}

impl Default for TranslateConfig {
    fn default() -> Self {
        Self {
            // 预置一条 DeepSeek（Key 留空）：用户只要填 Key 就能用，
            // 行内链接图标也有地方可去。不预置全家桶——那是营销，不是默认值。
            models: vec![TranslateModel::default()],
            target_lang: "英文".to_string(),
        }
    }
}

impl TranslateConfig {
    /// 当前启用的模型（列表第一个）。
    pub fn active(&self) -> Option<&TranslateModel> {
        self.models.first()
    }

    /// 按 id 找模型（编辑 / 删除 / 测试连接用）。
    pub fn find(&self, id: &str) -> Option<&TranslateModel> {
        self.models.iter().find(|m| m.id == id)
    }
}

/// 翻译结果（供前端渲染与错误展示）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TransOutcome {
    pub ok: bool,
    /// 译文（ok = true 时非空）
    pub translated: String,
    /// 失败原因（ok = false 时非空，中文可操作文案）
    pub error: String,
}

/// 把 `text` 翻译成配置的目标语言。
///
/// 阻塞调用（内部 reqwest blocking）——**必须放在阻塞线程**里执行，
/// 见 src-tauri 的 `trans_start`。
pub fn translate(cfg: &TranslateConfig, text: &str) -> TransOutcome {
    let text = text.trim();
    if text.is_empty() {
        return TransOutcome::fail("没有可翻译的内容");
    }
    let Some(m) = cfg.active() else {
        return TransOutcome::fail(
            "尚未添加翻译模型：托盘菜单 → 设置 → 模型，添加一个模型并填写 API Key",
        );
    };
    if m.api_key.trim().is_empty() {
        return TransOutcome::fail(format!(
            "模型「{}」还没有填写 API Key：托盘菜单 → 设置 → 模型，点该行的编辑图标补上 Key",
            if m.name.is_empty() {
                "（未命名）"
            } else {
                &m.name
            }
        ));
    }
    let url = format!("{}/chat/completions", m.base_url.trim().trim_end_matches('/'));
    let body = serde_json::json!({
        "model": m.model,
        "temperature": 0.2,
        "messages": [
            {
                "role": "system",
                "content": format!(
                    "你是专业翻译引擎。把用户给出的内容翻译成{}，只输出译文本身，\
                     不要解释、不要加引号、不要标注语言名称；原文中的换行与分段原样保留。\
                     如果原文已经是{}，则原样返回。",
                    cfg.target_lang, cfg.target_lang
                ),
            },
            { "role": "user", "content": text },
        ],
    });

    let client = match reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(60))
        .build()
    {
        Ok(c) => c,
        Err(e) => return TransOutcome::fail(format!("创建 HTTP 客户端失败：{e}")),
    };

    let resp = match client.post(&url).bearer_auth(m.api_key.trim()).json(&body).send() {
        Ok(r) => r,
        Err(e) => {
            // 超时 / DNS / 连接失败在这一类里，用户能做的只有换网络或换端点
            return TransOutcome::fail(format!(
                "连接翻译服务失败（{}）：请检查网络与 Base URL（当前 {}）",
                short_net_err(&e),
                m.base_url
            ));
        }
    };

    let status = resp.status();
    let parsed: serde_json::Value = match resp.json() {
        Ok(v) => v,
        Err(e) => {
            return TransOutcome::fail(format!(
                "翻译服务返回了无法解析的内容（HTTP {}）：{e}",
                status.as_u16()
            ));
        }
    };

    if !status.is_success() {
        let msg = parsed
            .get("error")
            .and_then(|e| e.get("message"))
            .and_then(|m| m.as_str())
            .unwrap_or("（服务端未给出原因）");
        let hint = match status.as_u16() {
            401 => "API Key 无效或被撤销",
            402 => "账户余额不足，请充值后重试",
            429 => "请求太频繁，稍等几秒再试",
            404 => "模型名或 Base URL 不对（404）",
            _ => "服务端报错",
        };
        return TransOutcome::fail(format!(
            "翻译失败（HTTP {}）：{hint} — {msg}",
            status.as_u16()
        ));
    }

    let content = parsed
        .get("choices")
        .and_then(|c| c.get(0))
        .and_then(|c| c.get("message"))
        .and_then(|m| m.get("content"))
        .and_then(|c| c.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    if content.is_empty() {
        return TransOutcome::fail("翻译服务返回了空结果，请重试");
    }
    TransOutcome {
        ok: true,
        translated: content,
        error: String::new(),
    }
}

/// 测试连接：用最小请求（`max_tokens=1`）探活。
///
/// 与 `translate` 分开而不是复用：翻译失败可能是内容问题，
/// 测试连接只回答"这个端点 + 这把 Key 通不通"。
pub fn test_connection(cfg: &TranslateConfig, id: &str) -> Result<(), String> {
    let Some(m) = cfg.find(id) else {
        return Err("找不到这个模型（可能已被删除）".to_string());
    };
    if m.api_key.trim().is_empty() {
        return Err("还没有填写 API Key".to_string());
    }
    let url = format!("{}/chat/completions", m.base_url.trim().trim_end_matches('/'));
    let body = serde_json::json!({
        "model": m.model,
        "max_tokens": 1,
        "messages": [{ "role": "user", "content": "hi" }],
    });
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(20))
        .build()
        .map_err(|e| format!("创建 HTTP 客户端失败：{e}"))?;
    let resp = client
        .post(&url)
        .bearer_auth(m.api_key.trim())
        .json(&body)
        .send()
        .map_err(|e| format!("连接失败：{}（{}）", short_net_err(&e), m.base_url))?;
    if resp.status().is_success() {
        return Ok(());
    }
    let status = resp.status().as_u16();
    let parsed: serde_json::Value = resp.json().unwrap_or(serde_json::Value::Null);
    let msg = parsed
        .get("error")
        .and_then(|e| e.get("message"))
        .and_then(|m| m.as_str())
        .unwrap_or("（服务端未给出原因）");
    Err(match status {
        401 => format!("API Key 无效（HTTP 401）：{msg}"),
        402 => format!("余额不足（HTTP 402）：{msg}"),
        404 => "模型或地址不存在（HTTP 404）：检查模型 ID 与 Base URL".to_string(),
        _ => format!("服务端返回 HTTP {status}：{msg}"),
    })
}

impl TransOutcome {
    fn fail(msg: impl AsRef<str>) -> Self {
        Self {
            ok: false,
            translated: String::new(),
            error: msg.as_ref().to_string(),
        }
    }
}

/// 网络错误摘要（reqwest 的 Display 很长，取前 80 字够了）。
fn short_net_err(e: &reqwest::Error) -> String {
    let s = e.to_string();
    if s.chars().count() > 80 {
        format!("{}…", s.chars().take(80).collect::<String>())
    } else {
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg_with(models: Vec<TranslateModel>) -> TranslateConfig {
        TranslateConfig {
            models,
            ..Default::default()
        }
    }

    #[test]
    fn no_models_is_rejected_with_actionable_message() {
        let out = translate(&cfg_with(vec![]), "hello");
        assert!(!out.ok);
        assert!(out.error.contains("设置 → 模型"), "{}", out.error);
    }

    #[test]
    fn empty_key_names_the_model_and_points_at_editor() {
        let mut m = TranslateModel::default();
        m.name = "Kimi K3".into();
        let out = translate(&cfg_with(vec![m]), "hello");
        assert!(!out.ok);
        assert!(out.error.contains("Kimi K3"), "{}", out.error);
        assert!(out.error.contains("API Key"), "{}", out.error);
    }

    #[test]
    fn empty_text_is_rejected_before_touching_network() {
        let out = translate(&cfg_with(vec![TranslateModel::default()]), "  \n ");
        assert!(!out.ok);
        assert!(out.error.contains("没有可翻译"));
    }

    #[test]
    fn active_is_first_and_find_by_id() {
        let mut a = TranslateModel::default();
        a.id = "a".into();
        a.name = "A".into();
        let mut b = TranslateModel::default();
        b.id = "b".into();
        let cfg = cfg_with(vec![a, b]);
        assert_eq!(cfg.active().map(|m| m.id.as_str()), Some("a"));
        assert_eq!(cfg.find("b").map(|m| m.id.as_str()), Some("b"));
        assert!(cfg.find("zzz").is_none());
    }

    #[test]
    fn test_connection_without_key_fails_fast() {
        let mut m = TranslateModel::default();
        m.id = "x".into();
        m.api_key.clear();
        let err = test_connection(&cfg_with(vec![m]), "x").unwrap_err();
        assert!(err.contains("API Key"), "{err}");
    }
}
