/**
 * 前端 IPC 契约层。
 *
 * 规则（架构约束④：前端零耦合）：
 * - 本文件是前后端唯一契约，TS 类型在此独立声明，不 import 任何 Rust 类型；
 * - Rust 命令参数为 snake_case，Tauri v2 自动把前端的 camelCase 转为 snake_case；
 * - 鸿蒙 PC 壳将来只需按同一份契约实现适配层，UI 代码零改动。
 */
import { invoke, convertFileSrc } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
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

/** 自绘菜单的初始状态（打开时由菜单窗拉取）。 */
export interface MenuState {
  /** 是否处于打开流程中（窗口刚建好时前端收不到 emit，靠它补一次 fit） */
  open: boolean
  /** 注册失败的内置快捷键名（空 = 全部正常）；非空时菜单顶部显示告警行 */
  shortcutFailed: string[]
  /** 当前菜单种类：app = 托盘/悬浮球，pin = 贴图右键。
   *  首次创建菜单窗时 `menu-open` 事件会丢，靠它补上正确内容。 */
  kind: 'app' | 'pin'
  /** 贴图菜单的目标窗口 label（kind = pin 时有值） */
  pinLabel: string | null
}

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
  /** 悬浮球不透明度 0.15~1 */
  ballOpacity: number
  shortcutCapture: string
  shortcutPinClipboard: string
  saveDir: string | null
  /** 翻译配置（在线 API，OpenAI 兼容协议；Key 由用户自填，只存本地） */
  translation: TranslationConfig
}

/** 一个自定义模型（与 Rust `TranslateModel` 对应）。 */
export interface TranslateModel {
  id: string
  /** 显示名 */
  name: string
  /** 副标题（如 "深度搜索"） */
  note: string
  /** 端点根地址，不带 /chat/completions */
  baseUrl: string
  /** API Key；空 = 未配置（点翻译会引导来设置窗） */
  apiKey: string
  /** 模型 ID */
  model: string
  /** 「获取 Key」的网页 */
  keyUrl: string
}

/** 翻译配置（与 Rust `TranslateConfig` 对应）。models 第一个 = 当前启用。 */
export interface TranslationConfig {
  models: TranslateModel[]
  /** 目标语言，如 "英文" / "日文" */
  targetLang: string
}

