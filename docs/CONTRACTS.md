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
| `pin_apply_edit` | `label`, `pngBase64`, `width`, `height` | `PinContext` | 应用一次图像处理：落盘新临时图 → 换源图 → 按新尺寸（中心不动）调窗口 |
| `ball_command` | `cmd` | `void` | 悬浮球命令：`capture`/`pin-clipboard`/`hide`/`quit` |
| `ball_menu` | – | `void` | 在光标处打开**自绘菜单窗**（悬浮球右键） |
| `read_image_bytes` | `path` | `ArrayBuffer` | 读图片原始字节（二进制 IPC）。取色/放大镜要读像素，绕开 asset 协议跨域限制 |
| `pin_menu_open` | – | `void` | 在光标处打开**贴图右键菜单**（目标 = 调用方贴图窗） |
| `pin_publish_flags` | `label`, `flags: PinFlags` | `void` | 贴图窗上报状态快照（开关 + 图像处理参数，供菜单画勾选/档位） |
| `pin_flags` | `label` | `PinFlags` | 读取贴图状态快照（无记录返回默认值） |
| `pin_set_opacity` | `label`, `value` | `void` | 菜单「不透明度」滑块：clamp 到 0.15~1 → 写快照 → emit `pin-set-opacity` 给贴图窗 |
| `menu_fit` | `width`, `height` | `void` | 菜单窗量好尺寸后回报；Rust 据此定位并显示 |
| `menu_close` | – | `void` | 关闭菜单（Esc / 点到面板外留白） |
| `menu_state` | – | `MenuState` | 菜单初始状态（是否打开、快捷键是否注册失败） |
| `menu_action` | `id` | `void` | 菜单项点击：先关菜单再分发。**贴图菜单**改为向目标贴图窗发 `pin-menu-action` 事件 |
| `drag_window` | – | `void` | 对调用方窗口启动原生拖拽 |
| `ocr_download_models` | – | `OcrModels` | 下载 OCR 三件套（15.6MB，Gitee→ModelScope→GitHub 三镜像 + SHA256 校验，幂等） |
| `pin_list` | – | `PinInfo[]` | 列出所有贴图（label / src / 尺寸 / 分组），供贴图管理面板 |
| `pin_focus` | `label` | `void` | 把某张贴图置前（管理面板点缩略图）；窗口不存在则报错 |
| `open_pin_manager` | – | `void` | 打开贴图管理面板。**必须投主线程**（菜单事件里同步建窗会死锁） |
| `get_config` | – | `AppConfig` | 读取配置 |
| `update_config` | `config` | `AppConfig` | 整体更新配置并持久化 |
| `app_info` | – | `AppInfo` | 版本/平台/架构 |

## 类型

