/**
 * 前端 IPC 契约层。
 *
 * 规则（架构约束④：前端零耦合）：
 * - 本文件是前后端唯一契约，TS 类型在此独立声明，不 import 任何 Rust 类型；
 * - Rust 命令参数为 snake_case，Tauri v2 自动把前端的 camelCase 转为 snake_case；
 * - 鸿蒙 PC 壳将来只需按同一份契约实现适配层，UI 代码零改动。
 */
import { invoke, convertFileSrc } from '@tauri-apps/api/core'
import { getCurrentWindow } from '@tauri-apps/api/window'

// ---------- 共享类型 ----------

export interface Rect {
  x: number
  y: number
  w: number
  h: number
}

export interface MonitorInfo {
  id: string
  name: string
  x: number
  y: number
  width: number
  height: number
  scaleFactor: number
}

export interface ShotAsset {
  monitorId: string
  path: string
}

export interface CapturePayload {
  monitors: MonitorInfo[]
  shots: ShotAsset[]
}

export interface OverlayContext {
  monitorId: string
  imagePath: string
  /** 截图物理像素宽 */
  naturalWidth: number
  /** 截图物理像素高 */
  naturalHeight: number
  scaleFactor: number
  /**
   * false = 采集中（轮询等待）或会话已结束（取消/确认成功后 Rust 清空会话，
   * 前端检测到 ready 由 true→false 时自关窗口兜底）；true = 冻结图就绪
   */
  ready: boolean
  /** 本会话代号：取消/确认时回传，Rust 据此丢弃上一轮迟到的请求 */
  sessionId: number
}

export type ShotAction = 'copy' | 'pin' | 'save'

export type BallCommand = 'capture' | 'pin-clipboard' | 'hide' | 'quit'

export interface PinContext {
  label: string
  /** 贴图图片的本地路径 */
  src: string
  /** 逻辑像素显示宽 */
  width: number
  /** 逻辑像素显示高 */
  height: number
}

export interface AppConfig {
  themeColor: string
  ballSize: number
  ballPosition: { x: number; y: number } | null
  shortcutCapture: string
  shortcutPinClipboard: string
  saveDir: string | null
}

export interface AppInfo {
  version: string
  platform: string
  arch: string
}

// ---------- 资源路径 ----------

/** 把本地文件路径转为 webview 可加载的 URL（依赖 assetProtocol 配置）。 */
export const imgSrc = (path: string): string => convertFileSrc(path)

// ---------- 命令封装 ----------

/**
 * 前端诊断日志：release 构建没有 DevTools，日志统一汇到 Rust stderr，
 * 与后端日志共用一条时间线（排查"浮层不显示/采集超时"用）。
 */
export const uiLog = (msg: string): void => {
  invoke('ui_log', { msg }).catch(() => {})
}

export const startCapture = (): Promise<CapturePayload> => invoke('start_capture')

export const overlayContext = (): Promise<OverlayContext> => invoke('overlay_context')

/**
 * 确认选区并执行动作。
 * `shapes`：标注列表（M3，可省略）。坐标为逻辑像素、原点=裁剪图左上角，
 * Rust 侧统一乘 DPI 缩放因子转物理像素后光栅化到裁剪图。
 */
export const confirmSelection = (
  monitorId: string,
  rect: Rect,
  action: ShotAction,
  shapes?: unknown[],
  sessionId?: number,
): Promise<{ path: string }> =>
  invoke('confirm_selection', { monitorId, rect, action, shapes, sessionId })

/**
 * 取消截图。
 * 传 `sessionId` 时 Rust 会校验代号：上一轮迟到的取消不会误杀新会话。
 * 采集尚未完成（还没有 sessionId）时传 undefined，Rust 会登记取消请求，
 * 在显示浮层之前消费。
 */
export const cancelCapture = (sessionId?: number): Promise<void> =>
  invoke('cancel_capture', { sessionId })

/**
 * 悬浮球右键菜单：Rust 在光标位置弹出原生菜单。
 * （WebView2 默认浏览器右键菜单已被 webview 初始化脚本抑制。）
 */
export const ballMenu = (): Promise<void> => invoke('ball_menu')

/** 关闭调用方窗口自身（浮层"完全退出截图模式"的自关兜底）。 */
export const closeSelf = (): Promise<void> => getCurrentWindow().close()

export const pinFromClipboard = (): Promise<string> => invoke('pin_from_clipboard')

export const pinContext = (): Promise<PinContext> => invoke('pin_context')

export const pinClose = (label: string): Promise<void> => invoke('pin_close', { label })

/** 切换贴图窗置顶（置顶时压在其他窗口之上）。 */
export const pinSetTopmost = (label: string, topmost: boolean): Promise<void> =>
  invoke('pin_set_topmost', { label, topmost })

/**
 * 切换贴图窗鼠标穿透。
 * 穿透后本窗不收鼠标事件（工具条也点不到），
 * 前端需配合定时自动恢复，见 PinWindow。
 */
export const pinSetIgnoreCursorEvents = (label: string, ignore: boolean): Promise<void> =>
  invoke('pin_set_ignore_cursor_events', { label, ignore })

export const pinCopy = (label: string): Promise<void> => invoke('pin_copy', { label })

export const pinSave = (label: string): Promise<void> => invoke('pin_save', { label })

export const ballCommand = (cmd: BallCommand): Promise<void> =>
  invoke('ball_command', { cmd })

/** 对调用方窗口启动原生拖拽（OS 级移动循环）。 */
export const dragWindow = (): Promise<void> => invoke('drag_window')

export const getConfig = (): Promise<AppConfig> => invoke('get_config')

export const updateConfig = (config: AppConfig): Promise<AppConfig> =>
  invoke('update_config', { config })

export const appInfo = (): Promise<AppInfo> => invoke('app_info')