/** 翻译窗状态（trans_state 返回 + trans-update 事件载荷）。 */
export interface TransState {
  original: string
  translated: string
  /** 失败原因（中文可操作文案；空 = 没出错） */
  error: string
  busy: boolean
  targetLang: string
  /** 产出本次译文的模型名（空 = 还没翻译） */
  modelName: string
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

/**
 * 快捷键录制期间挂起/恢复全局热键。
 *
 * 不挂起的话：录制时按下"当前已注册"的组合（比如照着 Ctrl+1 再按一次），
 * 全局热键会立刻触发截图，浮层盖住设置页 —— 用户会以为设置功能坏了。
 */
export const hotkeysSuspend = (): Promise<void> => invoke('hotkeys_suspend')
export const hotkeysResume = (): Promise<void> => invoke('hotkeys_resume')

/**
 * 缩放贴图：真改窗口尺寸（图片按新尺寸渲染）。
 *
 * 不用 CSS `transform`：transform 只改绘制不改窗口，放大超出窗口就被裁掉、
 * 缩小则四周留透明边——这是路线图 M2 里遗留的"贴图缩放同步改窗口"。
 */
/** 贴图分组（多贴图管理）：分组清单 / 当前分组 / 换组 */
export const pinGroups = (): Promise<string[]> => invoke('pin_groups')
export const pinGroupOf = (label: string): Promise<string> =>
  invoke('pin_group_of', { label })
export const pinSetGroup = (label: string, group: string): Promise<string[]> =>
  invoke('pin_set_group', { label, group })

export const pinSetDisplaySize = (
  label: string,
  width: number,
  height: number,
): Promise<void> => invoke('pin_set_display_size', { label, width, height })

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

/** 识别到的码（M4 二维码）。
 *  `points` 为定位点，**相对选区左上角的逻辑像素坐标**，可直接画高亮框。 */
export interface DecodedCode {
  text: string
  format: string
  points: [number, number][]
}

/**
 * 识别选区内的二维码 / 条码（M4，支持一图多码）。
 * 没有任何码时返回空数组（不是错误）。
 */
export const decodeQr = (
  monitorId: string,
  rect: Rect,
  sessionId?: number,
): Promise<DecodedCode[]> => invoke('decode_qr', { monitorId, rect, sessionId })

/** 复制纯文本到剪贴板（取色器复制 HEX、二维码复制内容都用它）。 */
export const copyText = (text: string): Promise<void> => invoke('copy_text', { text })

/**
 * 用系统默认浏览器打开链接（二维码识别结果的"打开链接"按钮）。
 * Rust 侧只放行 http/https——二维码内容不可信，其它协议一律拒绝。
 */
export const openUrl = (url: string): Promise<void> => invoke('open_url', { url })

// ---------- 文字识别（M4 OCR） ----------

/** 一行识别结果。
 *  框为相对**选区左上角**的逻辑像素，与二维码定位点同口径，可直接画高亮框。 */
export interface OcrLine {
  text: string
  /** 识别置信度 0..1 */
  score: number
  minX: number
  minY: number
  maxX: number
  maxY: number
}

/** 悬停吸附到的窗口（逻辑坐标，与浮层选区同一坐标系）。 */
export interface WindowSnap {
  x: number
  y: number
  w: number
  h: number
  /** 窗口标题；空串表示该窗口无标题 */
  title: string
}

/**
 * 查询光标处应吸附的顶层窗口（QQ/微信截图那种"悬停识窗、单击选中"）。
 * 返回 null = 光标下只有本应用自己的窗口，不该高亮。
 */
export const windowSnap = (
  monitorId: string,
  x: number,
  y: number,
): Promise<WindowSnap | null> =>
  invoke<WindowSnap | null>('window_snap', { monitorId, x, y })

/** OCR 模型状态（前端据此提示下载，而不是等点下去才报错）。 */
export interface OcrModelsStatus {
  available: boolean
  /** 模型目录（缺失时告诉用户该往哪放） */
  dir: string
}

/**
 * 识别选区内的文字（M4 OCR：PP-OCRv4 mobile + rten，中文场景）。
 * 模型缺失时 reject，错误信息里带模型目录与下载说明。
 */
export const ocrRecognize = (
  monitorId: string,
  rect: Rect,
  sessionId?: number,
): Promise<OcrLine[]> => invoke('ocr_recognize', { monitorId, rect, sessionId })

/** 查询 OCR 模型是否就绪（不触发模型加载）。 */
export const ocrModelsStatus = (): Promise<OcrModelsStatus> =>
  invoke('ocr_models_status')

/** 下载 OCR 模型三件套（15.6MB，幂等：已校验通过的文件会跳过） */
export const ocrDownloadModels = (): Promise<{ available: boolean; dir: string }> =>
  invoke('ocr_download_models')

// ---------- 长截图（M5） ----------

/** 长截图进度事件（Rust 侧 `emit("long-capture-progress", payload)`）。 */
export interface LongProgress {
  /** "scrolling" 滚动拼接中 | "done" 正常结束 | "error" 出错 */
  phase: 'scrolling' | 'done' | 'error'
  /** 当前累积高度（物理像素） */
  height: number
  /** 累计步数 */
  steps: number
  /** 最近一次匹配置信度 0..1 */
  score: number
  /** 缩略图临时 PNG 路径（imgSrc 转换后加载） */
  thumb: string
  /** 状态条文案 */
  message: string
}

/**
 * 开始长截图：把当前选区交给 Rust 自动滚动拼接。
 * `rect` 为**显示器逻辑像素**（与 confirm_selection 同口径）。
 * 启动失败时也会收到 phase="error" 的进度事件（前端据此复位）。
 */
export const longCaptureStart = (
  monitorId: string,
  rect: Rect,
  scrollLines?: number,
  sessionId?: number,
): Promise<void> =>
  invoke('long_capture_start', { monitorId, rect, scrollLines, sessionId })

/** 停止长截图滚动（Esc / 前端停止按钮）：驱动循环收尾后发 done 事件。 */
export const longCaptureStop = (): Promise<void> => invoke('long_capture_stop')

/**
 * 长截图收尾出口：copy=复制到剪贴板 / pin=贴图 / save=保存。
 * 返回最终长图的临时文件路径（save 时为用户选定的路径）。
 */
export const longCaptureCommit = (
  action: ShotAction,
): Promise<{ path: string }> => invoke('long_capture_commit', { action })

/**
 * 悬浮球右键菜单：在光标处打开**自绘菜单窗**。
 *
 * 不再是原生 popup_menu——原生菜单样式不可控（浅色、无分组标题、
 * 右侧不能放快捷键提示），见 src-tauri/src/menuwin.rs。
 * （WebView2 默认浏览器右键菜单已被 webview 初始化脚本抑制。）
 */
export const ballMenu = (): Promise<void> => invoke('ball_menu')

/** 菜单窗量出面板尺寸后回报；Rust 据此定位并显示窗口。 */
export const menuFit = (width: number, height: number): Promise<void> =>
  invoke('menu_fit', { width, height })

/** 关闭菜单（Esc / 点到面板外的透明留白）。 */
export const menuClose = (): Promise<void> => invoke('menu_close')

/** 菜单初始状态（是否打开、快捷键是否注册失败）。 */
export const menuState = (): Promise<MenuState> => invoke('menu_state')

/** 点击菜单项：Rust 先关菜单，再按 id 分发动作。 */
export const menuAction = (id: string): Promise<void> => invoke('menu_action', { id })

/**
 * 贴图窗状态快照（界面开关 + 图像处理参数）。
 *
 * 双向：贴图窗上报给 Rust 缓存，右键菜单再读回来画勾选与当前档位。
 * 字段都是"显示态"，不是累积操作——处理永远从原图重算（见 PinWindow）。
 */
export interface PinFlags {
  topmost: boolean
  locked: boolean
  shadow: boolean
  /** 悬停是否显示工具条（右键菜单"工具栏" / H 键） */
  toolbar: boolean
  /** 窗口整体不透明度（顶部滑块 / 工具条滑条 / Alt+滚轮），0.15 ~ 1 */
  opacity: number
  /** 顺时针旋转角度：0 / 90 / 180 / 270 */
  rotate: number
  flipH: boolean
  flipV: boolean
  gray: boolean
  invert: boolean
  /** 图像自身 alpha（菜单"图像处理 → 图像不透明度"） */
  alpha: number
  /** 亮度系数，1 = 原始 */
  brightness: number
}

/** 贴图窗右键：在光标处打开自绘菜单（Rust 记住目标为本窗口）。 */
export const pinMenuOpen = (): Promise<void> => invoke('pin_menu_open')

/** 贴图窗上报自己的状态快照（供右键菜单画勾选标记与当前档位）。 */
export const pinPublishFlags = (label: string, flags: PinFlags): Promise<void> =>
  invoke('pin_publish_flags', { label, flags })

/** 读取贴图状态快照（无记录时 Rust 返回默认值）。 */
export const pinFlags = (label: string): Promise<PinFlags> => invoke('pin_flags', { label })

/**
 * 应用一次图像处理：前端把 canvas 处理结果编码成 base64 PNG 上传，
 * Rust 落成新临时文件、换掉贴图源图、按新尺寸（中心不动）调整窗口。
 * 返回新的上下文（src / width / height）。
 */
export const pinApplyEdit = (
  label: string,
  pngBase64: string,
  width: number,
  height: number,
): Promise<PinContext> =>
  invoke('pin_apply_edit', { label, pngBase64, width, height })

/** 贴图菜单项点击（Rust → 贴图窗）。id 见 PinWindow 的动作表。 */
export const onPinMenuAction = (cb: (id: string) => void): Promise<UnlistenFn> =>
  listen<{ id: string }>('pin-menu-action', (e) => cb(e.payload.id))

/** 菜单「不透明度」滑块把实时值转给目标贴图窗（菜单窗 → Rust → 贴图窗）。 */
export const pinSetOpacity = (label: string, value: number): Promise<void> =>
  invoke('pin_set_opacity', { label, value })

/** 贴图窗接收滑块实时值（拖动过程中连续到达）。 */
export const onPinSetOpacity = (cb: (value: number) => void): Promise<UnlistenFn> =>
  listen<{ value: number }>('pin-set-opacity', (e) => cb(e.payload.value))

/** 关闭调用方窗口自身（浮层"完全退出截图模式"的自关兜底）。 */
export const closeSelf = (): Promise<void> => getCurrentWindow().close()

export const pinFromClipboard = (): Promise<string> => invoke('pin_from_clipboard')

/** 读取图片原始字节（PNG）→ ArrayBuffer。
 *  取色/放大镜要读像素，而 asset 协议的 <img> 读像素会被判跨域污染；
 *  由 Rust 直接给字节，前端再构造 Blob → ImageBitmap，读取永远合法。 */
export const readImageBytes = (path: string): Promise<ArrayBuffer> =>
  invoke<ArrayBuffer>('read_image_bytes', { path })

export const pinContext = (): Promise<PinContext> => invoke('pin_context')

export const pinClose = (label: string): Promise<void> => invoke('pin_close', { label })

/** 贴图记录（管理面板用） */
export interface PinInfo {
  label: string
  /** 当前显示图片的本地路径（处理后可能指向临时文件） */
  src: string
  width: number
  height: number
  group: string
}

/** 列出所有贴图 */
/** 长截图结果的**全分辨率**临时 PNG 路径（最大化时按 1:1 展示用，按需生成） */
export const pinList = (): Promise<PinInfo[]> => invoke('pin_list')

/** 把某张贴图置前 */
export const pinFocus = (label: string): Promise<void> => invoke('pin_focus', { label })

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

/** 悬浮球实时预览（拖滑条时调用，不落盘） */
export const ballPreview = (size: number, opacity: number): Promise<void> =>
  invoke('ball_preview', { size, opacity })

export const ballCommand = (cmd: BallCommand): Promise<void> =>
  invoke('ball_command', { cmd })

/** 对调用方窗口启动原生拖拽（OS 级移动循环）。 */
export const dragWindow = (): Promise<void> => invoke('drag_window')

export const getConfig = (): Promise<AppConfig> => invoke('get_config')

export const updateConfig = (config: AppConfig): Promise<AppConfig> =>
  invoke('update_config', { config })

export const appInfo = (): Promise<AppInfo> => invoke('app_info')

// ---------- OCR 结果窗（独立窗口：可选中 / 可复制 / 可翻译） ----------

/**
 * 打开 OCR 结果窗（识别成功后调用）。行数据先存 Rust 侧，窗口通过
 * `ocr-open` 事件刷新，因此重复识别时已开着的窗口内容会跟着更新。
 */
export const ocrShow = (lines: OcrLine[]): Promise<void> =>
  invoke('ocr_show', { lines })

/** `ocr_result` 的返回体：识别行 + 原图路径（结果窗左栏显示用）。 */
export interface OcrPayload {
  lines: OcrLine[]
  image: string | null
}

/** OCR 结果窗内容（挂载时 / 收到 ocr-open 时调用）。 */
export const ocrResult = (): Promise<OcrPayload> => invoke('ocr_result')

/** 关闭 OCR 结果窗。 */
export const ocrClose = (): Promise<void> => invoke('ocr_close')

/** OCR 结果窗打开/刷新事件（Rust `emit("ocr-open")`）。 */
export const onOcrOpen = (cb: () => void): Promise<UnlistenFn> =>
  listen('ocr-open', () => cb())

// ---------- 翻译窗（左原文右译文） ----------

/**
 * 开始翻译并打开翻译窗。
 *
 * 同步返回：窗口立刻显示"翻译中"，译文通过 `trans-update` 事件推过来，
 * 不阻塞调用方（网络请求在 Rust 阻塞线程里跑）。
 */
export const transStart = (text: string): Promise<void> =>
  invoke('trans_start', { text })

/** 测试某个模型的连通性（设置窗「模型」页用）。 */
export const transTest = (id: string): Promise<void> =>
  invoke('trans_test', { id })

/** 翻译窗当前状态（挂载时拉一次）。 */
export const transState = (): Promise<TransState> => invoke('trans_state')

/** 关闭翻译窗（状态保留，下次打开接着看）。 */
export const transClose = (): Promise<void> => invoke('trans_close')

/** 翻译进度/结果事件（Rust `emit("trans-update", TransState)`）。 */
export const onTransUpdate = (cb: (s: TransState) => void): Promise<UnlistenFn> =>
  listen<TransState>('trans-update', (e) => cb(e.payload))

// ---------- 设置窗 ----------

/** 打开设置窗（单例；翻译未配 Key 时的"去设置"入口）。 */
export const openSettingsWindow = (): Promise<void> =>
  invoke('open_settings_window')