```ts
interface Rect { x: number; y: number; w: number; h: number }
// 逻辑像素（**可以带小数**）。Rust 侧 `RectPayload` 是 f64：
// 高 DPI 屏上逻辑坐标 = 物理像素 / scaleFactor 必然出现小数
// （Retina 2x、Windows 125%/150%）。**不要**把字段改成 i32/u32 ——
// serde 会在反序列化阶段直接拒收，报
// `invalid type: floating point '484.71', expected u32`，
// 而这种失败只在非 100% 缩放的机器上复现。转物理像素时统一 round。

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

/** 标注图形载荷（confirm_selection 的 shapes 字段，逻辑像素、原点=裁剪图左上角） */
type ShapePayload =
  | { type: 'rect'; x: number; y: number; w: number; h: number; color: Rgba; width: number }
  | { type: 'ellipse'; x: number; y: number; w: number; h: number; color: Rgba; width: number }
  | { type: 'arrow'; x1: number; y1: number; x2: number; y2: number; color: Rgba; width: number }
  | { type: 'pen'; points: [number, number][]; color: Rgba; width: number }
  | { type: 'highlighter'; points: [number, number][]; color: Rgba; width: number }
  | { type: 'mosaic'; x: number; y: number; w: number; h: number; block: number }
  // 聚光灯：区域外压暗。radius 圆角半径（逻辑像素，核心层 clamp 到短边一半），
  // dim 压暗强度 0-255（乘性压暗，不影响 alpha）
  | { type: 'spotlight'; x: number; y: number; w: number; h: number; radius: number; dim: number }
  // 文字/序号等"需字形的标注"由 UI 层光栅化成位图上传（见 ARCHITECTURE 三·5）
  // png: base64（无 data: 前缀），尺寸为物理像素；x/y 仍为逻辑像素
  // 序号徽标复用此载荷：圆底+数字已在 UI 层画进位图，核心层无感知
  | { type: 'bitmap'; x: number; y: number; png: string }

type Rgba = [number, number, number, number]   // 0-255

interface AppConfig {
  themeColor: string
  ballSize: number
  shortcutCapture: string        // 全局快捷键，可改（设置窗 → 快捷键），改后热重注册
  shortcutPinClipboard: string
  saveDir: string | null
}

interface AppInfo { version: string; platform: string; arch: string }

/** 自绘菜单初始状态（menu_state） */
interface MenuState {
  /** 是否处于打开流程中（窗口刚建好时前端收不到 menu-open，靠它补一次 fit） */
  open: boolean
  /** 注册失败的内置快捷键名（空 = 正常）；非空时菜单顶部显示告警行 */
  shortcutFailed: string[]
  /** 当前菜单种类（首次创建菜单窗时 menu-open 会丢，靠它补齐内容） */
  kind: 'app' | 'pin'
  /** 贴图菜单的目标窗口 label（kind = pin 时有值） */
  pinLabel: string | null
}

/** 贴图窗状态快照（pin_publish_flags / pin_flags）；无记录时取默认值。
 *  既含界面开关，也含**图像处理参数**——菜单要画勾选与当前档位。 */
interface PinFlags {
  topmost: boolean     // 窗口置顶（右键菜单 / 工具条 / T 键）
  locked: boolean      // 锁定位置（右键菜单 / 工具条 / L 键）
  shadow: boolean      // 边框阴影（内边框 + 外发光，右键菜单 / Y 键）
  toolbar: boolean     // 悬停是否显示工具条（右键菜单"工具栏" / H 键；关掉后只剩图片）
  opacity: number      // 窗口整体不透明度 0.15~1（菜单顶部滑块 / 工具条滑条 / Alt+滚轮，三者同源）
  rotate: number       // 顺时针旋转 0 / 90 / 180 / 270（菜单 1 / 2 键）
  flipH: boolean       // 水平翻转（3 键）
  flipV: boolean       // 垂直翻转（4 键）
  gray: boolean        // 灰度化（5 键）
  invert: boolean      // 反色（6 键）
  alpha: number        // 图像自身 alpha（菜单"图像处理 → 图像不透明度"，会烤进像素）
  brightness: number   // 亮度系数，1 = 原始（菜单"调整亮度"）
}
```

## 自绘菜单协议（菜单窗）

托盘图标点击、悬浮球右键都弹出**同一个自绘菜单窗**（label `ball-menu`，
透明无边框 webview）。原生 `TrackPopupMenu` 做不出深色圆角 / 分组标题 /
右侧快捷键提示，故整体自绘。三段式避免尺寸抖动：

1. Rust 记录锚点（物理屏幕坐标）、把窗口移出屏幕、emit `menu-open`；
2. 前端渲染后用 `offsetWidth/offsetHeight` + 阴影留白量出窗口尺寸 → `menu_fit`；
3. Rust 按锚点定位（越界翻转）→ resize → 移到锚点 → 抢焦点。

关闭：失焦（点外部）由 Rust 处理；`Esc` 与"点面板外透明留白"由前端调 `menu_close`。
⚠️ 窗口**始终 `visible`**（只把坐标挪到 -32000）：WebView2 对不可见窗口会暂停
`requestAnimationFrame`，隐藏着的菜单窗测不出尺寸。因此"是否打开"一律以锚点判断。

菜单项 id 与托盘动作共用（`capture` / `pin-clipboard` / `toggle-ball` /
`about` / `open-log` / `quit`），由 `menu_action` 汇入同一分发函数。

**贴图右键菜单**复用同一窗口，但 `menu-open` 载荷带 `{ kind: "pin", pinLabel }`：

