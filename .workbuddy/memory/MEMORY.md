# SnapPin 项目长期约定

## 模型文件托管（2026-10-21 用户明确要求）
- OCR / 识别类模型**不放进安装包**，运行时按需下载到**固定目录**：
  Windows `%APPDATA%\com.snappin.dev\models\`，macOS `~/Library/Application Support/com.snappin.dev/models/`。
- 仓库侧建 `models/` 目录作为**模型源仓库**：模型文件（连同 README、校验和、下载脚本）
  直接提交进 git，用户从仓库/发行页下载，`scripts/fetch_models.py`（或 .bat）负责
  下载到上述固定目录。
- 应用侧契约：`ensure_models` 命令检查固定目录 → 缺失则从配置的 URL 下载
  （默认指向仓库发行页）→ 下载失败给出明确错误，**不让应用崩溃**。
- 因此 M4 OCR 的实现顺序：先做模型固定目录 + ensure_models 契约，再接推理。

## 构建环境（本机）
- 本会话内直接 `cargo build` 会 `STATUS_DLL_NOT_FOUND`（rustup 代理在沙箱内起不来）。
  必须：`cd src-tauri && RUSTC="C:/Users/dskc/.rustup/toolchains/stable-x86_64-pc-windows-msvc/bin/rustc.exe" ~/.rustup/toolchains/stable-x86_64-pc-windows-msvc/bin/cargo.exe build`
- Windows NTFS 大小写不敏感：`snappin.exe` 与 `SnapPin.exe` 是同一条目录项，别用旧名删文件。
- 本会话无法拉起持久 GUI 进程（Job 对象回收 + schtasks 黑名单 + WMI 被拦），
  kill explorer 后只能让用户手动恢复：Ctrl+Shift+Esc → 文件 → 运行新任务 → explorer.exe。

## 代码约定
- core-algo 零依赖、只吃 RGBA8、每个公开 API 有单测（远期 C++ 移植）。
- 文字/水印等带字形的标注一律走「UI 层光栅化 → 位图上传」通道，核心层不开字体口子。
- 新增 IPC 必须同步 docs/CONTRACTS.md，前端类型在 src/lib/api.ts。
- tauri.conf.json 已设 `mainBinaryName: "SnapPin"`；产物为 target/debug/SnapPin.exe。
