# SnapPin 架构

## 一、五层架构

```
┌─────────────────────────────────────────────────────┐
│ UI 层      React 19 + TS + Canvas（单 SPA 四窗口）        │
│            悬浮球 / 截图浮层 / 贴图 / 主窗                │
├─────────────────────────────────────────────────────┤
│ IPC 契约层  src/lib/api.ts（camelCase 类型，唯一契约）     │
├─────────────────────────────────────────────────────┤
│ 核心层      snappin-core（Rust，不依赖 Tauri）            │
│            ├─ core-algo      零依赖纯算法（RGBA8）        │
│            └─ core-platform  平台胶合（截图/剪贴板/IO）    │
├─────────────────────────────────────────────────────┤
│ 平台抽象    xcap（Win DXGI / mac ScreenCaptureKit /      │
│            Linux X11）、arboard、系统 WebView            │
├─────────────────────────────────────────────────────┤
│ Tauri 壳    src-tauri：窗口管理 / 全局快捷键 / 托盘 /      │
│            保存对话框 / 单实例                            │
└─────────────────────────────────────────────────────┘
```

核心原则：**core-algo 与 core-platform 不依赖 Tauri**，可独立编译、独立单测；
壳层只做 IPC 与窗口管理。

## 二、开源组件选型

| 能力 | 组件 | 协议 |
|------|------|------|
| 桌面壳 | Tauri v2 | MIT/Apache-2.0 |
| 屏幕采集 | xcap（Win DXGI / mac ScreenCaptureKit / Linux X11） | Apache-2.0 |
| 剪贴板 | arboard | MIT |
| 图像编解码 | image | MIT |
| 全局快捷键 | tauri-plugin-global-shortcut | MIT/Apache-2.0 |
| 保存对话框 | tauri-plugin-dialog | MIT/Apache-2.0 |
| 单实例 | tauri-plugin-single-instance | MIT/Apache-2.0 |
| UI | React + Vite | MIT |

规划中组件的选型（OCR=RapidOCR/ONNX、公式=pix2tex、二维码=rxing、
编码=gifski/openh264、脚本=rquickjs、擦除=OpenCV inpaint）见 ROADMAP。

## 三、鸿蒙 PC 留门规则（现在就要遵守）

远期第四平台为鸿蒙电脑（PC）。Rust 无 OpenHarmony 编译目标，
因此以下四条是现在的硬性设计约束：

1. **算法层纯化**：`core-algo` 零依赖、只吃 RGBA8 缓冲、100% 单测。
   将来移植到 C++ 是机械翻译，不是重新设计。
2. **重算法选型对齐鸿蒙可用的开源库**：拼接=OpenCV、擦除=OpenCV inpaint、
   编码=FFmpeg（不开 GPL）、图像基础处理=image。
3. **推理双轨**：模型同时导出 ONNX + ncnn 两种格式（桌面 ort / 鸿蒙 ncnn）。
4. **前端零耦合**：UI 只认 IPC 契约（camelCase TS 类型），鸿蒙壳用
   ArkTS + Web 组件加载同一套 `dist/`，另写 IPC 适配层。

> 注：xcap 0.9 已声明支持 HarmonyOS（OpenHarmony）截图后端，
> 若经验证可用，鸿蒙 PC 的采集层可直接复用（此点为待验证信息）。

## 四、截图主流程（数据流）

```
Ctrl+1 / 悬浮球 / 托盘
  → commands::do_capture
      → core-platform::capture_all_monitors（xcap 全屏截图 + 存临时 PNG）
      → windows::open_overlays（每显示器一个透明全尺寸浮层）
  → 前端 OverlayWindow（冻结图 + 拖拽选区 + 工具栏）
  → confirm_selection(monitorId, rect, action)
      → core-algo::RgbaBuffer::crop（逻辑像素 × scaleFactor → 物理像素）
      → copy → arboard 写剪贴板
        pin  → create_pin_window（无边框置顶窗）
        save → 对话框选路径
      → close_overlays
```

## 五、已知限制（诚实清单）

| 限制 | 说明 |
|------|------|
| 混合 DPI 多显示器 | 浮层位置按 `物理像素 / scaleFactor` 换算，不同缩放因子的副屏可能有偏差 |
| Linux Wayland | xcap 走 X11 路径；纯 Wayland 会话需 XWayland，后续换 PipeWire portal |
| macOS 权限 | 屏幕录制/辅助功能授权流程依赖 xcap 行为，尚未在真机验证 |
| 快捷键冲突 | 注册失败时静默降级（仍可用悬浮球/托盘），后续加失败提示 |
| 保存对话框 | 当前为阻塞式原生对话框；格式选项仅 PNG/JPEG；保存流程未做 E2E 验证 |
| 鸿蒙 PC | 截图/全局置顶窗 API 开放度未经 DevEco 文档验证（推断） |

## 六、Windows 平台硬约束（踩坑记录，2026-10-20）

1. **裸 `cargo run` 白屏**：tauri 的 `dev` cfg 由 `custom-protocol` 特性决定
   （`let dev = !custom_protocol`）。未开该特性时 `WebviewUrl::App` 解析到
   devUrl，直连二进制（无 vite dev server）即白屏。本项目在 src-tauri/Cargo.toml
   透传 `custom-protocol` 特性，开发运行统一带 `--features custom-protocol`。
2. **同步命令内创建 webview 死锁/空白**：Tauri 文档 Known issues——Windows 上
   `WebviewWindowBuilder` 在同步命令或事件处理器（均跑在主线程）中创建 webview
   会死锁，症状为"窗口建成但页面永不加载、无鼠标事件"。所有建窗流程
   （截图浮层/贴图窗）必须在 `async` 命令 + `tauri::async_runtime::spawn_blocking`
   中执行。
3. **透明窗点击穿透**：WebView2 透明背景下，页面透明区域会透传鼠标事件
   （浮层未加载出内容时表现为"完全无法交互"）。加载失败时先查资源协议
   （assetProtocol scope 与 convertFileSrc URL），不要误判为事件系统问题。
