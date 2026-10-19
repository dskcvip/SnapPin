# SnapPin IPC 契约

前端与 Rust 核心的唯一契约。规则：

1. 前端 `src/lib/api.ts` 独立声明 TS 类型，**不 import 任何 Rust 类型**（架构约束④）。
2. 命令参数：JS camelCase → Rust snake_case 由 Tauri 自动转换。
3. 响应载荷：Rust 侧统一 `#[serde(rename_all = "camelCase")]`。
4. 鸿蒙 PC 壳将来按本文件实现适配层即可复用全部 UI。

## 命令

| 命令 | 参数 | 返回 | 说明 |
|------|------|------|------|
| `start_capture` | – | `CapturePayload` | 截取全部显示器并打开浮层 |
| `overlay_context` | – | `OverlayContext` | 浮层窗口获取自己的显示器上下文 |
| `confirm_selection` | `monitorId`, `rect`, `action` | `{ path }` | 执行选区动作并关闭全部浮层 |
| `cancel_capture` | – | `void` | 取消截图，关闭全部浮层 |
| `pin_from_clipboard` | – | `string`(label) | 剪贴板图片贴图，返回贴图窗口 label |
| `pin_close` | `label` | `void` | 关闭贴图 |
| `pin_copy` | `label` | `void` | 复制贴图到剪贴板 |
| `pin_save` | `label` | `boolean` | 对话框保存贴图（取消返回 false） |
| `ball_command` | `cmd` | `void` | 悬浮球命令：`capture`/`pin-clipboard`/`hide`/`quit` |
| `drag_window` | – | `void` | 对调用方窗口启动原生拖拽 |
| `get_config` | – | `AppConfig` | 读取配置 |
| `update_config` | `config` | `AppConfig` | 整体更新配置并持久化 |
| `app_info` | – | `AppInfo` | 版本/平台/架构 |

## 类型

```ts
interface Rect { x: number; y: number; w: number; h: number }   // 逻辑像素

interface MonitorInfo {
  id: string; name: string
  x: number; y: number            // 显示器原点（物理像素）
  width: number; height: number   // 物理像素
  scaleFactor: number
}

interface CapturePayload {
  monitors: MonitorInfo[]
  shots: { monitorId: string; path: string }[]
}

interface OverlayContext {
  monitorId: string
  imagePath: string        // 冻结图 PNG 本地路径（asset 协议加载）
  naturalWidth: number     // 物理像素
  naturalHeight: number
  scaleFactor: number
}

type ShotAction = 'copy' | 'pin' | 'save'
type BallCommand = 'capture' | 'pin-clipboard' | 'hide' | 'quit'

interface AppConfig {
  themeColor: string
  ballSize: number
  ballPosition: { x: number; y: number } | null   // 物理像素
  shortcutCapture: string        // 展示用；实际快捷键为内置常量（M7 开放自定义）
  shortcutPinClipboard: string
  saveDir: string | null
}

interface AppInfo { version: string; platform: string; arch: string }
```

## 事件

当前版本无推送事件（全部为命令请求/响应）。规划中：

| 事件 | 载荷 | 说明 |
|------|------|------|
| `capture-state` | `{ phase: 'idle' \| 'selecting' \| 'recording' }` | 悬浮球状态机（录屏计时等） |

## 窗口 URL 约定

| 窗口 | URL |
|------|-----|
| 悬浮球 | `index.html?window=ball` |
| 截图浮层 | `index.html?window=overlay&monitor=m0` |
| 贴图 | `index.html?window=pin&label=pin-xxx&src=<pct-encoded>&w=<logical>&h=<logical>` |
| 主窗 | `index.html?window=main` |

## 坐标系统一约定

- 前端选区 rect：overlay 窗口的逻辑像素坐标
- Rust 裁剪：`rect × scaleFactor` → 物理像素后调用 `core-algo::crop`
- 贴图显示尺寸：`物理像素 / scaleFactor` → 逻辑像素