- 前端据此渲染贴图项组（浅色面板与托盘菜单共用同一套样式），顺序为：
  **不透明度〔滑块〕** → `图像处理 ▸`（向右旋转 1 / 向左旋转 2 / 水平翻转 3 /
  垂直翻转 4 / 灰度化 5 / 反色 6 / 图像不透明度 ▸ / 调整亮度 ▸ / 重置所有处理 0）→
  复制图像 / 保存图像 → 鼠标穿透 → ✓边框阴影 Y / ✓窗口置顶 T / 锁定 L / ✓工具栏 H →
  关闭 Esc；打开时用 `pin_flags(pinLabel)` 读勾选与滑块位置；
- **滑块是第一项**（透明度最常调，不该埋在二级菜单里）：拖动时**不走**
  `menu_action`（那条路会先关菜单），而是逐个值调 `pin_set_opacity`，
  Rust 转成 `pin-set-opacity` 事件给贴图窗实时 setState —— 菜单不关、拖动即所见。
  为什么不能由菜单直接改：贴图窗的透明度是它 CSS 上的 `opacity`，跨 webview 改不到；
  与「图像不透明度」区分：滑块只改显示，图像不透明度会把 alpha 烤进像素；
- 「工具栏」开关关掉后，贴图只剩图片（拖动与右键菜单仍可用），
  用于"贴在别处当参照"时不希望工具条悬停弹出；状态同样回写快照以画勾选；
- 子菜单（▸）是**同一份 DOM 里预先量好**的：`menu-fit` 的尺寸包含所有隐藏子菜单的
  外扩范围，展开时不再 resize 窗口（否则窗口在指针下方跳动、hover 被打断）；
  代价是菜单贴屏幕右边缘时，右侧会留出一条给子菜单的空白（窗口整体翻到光标左侧）；
- 尺寸：**贴图一级菜单 200px**（`.menu-stage.is-pin .sp-menu.sp-root`；托盘/悬浮球菜单仍 224px），
  二级菜单 `max-content`（最小 152px）；一二级之间 **1px** 缝——
  `left: calc(100% + var(--sp-pad) + 1px)`，其中 `--sp-pad` 是面板内边距（6px）。
  必须补上 `--sp-pad`：`.sp-node` 的 100% 是内容盒宽度，只写 `calc(100% + 1px)`
  会让子菜单压进父面板 5px（看似"贴住"，实则重叠）；
- 点击后 `menu_action(id)` 在 Rust 侧看到锚点 `kind = Pin` → 不执行应用动作，
  改向 `pinLabel` 窗口 emit `pin-menu-action { id }`；
- 动作的实际执行在贴图窗的 React 里（界面状态的事实来源），执行后由
  `pin_publish_flags` 回写快照。

## 事件

| 事件 | 载荷 | 说明 |
|------|------|------|
| `menu-open` | `{ kind: "app" \| "pin", pinLabel: string \| null }` | 菜单窗收到"请测量并显示"（发往 `ball-menu` 窗口） |
| `pin-set-opacity` | `{ value: number }` | 菜单滑块实时值 → 目标贴图窗（发往 pin 窗口） |
| `pin-menu-action` | `{ id: string }` | 贴图菜单项点击 → 目标贴图窗执行（id 见 PinWindow 动作表） |
| `long-capture-progress` | `LongProgress` | 长截图驱动循环每步进度（缩略图 + 段数 + 状态） |

规划中：

| 事件 | 载荷 | 说明 |
|------|------|------|
| `capture-state` | `{ phase: 'idle' \| 'selecting' \| 'recording' }` | 悬浮球状态机（录屏计时等） |

## 窗口 URL 约定

每个窗口是独立的 HTML 入口（vite 多入口，窗口类型由内联全局
`__SNAPPIN_WINDOW__` 声明，不走 URL 查询串——避免 `App(PathBuf)` 的编码问题）：

| 窗口 | HTML 入口 |
|------|-----------|
| 悬浮球 `ball` | `ball.html` |
| 截图浮层 `overlay-N` | `overlay.html` |
| 贴图 `pin-*` | `pin.html` |
| 自绘菜单 `ball-menu` | `menu.html` |
| 主窗 `main` | `main.html` |

## 坐标系统一约定

- 前端选区 rect：overlay 窗口的逻辑像素坐标
- Rust 裁剪：`rect × scaleFactor` → 物理像素后调用 `core-algo::crop`
- 贴图显示尺寸：`物理像素 / scaleFactor` → 逻辑像素
