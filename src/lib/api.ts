/**
 * 前端 IPC 契约层。
 *
 * 规则（架构约束④：前端零耦合）：
 * - 本文件是前后端唯一契约，TS 类型在此独立声明，不 import 任何 Rust 类型；
 * - Rust 命令参数为 snake_case，Tauri v2 自动把前端的 camelCase 转为 snake_case；
 * - 鸿蒙 PC 壳将来只需按同一份契约实现适配层，UI 代码零改动。
 */
import { invoke, convertFileSrc } from '@tauri-apps/api/core'

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

export const startCapture = (): Promise<CapturePayload> => invoke('start_capture')

export const overlayContext = (): Promise<OverlayContext> => invoke('overlay_context')

export const confirmSelection = (
  monitorId: string,
  rect: Rect,
  action: ShotAction,
): Promise<{ path: string }> =>
  invoke('confirm_selection', { monitorId, rect, action })

export const cancelCapture = (): Promise<void> => invoke('cancel_capture')

export const pinFromClipboard = (): Promise<string> => invoke('pin_from_clipboard')

export const pinContext = (): Promise<PinContext> => invoke('pin_context')

export const pinClose = (label: string): Promise<void> => invoke('pin_close', { label })

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
