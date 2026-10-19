# SnapPin

跨平台桌面截图 + 贴图工具。纯桌面端应用（Windows / macOS / Linux），全本地处理，无云端依赖，所用 SDK 全部开源。

## 功能矩阵（当前进度）

| 模块 | 状态 | 说明 |
|------|------|------|
| 截图（静态） | ✅ 已实现 | 多显示器、透明浮层选区、像素级选区、冻结画面 |
| 选区动作 | ✅ 已实现 | 复制到剪贴板 / 贴图 / 保存（对话框选格式） |
| 贴图 | ✅ 已实现 | 无边框置顶窗口、拖动、透明度、Ctrl+滚轮缩放、Esc 关闭 |
| 悬浮球 | ✅ 已实现 | 单击截图、拖动移动、位置记忆、主题色/尺寸可配 |
| 全局快捷键 | ✅ 已实现 | Ctrl+1 截图、Ctrl+2 剪贴板贴图 |
| 托盘 | ✅ 已实现 | 截图 / 剪贴板贴图 / 显隐悬浮球 / 退出 |
| 单实例 | ✅ 已实现 | 二次启动唤出悬浮球 |
| OCR / 二维码 / 取色 | ⏳ 规划中 | 见 ROADMAP M4 |
| 长截图拼接 | ⏳ 规划中 | 见 ROADMAP M5 |
| 录屏 | ⏳ 规划中 | 见 ROADMAP M6 |
| 标注工具 | ⏳ 规划中 | 见 ROADMAP M3 |
| 自定义快捷键 / 脚本引擎 | ⏳ 规划中 | 见 ROADMAP M7 |

## 技术栈

- **壳**：Tauri v2（Rust）+ 系统 WebView（Win WebView2 / mac WKWebView / Linux WebKitGTK）
- **前端**：React 19 + TypeScript + Vite 8，单 SPA 四窗口（悬浮球 / 截图浮层 / 贴图 / 主窗）
- **核心**：`snappin-core-algo`（零依赖纯算法）+ `snappin-core-platform`（xcap 截图 / arboard 剪贴板 / image 编解码）
- **插件**：global-shortcut、single-instance、dialog（全部开源）

架构细节见 [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md)，IPC 契约见 [docs/CONTRACTS.md](docs/CONTRACTS.md)，里程碑见 [docs/ROADMAP.md](docs/ROADMAP.md)。

## 开发

```bash
npm install        # 安装前端依赖
npm run tauri dev  # 开发模式（vite + 桌面端热更新）
npm run build      # 类型检查 + 前端构建
cargo test         # 算法层单元测试
npm run tauri build  # 打包当前平台安装包
```

## 目录结构

```
├── src/                     # 前端（单 SPA，按 ?window= 路由到四个窗口）
│   ├── windows/             #   ball / overlay / pin / main 四个窗口组件
│   ├── lib/                 #   api.ts（IPC 契约层）/ windowKind.ts
│   └── styles/              #   各窗口样式
├── core/
│   ├── core-algo/           # 零依赖纯算法（RGBA8 缓冲，为远期 C++ 移植预留）
│   └── core-platform/       # 平台胶合层（截图/剪贴板/图像 IO，不依赖 Tauri）
├── src-tauri/               # Tauri 壳：窗口管理 / 快捷键 / 托盘 / IPC 命令
├── docs/                    # 架构 / 契约 / 路线图
└── .github/workflows/       # 三平台 CI
```

## 数据位置

- 配置：`%APPDATA%/com.snappin.dev/config.json`（macOS: `~/Library/Application Support/com.snappin.dev/`）
- 截图临时文件：`%TEMP%/snappin/`

## License

MIT（第三方组件许可证见各自项目）。
