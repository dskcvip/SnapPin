import { emit, listen } from '@tauri-apps/api/event'
import { getCurrentWindow } from '@tauri-apps/api/window'
import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
  type CSSProperties,
  type Dispatch,
  type PointerEvent as ReactPointerEvent,
  type SetStateAction,
} from 'react'
import {
  accessibilityGranted,
  accessibilityReset,
  accessibilitySettingsOpen,
  cancelCapture,
  closeSelf,
  confirmSelection,
  copyText,
  decodeQr,
  imgSrc,
  longCaptureCommit,
  longCaptureStart,
  longCaptureStop,
  longResultOpen,
  openUrl,
  ocrModelsStatus,
  ocrDownloadModels,
  ocrRecognize,
  ocrShow,
  overlayContext,
  readImageBytes,
  transStart,
  uiLog,
  windowSnap,
  type DecodedCode,
  type LongProgress,
  type OverlayContext,
  type Rect,
  type ShotAction,
  type WindowSnap,
} from '../lib/api'
import type { StickerPick } from '../stickers/library'
import { AnnotationLayer } from '../annotate/AnnotationLayer'
import { DragMagnifier } from '../annotate/DragMagnifier'
import { ToolProps } from '../annotate/ToolProps'
import {
  ArrowIcon,
  CheckIcon,
  CloseIcon,
  EllipseIcon,
  EraserIcon,
  LongShotIcon,
  MosaicIcon,
  OcrIcon,
  PenIcon,
  PinIcon,
  QrIcon,
  RectIcon,
  SaveIcon,
  SequenceIcon,
  // ShareIcon,  // 分享按钮已按要求注释掉（见工具条处注释）
  SmartEraseIcon,
  SpotlightIcon,
  StickerIcon,
  TextIcon,
  TranslateIcon,
  RedoIcon,
  UndoIcon,
  WatermarkIcon,
} from '../annotate/icons'
import {
  ERASER_RADIUS,
  MOSAIC_BLOCKS,
  PRESET_COLORS,
  SPOTLIGHT_DIM,
  STICKER_DEFAULT,
  STICKER_COLORS,
  TEXT_FONT_SIZE,
  ZOOM_LEVELS,
  isActionTool,
  toShapePayloads,
  toolMeta,
  type AnnotateTool,
  type Rgba,
  type ShapeDraft,
} from '../annotate/tools'
import '../styles/overlay.css'

const MIN_SIZE = 3

/**
 * 跨屏"只保留一个选区"的事件名。
 *
 * 双屏时每块屏各有一个全屏浮层（各自独立的 React 实例），用户在两块屏上
 * 先后框选，两边就各留一个选区 + 各挂一条工具条（用户实测截图：两块屏
 * 各一个框、各一条工具栏）。一次截图只能有一个选区，所以谁开始画，
 * 就广播让别的屏把自己的选区清掉。
 */
/** 跨屏"只保留一个悬停高亮"的事件名（选中同理见 EVT_CLEAR_OTHER_SEL）。 */
const EVT_HOVER_OWNER = 'overlay-hover-owner'
const EVT_CLEAR_OTHER_SEL = 'overlay-clear-selection'

/** 选区锚点：四角 + 四边中点（边的热区是透明的，只给抓手和光标）。 */
/**
 * 选区缩放热区：四条边（整条线都能拖）+ 四角。
 *
 * **顺序有意义**：先渲染边、后渲染角 —— DOM 里靠后的盖在上面，
 * 这样角部（双轴缩放）优先于边部（单轴），否则贴角拖动只会改一个方向。
 */
const CORNER_HANDLES = ['t', 'r', 'b', 'l', 'tl', 'tr', 'br', 'bl'] as const
type HandleEdge = (typeof CORNER_HANDLES)[number]

/**
 * 按拖动的边/角推算出新选区。
 *
 * 支持拖过对边（宽/高变负时自动翻转，和系统级截图工具一致），
 * 并把结果钳进视口——否则往外拖会把选区推到屏幕外，工具栏也跟着跑丢。
 */
function resizeRect(start: Rect, edge: HandleEdge, dx: number, dy: number): Rect {
  let { x, y, w, h } = start
  if (edge.includes('l')) {
    x += dx
    w -= dx
  }
  if (edge.includes('r')) w += dx
  if (edge.includes('t')) {
    y += dy
    h -= dy
  }
  if (edge.includes('b')) h += dy
  // 拖过头：翻转而不是变成负数
  if (w < 0) {
    x += w
    w = -w
  }
  if (h < 0) {
    y += h
    h = -h
  }
  // 钳进视口
  const vw = window.innerWidth
  const vh = window.innerHeight
  if (x < 0) {
    w += x
    x = 0
  }
  if (y < 0) {
    h += y
    y = 0
  }
  if (x + w > vw) w = vw - x
  if (y + h > vh) h = vh - y
  return {
    x: Math.round(x),
    y: Math.round(y),
    w: Math.max(Math.round(w), MIN_SIZE),
    h: Math.max(Math.round(h), MIN_SIZE),
  }
}

/** RGB → `#RRGGBB`（大写十六进制，与主流取色器的输出一致）。 */
function toHex([r, g, b]: [number, number, number]): string {
  const h = (v: number) => v.toString(16).padStart(2, '0')
  return `#${h(r)}${h(g)}${h(b)}`.toUpperCase()
}

const ACTION_LABEL: Record<ShotAction, string> = {
  copy: '复制',
  pin: '贴图',
  save: '保存',
}

/**
 * 截图选区浮层窗口（每个显示器一个）。
 * 流程：Rust 截取全部显示器 → 打开等尺寸透明浮层 → 前端展示冻结图 →
 * 用户拖拽选区 → **顶部工具条立即出现**（绘制标注 / 撤销 / 保存 / 贴图 /
 * 取消 / 完成）→ 动作按钮直接把「选区 + 标注」交给 Rust
 * （裁剪 + 光栅化 + 执行）→ 关闭全部浮层。
 *
 * 交互参考主流截图工具：选中即出工具条，边画边看，无需模式切换。
 *
 * 退出截图模式三级保障（双屏下任一环节漏关都不残留）：
 * ① Rust end_capture_session：关闭全部浮层 + 清空会话数据；
 * ② 本窗自关：取消/确认成功后 closeSelf()；
 * ③ 轮询兜底：发现 ready 由 true→false（会话被结束）时自关。
 */
export function OverlayWindow() {
  const [ctx, setCtx] = useState<OverlayContext | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [rect, setRect] = useState<Rect | null>(null)
  const [draft, setDraft] = useState<Rect | null>(null)
  const [cursor, setCursor] = useState<{ x: number; y: number } | null>(null)
  /** 悬停吸附到的窗口（还没出选区时高亮它；单击即选中整窗） */
  const [hoverWin, setHoverWin] = useState<WindowSnap | null>(null)
  const [busy, setBusy] = useState(false)
  const [actError, setActError] = useState<string | null>(null)
  // 标注状态
  const [shapes, setShapes] = useState<ShapeDraft[]>([])
  /** 当前贴纸素材：emoji 字形或 SVG 图标名（默认 ✅） */
  const [sticker, setSticker] = useState<StickerPick>({ kind: 'emoji', value: STICKER_DEFAULT })
  /** 贴纸颜色（对 SVG 图标生效；emoji 是彩色字形，选了也没影响） */
  const [stickerColor, setStickerColor] = useState<string>(STICKER_COLORS[0])
  /** 贴纸显示尺寸（逻辑像素）—— 独立于文字字号，贴纸要大得多 */
  const [stickerSize, setStickerSize] = useState<number>(48)
  /**
   * 重做栈：撤销时把被摘掉的形状压进来，重做时弹回 shapes。
   *
   * 纯"删最后一个"的撤销（原先的实现）没有重做；有了这个栈之后，
   * 只有**新增标注**会清空它（标准编辑器语义：画了新东西，旧的撤销分支作废）。
   */
  const redoRef = useRef<ShapeDraft[]>([])
  const [redoCount, setRedoCount] = useState(0)
  /** null = 未选工具（"选择"态）：此时画布不拦截鼠标，可在选区内重新框选 */
  const [tool, setTool] = useState<AnnotateTool | null>(null)
  const [color, setColor] = useState<Rgba>(PRESET_COLORS[0])
  const [width, setWidth] = useState(2)
  const [mosaicBlock, setMosaicBlock] = useState(MOSAIC_BLOCKS[1])
  /** 文字字号（逻辑像素）；与画笔线宽分开存，切工具不会互相污染 */
  const [fontSize, setFontSize] = useState(TEXT_FONT_SIZE.default)
  /** 聚光灯压暗强度（0-255）；同样独立存，不借用 width */
  const [spotDim, setSpotDim] = useState(SPOTLIGHT_DIM.default)
  /** 橡皮擦半径（逻辑像素）；切工具不重置，保留用户上次的选择 */
  const [eraserRadius, setEraserRadius] = useState(ERASER_RADIUS.default)
  /** 放大镜倍数 */
  const [zoomLevel, setZoomLevel] = useState(ZOOM_LEVELS[1])
  /** 取色器：当前采样到的颜色（null = 未在取色）。
   *  `x`/`y` 是**屏幕坐标**（CSS px），气泡跟随指针渲染。 */
  const [pick, setPick] = useState<{
    x: number
    y: number
    rgb: [number, number, number]
    hex: string
  } | null>(null)
  /** 取色器：刚刚复制成功的 HEX（null = 无提示）。
   *  存字符串而不是布尔：松手时气泡已经收起，需要记住"复制的是哪个色"。 */
  const [copiedHex, setCopiedHex] = useState<string | null>(null)
  const pickCopiedTimer = useRef<number | undefined>(undefined)
  /** 选区阶段（拖选/悬停，未出 rect）光标处颜色：放大镜气泡底部展示，
   *  并按 C 直接复制。与取色器（选区内 pick 工具）各管一个阶段。 */
  const [dragColor, setDragColor] = useState<{
    rgb: [number, number, number]
    hex: string
  } | null>(null)
  /** 选区阶段首次提示（每个应用会话只出一次，不每次截图都刷屏） */
  const [selHint, setSelHint] = useState<string | null>(null)
  const selHintTimer = useRef<number | undefined>(undefined)
  const selHintShownRef = useRef(false)
  /** 二维码识别结果（null = 未识别 / 未打开面板） */
  const [qrCodes, setQrCodes] = useState<DecodedCode[] | null>(null)
  const [qrBusy, setQrBusy] = useState(false)
  const [qrError, setQrError] = useState<string | null>(null)
  /** OCR 识别结果（null = 未识别 / 未打开面板） */
  const [ocrBusy, setOcrBusy] = useState(false)
  /** 翻译进行中：防止连点重复跑识别+请求 */
  const [transBusy, setTransBusy] = useState(false)
  const [ocrError, setOcrError] = useState<string | null>(null)
  /** OCR 模型是否就绪（null = 还没查到）。false 时按钮提示去下载模型。 */
  const [ocrModelsOk, setOcrModelsOk] = useState<boolean | null>(null)
  /** 正在下载 OCR 模型（15.6MB）：期间按钮禁用，避免重复触发 */
  const [modelsBusy, setModelsBusy] = useState(false)
  /**
   * 长截图（M5）：off=未开始 / running=滚动拼接中 / result=结果面板 /
   * blocked=缺「辅助功能」权限，等用户勾选（勾上自动开跑，见下方轮询）
   */
  const [longMode, setLongMode] = useState<'off' | 'running' | 'result' | 'blocked'>('off')
  const [longProgress, setLongProgress] = useState<LongProgress | null>(null)
  /** 结果面板动作执行中（null = 空闲；保存对话框期间保持禁用） */
  const [longCommit, setLongCommit] = useState<ShotAction | null>(null)
  const [longActError, setLongActError] = useState<string | null>(null)
  /** 每次收到 need-accessibility 递增：让轮询 effect 在"手动重试仍失败"时重启 */
  const [longWaitNonce, setLongWaitNonce] = useState(0)
  /** 1×1 取色 Scratch 画布（模块级复用会与其它预览画布抢状态，故随组件） */
  const pickCanvasRef = useRef<HTMLCanvasElement | null>(null)
  /** 冻结图的 ImageBitmap（Rust 读字节 → Blob → 解码）。
   *
   *  为什么不直接拿 <img> 读像素：`convertFileSrc` 的 asset 图片在 WebView2 里
   *  属于跨域资源，`drawImage` 能画（所以放大镜看得见画面），但
   *  `getImageData` 会被判为污染画布而抛 SecurityError——取色因此拿不到值
   *  （用户实测：气泡底部只剩"按 C 复制颜色"兜底文案）。
   *  Blob 解码出的 ImageBitmap 是本地数据，读取永远合法；顺带省掉每帧解码。
   *  位图未就绪时回退 <img>（只绘制不读取，两者等价）。 */
  const [bitmap, setBitmap] = useState<ImageBitmap | null>(null)
  /** 取样异常只记一次日志（每次移动都抛的话会把日志刷爆） */
  const sampleErrLoggedRef = useRef(false)

  /**
   * 调暗度：**同时改最近一个聚光灯**。
   *
   * 滑条如果只影响"下一次拖出来的"框，用户画完再拖滑条会看不到任何变化
   * （实际踩到的反馈：调到 16% 和 67% 截两张图，两张一模一样）。
   * 所以对"最近这一个"做即时回改；再画新的仍然用当前滑条值。
   */
  const applySpotDim = (v: number) => {
    setSpotDim(v)
    setShapes((prev) => {
      for (let i = prev.length - 1; i >= 0; i -= 1) {
        const s = prev[i]
        if (s.type === 'spotlight') {
          const next = prev.slice()
          next[i] = { ...s, dim: v }
          return next
        }
      }
      return prev
    })
  }
  /** 未实现工具的提示气泡（1.8s 自动消失） */
  const [hint, setHint] = useState<string | null>(null)
  const hintTimer = useRef<number | undefined>(undefined)
  /** 二级菜单锚点：跟随当前选中的一级按钮（x 居中、bottom = 按钮底边） */
  const barRef = useRef<HTMLDivElement | null>(null)
  const propsRef = useRef<HTMLDivElement | null>(null)
  const hintRef = useRef<HTMLDivElement | null>(null)
  const [anchor, setAnchor] = useState<{ x: number; bottom: number } | null>(null)
  const [hintAnchor, setHintAnchor] = useState<{ x: number; bottom: number } | null>(null)
  /**
   * 一级工具条的矩形（左/宽/底）。
   *
   * 二级属性条的对齐规则（docs/BUILD-macos.md 验收项）：
   * **在按钮下方、且不超出工具条**。所以二级条直接跟一级条同宽、左边缘对齐 ——
   * 之前那套"跟随选区中心 + 边界收拢"在属性条还是窄面板时看不出来，
   * 改成单行条之后就明显左偏了（二级条比一级条窄，中心还被选区拽偏）。
   */
  const [barBox, setBarBox] = useState<{ left: number; width: number; bottom: number } | null>(
    null,
  )
  const [hintW, setHintW] = useState(0)
  const imgRef = useRef<HTMLImageElement | null>(null)
  const dragRef = useRef<{ x: number; y: number; snap: WindowSnap | null } | null>(null)
  /** 悬停识窗的节流与乱序保护：前端每次 mousemove 都发 IPC 太贵，
   *  限到 ~15fps；慢响应回来时若已有更新的请求则丢弃。 */
  const snapAtRef = useRef(0)
  const snapSeqRef = useRef(0)
  /** 吸附查询失败只记一次日志（避免 15fps 刷屏） */
  const snapErrLogged = useRef(false)
  /** 锚点缩放进行中：记下拖的是哪条边、起始矩形、按下时的指针位置。 */
  const resizeRef = useRef<{
    edge: HandleEdge
    start: Rect
    px: number
    py: number
  } | null>(null)
  /** 本浮层是否已进入就绪态（用于识别"会话被结束"的 true→false 跳变） */
  const readyRef = useRef(false)
  /** 本轮等待的起始时间。浮层窗口会被跨会话复用（页面不重载），
   *  若用挂载时刻做基准，第二轮起会立刻判定"已超时"。故每次收到
   *  overlay-ready 或窗口转为可见时重置。 */
  const startedRef = useRef(0)
  /** 连续观察到 ready=false 的次数（会话结束自关的防抖） */
  const missRef = useRef(0)

  useEffect(() => {
    uiLog('overlay mounted')
    return () => uiLog('overlay unmounted')
  }, [])

  // 冻结图 → ImageBitmap：取色（getImageData）与放大镜绘制的统一数据源。
  // 字节由 Rust 直读（read_image_bytes）→ Blob → 解码，完全绕开
  // asset 协议的跨域判定；直接对 asset <img> 读像素会抛 SecurityError。
  useEffect(() => {
    if (!ctx) return
    let disposed = false
    let bmp: ImageBitmap | null = null
    setBitmap(null)
    sampleErrLoggedRef.current = false
    readImageBytes(ctx.imagePath)
      .then((buf) => new Blob([buf], { type: 'image/png' }))
      .then((b) => createImageBitmap(b))
      .then((b) => {
        if (disposed) {
          b.close()
          return
        }
        bmp = b
        setBitmap(b)
        uiLog(`bitmap ready ${b.width}x${b.height}`)
      })
      .catch((err) => {
        // 不阻断截图流程：位图缺失时放大镜仍用 <img> 绘制，
        // 只是取色退化为空值（曾因这条路径静默失败排查很久，故留日志）
        uiLog(`bitmap load failed: ${String(err)}`)
      })
    return () => {
      disposed = true
      bmp?.close()
    }
  }, [ctx])

  // 浮层窗口在采集前已预创建（隐藏态），挂载时可能不可见：
  // 隐藏页面的定时器会被浏览器节流，故轮询不能只依赖 visibilitychange
  // （WebView2 的窗口 show() 不保证触发 visibilitychange——曾因此
  // 永远等不到 ready 而误报"截图采集超时"）。
  // 现行三路保障：
  // ① Rust 采集完成后 emit 'overlay-ready' 事件 → 立即拉取上下文（主路径）；
  // ② visibilitychange → 变可见时补拉（快速路径）；
  // ③ 低频轮询兜底（无门控，400ms；隐藏态最差被节流到 1s/次，
  //    15s 超时窗口内足够）。
  useEffect(() => {
    let cancelled = false
    let pollTimer: number | undefined
    let fallback: number | undefined
    let unlisten: (() => void) | undefined
    const TIMEOUT_MS = 15000
    startedRef.current = Date.now()
    const poll = () => {
      if (cancelled) return
      // 超时判定只对"还没拿到数据"的会话有效。
      // 曾经不加 readyRef 门控：兜底 interval 一直跑，就绪 15 秒后
      // 把已经正常显示的浮层覆盖成错误页 → 用户报"截图采集失败/超时"。
      if (!readyRef.current && Date.now() - startedRef.current > TIMEOUT_MS) {
        uiLog('poll TIMEOUT -> 截图采集超时')
        setError('截图采集超时，请重试')
        return
      }
      overlayContext()
        .then((c) => {
          if (cancelled) return
          if (c.ready) {
            missRef.current = 0
            if (!readyRef.current) {
              readyRef.current = true
              uiLog(
                `poll ready session=${c.sessionId} img=${c.imagePath} ` +
                  `${c.naturalWidth}x${c.naturalHeight} sf=${c.scaleFactor} dpr=${window.devicePixelRatio} ` +
                  `win=${window.innerWidth}x${window.innerHeight}`,
              )
              setCtx(c)
            }
          } else if (readyRef.current) {
            // 会话已结束（取消/确认成功，Rust 清空了会话数据）：
            // 正常情况下 Rust 已关闭本窗，这里是双屏漏关的轮询兜底。
            // 连续多次确认才自关：避免把复用窗口的陈旧状态误判成会话结束。
            missRef.current += 1
            if (missRef.current === 1) uiLog('poll ready->false (session 已结束)')
            if (missRef.current >= 15) {
              uiLog('poll miss>=15 -> closeSelf')
              closeSelf().catch(() => {})
            }
            pollTimer = window.setTimeout(poll, 300)
          } else {
            pollTimer = window.setTimeout(poll, 80)
          }
        })
        .catch((e) => {
          if (cancelled) return
          uiLog(`poll error: ${String(e)}`)
          // 极端情况（窗口已被关闭等）：短退避后重试
          pollTimer = window.setTimeout(poll, 150)
          if (!readyRef.current && Date.now() - startedRef.current > TIMEOUT_MS) {
            setError(String(e))
          }
        })
    }
    const kick = () => {
      if (cancelled) return
      if (pollTimer !== undefined) window.clearTimeout(pollTimer)
      poll()
    }
    // ① 主路径：Rust 采集完成事件（同时标志新一轮会话开始）
    // 页面可能被跨会话复用（窗口未销毁 → 不重载、状态陈旧），
    // 故在此把全部交互状态重置回"新会话"。
    listen('overlay-ready', () => {
      uiLog('event overlay-ready')
      startedRef.current = Date.now()
      readyRef.current = false
      missRef.current = 0
      setError(null)
      setActError(null)
      setRect(null)
      setDraft(null)
      setHoverWin(null)
      setShapes([])
      redoRef.current = []
      setRedoCount(0)
      setTool(null)
      setHint(null)
      setCursor(null)
      setBusy(false)
      setPick(null)
      setCopiedHex(null)
      setDragColor(null)
      setQrCodes(null)
      setQrError(null)
      setQrBusy(false)
        setOcrError(null)
      setOcrBusy(false)
      setLongMode('off')
      setLongProgress(null)
      setLongCommit(null)
      setLongActError(null)
      // 每轮会话查一次模型状态（跨会话可能已下载好），失败不阻塞截图
      ocrModelsStatus()
        .then((s) => setOcrModelsOk(s.available))
        .catch(() => setOcrModelsOk(null))
      // 首次进入截图态提示选区阶段的新交互（放大镜 / C 取色）。
      // 只提示一次：重复弹同一句话会变成骚扰。
      if (!selHintShownRef.current) {
        selHintShownRef.current = true
        setSelHint('移动显示放大镜 · 按 C 复制颜色')
        if (selHintTimer.current !== undefined) window.clearTimeout(selHintTimer.current)
        selHintTimer.current = window.setTimeout(() => setSelHint(null), 3200)
      }
      kick()
    })
      .then((fn) => {
        if (cancelled) fn()
        else unlisten = fn
      })
      .catch(() => {})
    // ② 快速路径：窗口变可见时补拉（保留门控：不可见时无意义）
    const onVisible = () => {
      if (document.visibilityState === 'visible') {
        startedRef.current = Date.now()
        kick()
      }
    }
    document.addEventListener('visibilitychange', onVisible)
    // ③ 兜底：无门控低频轮询（隐藏态定时器被节流也不怕，事件会补位）
    fallback = window.setInterval(kick, 400)
    kick()
    return () => {
      cancelled = true
      if (pollTimer !== undefined) window.clearTimeout(pollTimer)
      if (fallback !== undefined) window.clearInterval(fallback)
      document.removeEventListener('visibilitychange', onVisible)
      unlisten?.()
    }
  }, [])

  // 键盘：Esc 取消截图；Enter 完成（复制）；Ctrl+Enter 同；Ctrl+Z 撤销标注
  // 长截图进行中（M5）：Esc 改为"停止滚动并收尾出图"，不是退出截图——
  // 用户拼到一半按 Esc 的预期是"就到这里"，而不是"全部白干"。
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      // 文字输入中：键盘事件归输入框自己处理（否则 Enter/Esc/Ctrl+Z 会被
      // 当成"完成截图/取消截图/撤销标注"，打字打到一半整张图就没了）
      const target = e.target as HTMLElement | null
      const tag = target?.tagName
      if (tag === 'TEXTAREA' || tag === 'INPUT') return
      if (e.key === 'Escape') {
        e.preventDefault()
        if (longMode === 'running') {
          uiLog('long: stop requested (Esc)')
          void longCaptureStop().catch(() => {})
        } else {
          void exitCapture()
        }
      } else if (e.key === 'Enter' && rect && !busy && longMode === 'off') {
        e.preventDefault()
        void finish('copy')
      } else if (e.ctrlKey && e.key.toLowerCase() === 'z') {
        // Ctrl+Shift+Z 是重做（macOS/PS 习惯）；Ctrl+Y 也给，Windows 习惯
        e.preventDefault()
        if (e.shiftKey) redoShape()
        else undoShape()
      } else if (e.ctrlKey && e.key.toLowerCase() === 'y') {
        e.preventDefault()
        redoShape()
      } else if (
        (e.key === 'c' || e.key === 'C') &&
        !e.ctrlKey &&
        !e.metaKey &&
        !e.altKey &&
        cursor &&
        !rect &&
        longMode === 'off'
      ) {
        // 选区阶段取色：复制光标处颜色（放大镜气泡同位置出"已复制"提示）。
        // 不要求先出选区——这正是"这一步直接取色"的入口。
        e.preventDefault()
        const s = dragColor ?? sampleMonitorPixel(cursor.x, cursor.y)
        if (s) {
          void copyText(s.hex)
            .then(() => {
              setCopiedHex(s.hex)
              if (pickCopiedTimer.current !== undefined) window.clearTimeout(pickCopiedTimer.current)
              pickCopiedTimer.current = window.setTimeout(() => setCopiedHex(null), 1200)
            })
            .catch((e2) => uiLog(`copy hex failed: ${String(e2)}`))
        }
      }
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  })

  // 退出截图模式（三级保障，任一成功即完全退出）：
  // ① Rust 关闭全部浮层并清空会话；② 本窗口自关；
  // ③ 其余浮层轮询发现会话结束（ready true→false）也会自关。
  const exitCapture = async () => {
    uiLog(`exitCapture session=${ctx?.sessionId}`)
    try {
      // 带上本会话代号：Rust 会忽略上一轮迟到的取消，避免误杀新会话
      await cancelCapture(ctx?.sessionId)
    } catch {
      /* 忽略：交由自关兜底 */
    }
    closeSelf().catch(() => {})
  }

  // 完成输出：把选区 + 标注一起交给 Rust（裁剪 + 光栅化 + 执行动作）
  const finish = async (action: ShotAction) => {
    if (!ctx || !rect || busy) return
    uiLog(`finish action=${action} session=${ctx.sessionId}`)
    setBusy(true)
    setActError(null)
    try {
      // 发送前剥掉位图图形里的 canvas 引用（DOM 节点不能进 IPC）
      await confirmSelection(ctx.monitorId, rect, action, toShapePayloads(shapes), ctx.sessionId)
      // Rust 已关全部浮层；自关兜底（双屏下某块浮层漏关时由③轮询兜底）
      closeSelf().catch(() => {})
    } catch (e) {
      console.error('confirm selection failed', e)
      setActError(`${ACTION_LABEL[action]}失败：${String(e)}`)
      setBusy(false)
    }
  }

  // 未实现工具：给出短提示，不做静默失败（用户点了要有反馈）
  // `suffix` 传空串 = 原文展示（如"请先框选…"这类引导语，不是"开发中"）
  const showHint = (
    text: string,
    el?: HTMLElement | null,
    suffix = ' 正在开发中',
    /** 展示时长（ms）。错误文案要长一些，默认 1.8s 不够读 */
    durationMs = 1800,
  ) => {
    if (el) {
      const r = el.getBoundingClientRect()
      setHintAnchor({ x: r.left + r.width / 2, bottom: r.bottom })
    }
    setHint(suffix ? `${text}${suffix}` : text)
    if (hintTimer.current !== undefined) window.clearTimeout(hintTimer.current)
    hintTimer.current = window.setTimeout(() => setHint(null), durationMs)
  }

  /** 选工具：再次点击已选中的工具则回到"选择"态（工具条无高亮）。
   *  荧光笔没有独立按钮（挂在画笔的二级面板里），点画笔等于收起它。
   *  动作类工具（取色/二维码）不覆盖线宽——它们没有线宽语义，
   *  覆盖了会让用户回到标注工具时线宽变 0（看不见的笔触）。 */
  const pickTool = (t: AnnotateTool) => {
    const same = tool === t || (t === 'pen' && tool === 'highlighter')
    setTool(same ? null : t)
    if (!same && !isActionTool(t)) setWidth(toolMeta(t).defaultWidth)
  }

  /** 从冻结图采样 (mx, my)（**显示器逻辑像素**）处的像素。
   *
   *  坐标链路：显示器逻辑 → 物理像素（×sf）。
   *  与 AnnotationLayer 里马赛克/放大镜的取样口径保持一致。
   *  返回 null 表示越界（图外或图还没解码完）。 */
  const sampleMonitorPixel = (mx: number, my: number) => {
    const img = imgRef.current
    if (!img) return null
    const px = Math.round(mx * sf)
    const py = Math.round(my * sf)
    const w = bitmap?.width ?? img.naturalWidth
    const h = bitmap?.height ?? img.naturalHeight
    if (px < 0 || py < 0 || px >= w || py >= h) return null
    const c = pickCanvasRef.current ?? (pickCanvasRef.current = document.createElement('canvas'))
    c.width = 1
    c.height = 1
    const g = c.getContext('2d', { willReadFrequently: true })
    if (!g) return null
    g.clearRect(0, 0, 1, 1)
    g.imageSmoothingEnabled = false
    // 只用 ImageBitmap，**位图未就绪就直接返回 null，不要回退 <img>**：
    // 这块 canvas 是复用的，asset 协议的 <img> 画进来会把画布**永久**标记为
    // 跨域污染 —— 之后即使换 ImageBitmap 重画，getImageData 也永远抛
    // SecurityError（macOS 真机实测：一次回退 → 整场截图的取色器全废，
    // 日志表现就是反复 "sample pixel failed"）。
    if (!bitmap) return null
    g.drawImage(bitmap, px, py, 1, 1, 0, 0, 1, 1)
    let d: Uint8ClampedArray
    try {
      d = g.getImageData(0, 0, 1, 1).data
    } catch (err) {
      // 跨域污染画布（asset 图片尚未换成位图时会发生）：只记一次，避免刷屏
      if (!sampleErrLoggedRef.current) {
        sampleErrLoggedRef.current = true
        uiLog(`sample pixel failed: ${String(err)}`)
      }
      return null
    }
    const rgb: [number, number, number] = [d[0], d[1], d[2]]
    return { rgb, hex: toHex(rgb) }
  }

  /** 取色器：从冻结图采样 (lx, ly)（**选区内**逻辑像素）处的像素。
   *  选区内逻辑 → 显示器逻辑（+rect 原点）后转交 sampleMonitorPixel。 */
  const samplePixel = (lx: number, ly: number) => {
    if (!rect) return null
    return sampleMonitorPixel(rect.x + lx, rect.y + ly)
  }

  /** 取色器：移动中持续刷新气泡 */
  const onPickSample = (lx: number, ly: number) => {
    const s = samplePixel(lx, ly)
    if (!s) {
      setPick(null)
      return
    }
    setPick({ x: rect ? rect.x + lx : lx, y: rect ? rect.y + ly : ly, ...s })
  }

  /** 取色器：松手定色并复制 HEX */
  const onPickEnd = (lx: number, ly: number) => {
    const s = samplePixel(lx, ly)
    setPick(null)
    if (!s) return
    void copyText(s.hex)
      .then(() => {
        setCopiedHex(s.hex)
        if (pickCopiedTimer.current !== undefined) window.clearTimeout(pickCopiedTimer.current)
        pickCopiedTimer.current = window.setTimeout(() => setCopiedHex(null), 1200)
      })
      .catch((e) => uiLog(`copy hex failed: ${String(e)}`))
  }

  /** 二维码：拖出子区域后只识别这块（不是整个选区）。
   *
   *  坐标换算：AnnotationLayer 给的是**选区内**逻辑像素，而 decode_qr 的
   *  rect 与 confirm_selection 同口径（显示器逻辑像素），所以要加上选区原点。
   *  若直接传整个选区，用户在选区里圈一小块就会识别到圈外的码。 */
  const onQrRegion = async (x: number, y: number, w: number, h: number) => {
    if (!ctx || !rect) return
    setQrBusy(true)
    setQrError(null)
    setQrCodes(null)
    try {
      const codes = await decodeQr(
        ctx.monitorId,
        { x: rect.x + x, y: rect.y + y, w, h },
        ctx.sessionId,
      )
      setQrCodes(codes)
      if (codes.length === 0) uiLog('qr: no code found in region')
    } catch (e) {
      uiLog(`qr decode failed: ${String(e)}`)
      setQrError(String(e))
    } finally {
      setQrBusy(false)
    }
  }

  /** OCR：对当前整个选区跑文字识别（点按钮即用，不需要拖子区域）。
   *
   *  与二维码的区别：二维码要用户指明"识别哪一块"（一图多码时拖选），
   *  而 OCR 的输入就是选区本身——先框选、再点按钮，符合主流截图工具习惯。
   *  没有选区时给提示，不静默失败。
   *
   *  模型缺失时 Rust 返回带目录的错误，原样展示给用户（含下载说明）。 */
  const runOcr = async (anchor?: HTMLElement | null) => {
    if (!ctx || !rect) {
      if (anchor) showHint('请先框选要识别的区域', anchor)
      return
    }
    if (ocrBusy || modelsBusy) return
    // 模型缺失：直接把"下载"做了，而不是让用户自己去找三个 onnx 放到某个路径。
    // （原来只报一段带目录的错，对普通用户等于不可用。）
    if (ocrModelsOk === false) {
      setModelsBusy(true)
      setOcrError(null)
      if (anchor) showHint('首次使用：正在下载 OCR 模型（约 15.6MB）…', anchor, '', 0)
      try {
        const st = await ocrDownloadModels()
        setOcrModelsOk(st.available)
        if (anchor) {
          showHint(
            st.available ? '模型已就绪，请再点一次识别' : `模型仍缺失：${st.dir}`,
            anchor,
            '',
            6000,
          )
        }
        if (!st.available) setOcrError(`OCR 模型仍缺失：${st.dir}`)
      } catch (e) {
        uiLog(`ocr model download failed: ${String(e)}`)
        setOcrError(String(e))
        showHint(String(e), anchor ?? null, '', 8000)
      } finally {
        setModelsBusy(false)
      }
      return
    }
    setOcrBusy(true)
    setOcrError(null)
    // 反馈：识别是秒级操作，浮层必须给个"正在干活"的提示，
    // 否则用户看起来就是"点了什么都没发生"（曾经的线上反馈）。
    if (anchor) showHint('文字识别中…', anchor, '', 900)
    try {
      const lines = await ocrRecognize(ctx.monitorId, rect, ctx.sessionId)
      // 识别结果进**独立窗口**（可拖动选择 / 全部复制 / 顶部翻译）。
      // 空结果也开窗——让用户看到"确实没字"，而不是点了没反应。
      // 窗口打开后 Rust 侧会结束截图会话（关浮层），不会留在这里卡住。
      await ocrShow(lines)
      if (lines.length === 0) uiLog('ocr: no text found in selection')
    } catch (e) {
      uiLog(`ocr recognize failed: ${String(e)}`)
      // 失败就地提示（模型缺失 / 推理错误都带可操作文案），不开空窗口。
      // 提示必须可见——内嵌结果面板删掉后，这是用户唯一能看到的错误出口。
      setOcrError(String(e))
      showHint(String(e), null, '', 4000)
    } finally {
      setOcrBusy(false)
    }
  }

  /**
   * 框选 → 识别 → 翻译（工具条「翻译」按钮）。
   *
   * 复用 OCR 的识别结果当原文：翻译窗本身就是"左原文 / 右译文"，
   * 所以**不再开 OCR 窗**（两个窗一起弹太重），直接把识别文本交给 trans_start。
   * 失败分三种，都要有可见出口：
   *   1. 没框选        → 就地提示
   *   2. 没识别到文字  → 就地提示（不开空窗）
   *   3. 翻译未配置/网络错 → Rust 把错误塞进 trans 状态，翻译窗里会显示
   */
  const runTranslate = async (anchor?: HTMLElement | null) => {
    if (!ctx || !rect) {
      if (anchor) showHint('请先框选要翻译的区域', anchor)
      return
    }
    if (transBusy) return
    setTransBusy(true)
    if (anchor) showHint('识别并翻译中…', anchor, '', 1200)
    try {
      const lines = await ocrRecognize(ctx.monitorId, rect, ctx.sessionId)
      const text = lines
        .map((l) => l.text)
        .join('\n')
        .trim()
      if (!text) {
        showHint('这块区域没识别到文字', anchor ?? null, '', 3000)
        return
      }
      // 翻译窗打开后本轮截图就不需要了（与 OCR 一致：不留一个卡住的浮层）
      await transStart(text)
      void exitCapture()
    } catch (e) {
      uiLog(`translate failed: ${String(e)}`)
      showHint(String(e), anchor ?? null, '', 4000)
    } finally {
      setTransBusy(false)
    }
  }

  /** 长截图（M5）：把当前选区交给 Rust 自动滚动拼接。
   *
   *  启动失败不走 reject：Rust 会补发一条 phase="error" 的进度事件，
   *  UI 统一由事件驱动切到结果面板（避免"命令 reject 与 error 事件"
   *  抢状态导致面板闪跳）。选区太矮时前端先挡一道（Rust 也有同样校验）。
   *
   *  滚动期间浮层整体光标穿透 + 采集排除（Rust 侧设置），用户看到的是
   *  真实桌面在选区里滚动；本窗只负责状态条与结果面板。 */
  const startLong = async () => {
    if (!ctx || !rect || longMode === 'running') return
    if (rect.h * sf < 80) {
      showHint('选区太矮，长截图至少需要 80px 高', null, '')
      return
    }
    setLongMode('running')
    setLongProgress(null)
    setLongActError(null)
    try {
      // 每步 3 行滚轮（一格 ≈ 3 行）：长页面步进太小人肉等不及，太大易丢重叠
      await longCaptureStart(ctx.monitorId, rect, 3, ctx.sessionId)
    } catch (e) {
      uiLog(`long start failed: ${String(e)}`)
    }
  }

  /** 结果面板动作：保存 / 复制 / 贴图。成功后 Rust 已结束会话，自关兜底。 */
  const commitLong = async (action: ShotAction) => {
    if (longCommit) return
    setLongCommit(action)
    setLongActError(null)
    try {
      await longCaptureCommit(action)
      closeSelf().catch(() => {})
    } catch (e) {
      uiLog(`long commit failed (${action}): ${String(e)}`)
      setLongActError(`${ACTION_LABEL[action]}失败：${String(e)}`)
      setLongCommit(null)
    }
  }

  // 长截图进度事件（M5）：驱动循环每步一发；done/error 时切结果面板。
  // 缩略图同一路径每步覆盖写，用 `?t=<steps>` 绕开 WebView 缓存。
  useEffect(() => {
    let unlisten: (() => void) | undefined
    listen<LongProgress>('long-capture-progress', (ev) => {
      const p = ev.payload
      setLongProgress(p)
      if (p.phase === 'done') {
        uiLog(`long done: ${p.message} (${p.height}px, ${p.steps} steps)`)
        // 拼接完成 → 结果交给独立结果窗，并**退出截图模式**。
        // 顺序不能反：long_result_open 会先置位"保留会话"，之后再关浮层，
        // 否则 exitCapture 会把长截图会话一起丢掉（结果窗按钮全部失效）。
        void (async () => {
          try {
            await longResultOpen()
          } catch (e) {
            uiLog(`long result window failed: ${String(e)}`)
          }
          void exitCapture()
        })()
        return
      }
      if (p.phase === 'need-accessibility') {
        // macOS 缺「辅助功能」：Rust 已把系统设置拉到前台。切到"等授权"卡片，
        // 由下面的轮询在用户勾选后自动重跑（该权限即时生效，不用重启应用）。
        uiLog(`long blocked: ${p.message}`)
        setLongMode('blocked')
        setLongWaitNonce((n) => n + 1)
        return
      }
      if (p.phase === 'error') {
        // 出错时留在浮层面板里显示原因（用户要能看到失败信息）
        setLongMode('result')
        uiLog(`long error: ${p.message} (${p.height}px, ${p.steps} steps)`)
      }
    })
      .then((fn) => {
        unlisten = fn
      })
      .catch(() => {})
    return () => {
      unlisten?.()
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [])

  /**
   * 「等辅助功能授权」轮询（macOS）。
   *
   * 为什么轮询而不是让用户重新点一次长截图：0.1.10 真机日志里，用户去系统设置
   * 勾完还得回到浮层再点一次，中间很容易误以为"点了没反应"。这个权限是即时
   * 生效的（不像屏幕录制要重启），所以 800ms 轮一次，勾上就自己开跑。
   * 超时（90s）后保留卡片与手动按钮，不自动放弃。
   */
  useEffect(() => {
    if (longMode !== 'blocked') return
    let stopped = false
    let tries = 0
    let timer: number | undefined
    const tick = async () => {
      if (stopped) return
      tries += 1
      try {
        if (await accessibilityGranted()) {
          if (stopped) return
          uiLog('long: 辅助功能已授权，自动继续长截图')
          // 不先置 off：startLong 的重入判断只挡 'running'
          void startLong()
          return
        }
      } catch (e) {
        uiLog(`accessibility poll failed: ${String(e)}`)
      }
      if (tries >= 112) return // ~90s
      timer = window.setTimeout(tick, 800)
    }
    timer = window.setTimeout(tick, 400)
    return () => {
      stopped = true
      if (timer !== undefined) window.clearTimeout(timer)
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [longMode, longWaitNonce])

  /** 二级菜单贴着当前工具按钮：取按钮矩形的水平中心与底边 */
  /**
   * 提示气泡的水平收拢：以工具条为界，别溢出屏幕。
   * （二级属性条不用它——那条走 barBox 同宽左对齐，见下。）
   */
  const anchorX = (x: number, w: number) => {
    const half = w / 2
    return Math.min(Math.max(x, half + 8), window.innerWidth - half - 8)
  }

  useLayoutEffect(() => {
    if (!tool) {
      setAnchor(null)
      return
    }
    const key = tool === 'highlighter' ? 'pen' : tool
    const el = barRef.current?.querySelector<HTMLElement>(`[data-tool="${key}"]`)
    if (!el) return
    const r = el.getBoundingClientRect()
    setAnchor({ x: r.left + r.width / 2, bottom: r.bottom })
    // 同帧量一级条的矩形：二级条靠它对齐（不用量二级条自己的宽度了）
    const bar = barRef.current?.getBoundingClientRect()
    if (
      bar &&
      (!barBox ||
        Math.abs(bar.left - barBox.left) > 0.5 ||
        Math.abs(bar.width - barBox.width) > 0.5 ||
        Math.abs(bar.bottom - barBox.bottom) > 0.5)
    ) {
      setBarBox({ left: bar.left, width: bar.width, bottom: bar.bottom })
    }
  }, [tool, barBox])

  // 提示气泡宽度同样实测，避免收拢时用到偏大的估计值
  useLayoutEffect(() => {
    const w = hintRef.current?.offsetWidth ?? 0
    if (w && w !== hintW) setHintW(w)
  }, [hint, hintW])

  /** 二级面板居中于按钮，但左右两侧不得超出一级工具条的左右边界 */
  /** 本浮层的窗口 label：用于识别"这条广播是不是我发的"。 */
  const myLabel = getCurrentWindow().label

  /** 当前选区的镜像（供 querySnap 的异步回调判断"命中的是不是已选中的那个"）。 */
  const rectRef = useRef<Rect | null>(null)
  rectRef.current = rect

  /** 悬停识窗查询（节流 ~15fps + 乱序丢弃）。只查顶层窗口，
   *  查不到（光标下只有我们自己的窗口）就清空高亮。 */
  const querySnap = useCallback(
    (x: number, y: number) => {
      if (!ctx) return
      const now = performance.now()
      if (now - snapAtRef.current < 65) return
      snapAtRef.current = now
      const seq = ++snapSeqRef.current
      // 广播"光标在我这屏"：其它屏必须清掉自己的高亮，否则光标移过去之后
      // 原屏的高亮会**冻结**在最后悬停的窗口上（用户实测：两块屏上各有一个
      // 高亮框，一边 WorkBuddy 一边资源管理器）。
      emit(EVT_HOVER_OWNER, { owner: myLabel }).catch(() => {})
      windowSnap(ctx.monitorId, x, y)
        .then((w) => {
          if (seq !== snapSeqRef.current) return
          // 命中的窗口恰好就是当前选区 → 不显示高亮（同一个框画两遍没意义，
          // 而且盖在选区边框上很难看）。
          type R = { x: number; y: number; w: number; h: number }
          const near = (a: number, b: number) => Math.abs(a - b) < 1.5
          const sel = rectRef.current as R | null
          const sameAsSel =
            !!w &&
            !!sel &&
            near(w.x, sel.x) &&
            near(w.y, sel.y) &&
            near(w.w, sel.w) &&
            near(w.h, sel.h)
          setHoverWin(sameAsSel ? null : w)
        })
        .catch((e) => {
          // 首次失败才记日志：15fps 调用，失败刷屏会淹没别的线索
          if (!snapErrLogged.current) {
            snapErrLogged.current = true
            uiLog(`window_snap failed: ${String(e)}`)
          }
          if (seq === snapSeqRef.current) setHoverWin(null)
        })
    },
    [ctx, myLabel],
  )


  /** 广播：让其它显示器的浮层清掉自己的选区。 */
  const clearOtherSelections = useCallback(() => {
    emit(EVT_CLEAR_OTHER_SEL, { owner: myLabel }).catch(() => {})
  }, [myLabel])

  /** 收到别人的广播 → 清掉我这块屏的选区和工具条。 */
  useEffect(() => {
    let off: (() => void) | null = null
    listen<{ owner?: string }>(EVT_CLEAR_OTHER_SEL, (e) => {
      if (e.payload?.owner && e.payload.owner !== myLabel) {
        uiLog(`clear selection (owned by ${e.payload.owner})`)
        setRect(null)
        setDraft(null)
        setHoverWin(null)
      }
    })
      .then((f) => {
        off = f
      })
      .catch(() => {})
    return () => off?.()
  }, [myLabel])

  /** 光标在别的屏 → 清掉我这块屏的悬停高亮（否则会冻结在最后悬停的窗口上）。 */
  useEffect(() => {
    let off: (() => void) | null = null
    listen<{ owner?: string }>(EVT_HOVER_OWNER, (e) => {
      if (e.payload?.owner && e.payload.owner !== myLabel) setHoverWin(null)
    })
      .then((f) => {
        off = f
      })
      .catch(() => {})
    return () => off?.()
  }, [myLabel])

  /** 开始锚点缩放。stopPropagation 是关键——否则根节点的 onPointerDown
   *  会当成"重新框选"，一点锚点选区就没了。 */
  const beginResize = (e: ReactPointerEvent<HTMLElement>, edge: HandleEdge) => {
    if (e.button !== 0 || !rect) return
    e.stopPropagation()
    e.preventDefault()
    try {
      e.currentTarget.setPointerCapture(e.pointerId)
    } catch {
      /* 某些环境不支持捕获，退回普通冒泡也能用 */
    }
    resizeRef.current = { edge, start: rect, px: e.clientX, py: e.clientY }
    setHoverWin(null)
  }

  /** 撤销：摘掉最后一个形状并压入重做栈。 */
  const undoShape = useCallback(() => {
    setShapes((prev) => {
      if (prev.length === 0) return prev
      redoRef.current = [...redoRef.current, prev[prev.length - 1]]
      setRedoCount(redoRef.current.length)
      return prev.slice(0, -1)
    })
  }, [])

  /** 重做：把重做栈顶弹回形状列表。 */
  const redoShape = useCallback(() => {
    setShapes((prev) => {
      const stack = redoRef.current
      if (stack.length === 0) return prev
      const back = stack[stack.length - 1]
      redoRef.current = stack.slice(0, -1)
      setRedoCount(redoRef.current.length)
      return [...prev, back]
    })
  }, [])

  /** 供标注层用的 shapes setter：新增形状时清空重做栈。 */
  const setShapesFromLayer = useCallback<Dispatch<SetStateAction<ShapeDraft[]>>>((action) => {
    setShapes((prev) => {
      const next =
        typeof action === 'function' ? (action as (p: ShapeDraft[]) => ShapeDraft[])(prev) : action
      if (next.length > prev.length) {
        redoRef.current = []
        setRedoCount(0)
      }
      return next
    })
  }, [])

  const onPointerDown = (e: ReactPointerEvent<HTMLDivElement>) => {
    if (e.button !== 0 || !ctx) return
    // 等「辅助功能」授权期间浮层已取消置顶（系统设置盖在上面），用户在上面
    // 点来点去很容易误把已有选区清掉 —— 这一状态下不响应框选。
    if (longMode === 'blocked') return
    // 本屏开始画 → 让其它屏清掉它们的选区（双屏只能有一个选区/一条工具条）
    clearOtherSelections()
    // 按住时先把当前吸附窗口记进 dragRef：松手没移动 = 单击选整窗，
    // 移动了 = 普通拖选（见 onPointerMove / onPointerUp）。
    dragRef.current = { x: e.clientX, y: e.clientY, snap: hoverWin }
    setRect(null)
    setDraft({ x: e.clientX, y: e.clientY, w: 0, h: 0 })
  }

  const onPointerMove = (e: ReactPointerEvent<HTMLDivElement>) => {
    // 锚点缩放优先：拖的是边/角，交给 resizeRect，不进入框选/悬停逻辑
    const rs = resizeRef.current
    if (rs) {
      setRect(resizeRect(rs.start, rs.edge, e.clientX - rs.px, e.clientY - rs.py))
      return
    }
    const d = dragRef.current
    if (d) {
      // 移动超过阈值 → 这是拖选，取消吸附候选
      if (d.snap && (Math.abs(e.clientX - d.x) > 4 || Math.abs(e.clientY - d.y) > 4)) {
        d.snap = null
      }
      setDraft({
        x: Math.min(d.x, e.clientX),
        y: Math.min(d.y, e.clientY),
        w: Math.abs(e.clientX - d.x),
        h: Math.abs(e.clientY - d.y),
      })
    } else if (tool || longMode !== 'off') {
      // 正在标注 / 长截图中：光标交给各自的交互层，
      // 不再跟踪（避免无谓的重渲染，也避免与标注层抢光标语义）。
      return
    }

    if (d) return
    // 光标压在工具条上：不识别窗口，并把已有高亮收掉
    // （否则高亮会"冻"在上一个窗口上，人却在点工具条，很出戏）
    if ((e.target as HTMLElement).closest('.shot-bar')) {
      setHoverWin(null)
      return
    }

    // 已有选区时**仍然识别窗口**：光标离开选区到别的窗口上要能高亮，
    // 单击即可换选目标。此前一句 `if (!rect)` 把查询整个挡掉了，导致
    // "左屏（无选区）能高亮、右屏（已选微信窗）不能"的不对称（用户实测）。
    //
    // 也不能用"光标在选区内就跳过"来过滤：前边的窗口可能正好落在
    // 已选中的大窗口内部（微信窗浮在资源管理器上），那样一进选区就
    // 再也不高亮了（用户实测）。改为**按窗口矩形比对**：
    // 命中的窗口跟当前选区是同一个才不显示（见 querySnap）。
    if (rect) {
      querySnap(e.clientX, e.clientY)
      return
    }

    // 未出选区（悬停或拖选中）：持续跟踪光标，驱动放大镜 + 颜色读数。
    // 两次 setState 在同一事件里，React 自动批合成一次重渲染。
    setCursor({ x: e.clientX, y: e.clientY })
    setDragColor(sampleMonitorPixel(e.clientX, e.clientY))
    querySnap(e.clientX, e.clientY)
  }

  // 光标移出浮层：收起放大镜 + 清掉悬停高亮（拖选中不移出——指针被窗口捕获）
  const onPointerLeave = () => {
    if (!dragRef.current) {
      setCursor(null)
      setHoverWin(null)
    }
  }

  const onPointerUp = () => {
    // 缩放收尾：位置已经实时更新过，这里只清状态
    if (resizeRef.current) {
      resizeRef.current = null
      return
    }
    const d = dragRef.current
    dragRef.current = null
    setCursor(null)
    // 单击（没拖动）且有吸附窗口 → 直接选中整个窗口。
    // 这是 QQ 截图 / 微信截图的核心手感：悬停高亮、一点即整窗。
    if (d?.snap) {
      setRect({ x: d.snap.x, y: d.snap.y, w: d.snap.w, h: d.snap.h })
      setDraft(null)
      setHoverWin(null)
      return
    }
    // 拖出一个新选区：清掉吸附高亮（否刚它会在新选区上残留，
    // 有 rect 之后鼠标一动还可能被误当成"换目标"）
    setHoverWin(null)
    setDraft((dr) => {
      if (dr && dr.w >= MIN_SIZE && dr.h >= MIN_SIZE) setRect(dr)
      return null
    })
  }

  if (error) {
    return <div className="overlay-error">{error}</div>
  }
  if (!ctx) {
    return (
      <div className="overlay-loading">
        <span className="loading-chip">正在截图…</span>
      </div>
    )
  }

  const active = draft ?? rect
  const sf = ctx.scaleFactor
  // 冻屏图必须 1:1 落到物理像素上：
  // 冻结图是物理像素尺寸（如 1920×1200），浮层窗口的 CSS 尺寸是逻辑像素
  // （1920/1.75 = 1097）。若把 img 排成 1920 CSS 宽再 transform: scale(1/sf)，
  // 浏览器会先按大图栅格化再整体重采样 → 文字发虚（用户实测反馈）。
  // 直接排成逻辑尺寸，则 逻辑px × DPR(=sf) 恰好等于图片像素，采样比 1:1。
  const dpr = window.devicePixelRatio || sf || 1
  const style = {
    width: ctx.naturalWidth / dpr,
    height: ctx.naturalHeight / dpr,
  } as CSSProperties
  // 工具条固定在屏幕顶部（选中选区后常驻，不跟随选区）

  return (
    <div
      // ★ 结果态这里**不能**叫 long-result：.long-result 是「结果面板」自己的
      // 样式（宽 340 / 居中 / 面板底色 / 圆角）。根元素一旦也挂这个类，就会把自己
      // 从 inset:0 的全屏根缩成一块 340 宽的灰卡片，其余区域透明但仍在拦鼠标
      // （用户症状：中间一块灰框、整个桌面点不动）。用 -mode 后缀把两者分开。
      className={`overlay-root${longMode === 'running' ? ' long-running' : ''}${
        longMode === 'result' ? ' long-result-mode' : ''
      }${longMode === 'blocked' ? ' long-wait-mode' : ''}`}
      onPointerDown={onPointerDown}
      onPointerMove={onPointerMove}
      onPointerUp={onPointerUp}
      onPointerLeave={onPointerLeave}
      onContextMenu={(e) => {
        e.preventDefault()
        if (rect) setRect(null)
        else void exitCapture()
      }}
    >
      <img
        ref={imgRef}
        className="frozen"
        src={imgSrc(ctx.imagePath)}
        style={style}
        draggable={false}
        alt=""
      />

      {/* 进入截图态但尚未拖拽：整屏轻微压暗。冻结图与桌面几乎一模一样，
          没有这层压暗用户会以为"点了球没反应"（曾被误报为采集失败）。 */}
      {!active && <div className="overlay-dim" />}

      {rect && (
        <div
          className="annotate-wrap"
          style={{
            left: rect.x,
            top: rect.y,
            width: rect.w,
            height: rect.h,
          }}
        >
          <AnnotationLayer
            naturalWidth={rect.w * sf}
            naturalHeight={rect.h * sf}
            scaleFactor={sf}
            imageEl={imgRef.current}
            bitmap={bitmap}
            tool={tool}
            color={color}
            width={width}
            mosaicBlock={mosaicBlock}
            fontSize={fontSize}
            spotlightDim={spotDim}
            eraserRadius={eraserRadius}
            zoomLevel={zoomLevel}
            sticker={sticker}
            stickerColor={stickerColor}
            stickerSize={stickerSize}
            shapes={shapes}
            onShapesChange={setShapesFromLayer}
            onPickSample={onPickSample}
            onPickEnd={onPickEnd}
            onQrRegion={(x, y, w, h) => void onQrRegion(x, y, w, h)}
            qrHighlights={qrCodes ? qrCodes.map((c) => ({ points: c.points })) : null}
          />
        </div>
      )}

      {cursor && draft && (
        <>
          <div className="crosshair-v" style={{ left: cursor.x }} />
          <div className="crosshair-h" style={{ top: cursor.y }} />
        </>
      )}

      {/* 悬停识窗高亮：鼠标下方的窗口描边 + 标题。
          单击即选中整窗（QQ/微信截图同款手感）。pointer-events:none
          保证不挡住下面的鼠标事件。
          **有选区时也渲染**：光标移到别的窗口上要能看到"点这里会换目标"。 */}
      {hoverWin && !draft && !(hoverWin.w < 2 || hoverWin.h < 2) && (
        <div
          className="win-snap"
          style={{
            left: hoverWin.x,
            top: hoverWin.y,
            width: hoverWin.w,
            height: hoverWin.h,
          }}
        >
          <span className="win-snap-label">
            {hoverWin.title || '窗口'}　{hoverWin.w}×{hoverWin.h}
          </span>
        </div>
      )}

      {/* 选区阶段放大镜：悬停 / 拖选时跟随光标，底部带颜色读数。
          仅"未出选区"时显示——出选区后取色由工具条里的取色器负责，
          两个阶段各有一个入口，不重叠也不冲突。 */}
      {cursor && !rect && longMode === 'off' && (
        <DragMagnifier
          cursor={cursor}
          imageEl={imgRef.current}
          bitmap={bitmap}
          scaleFactor={sf}
          hex={dragColor?.hex ?? null}
          copiedHex={copiedHex}
        />
      )}

      {/* 选区阶段首次提示（每个应用会话一次，3.2s 自消） */}
      {selHint && !rect && longMode === 'off' && <div className="sel-hint">{selHint}</div>}

      {/* 压暗遮罩：**四块实心层**，不用 `box-shadow: 0 0 0 9999px`。
          后者每拖动一像素都要重绘一块 9999px 的巨大阴影 —— 在 Retina 全屏
          WebView 里是最典型的卡顿源（macOS 上尤其明显）。四块实心层只改
          left/top/width/height，合成器直接搬运，不重绘阴影。 */}
      {active && (
        <div className={`mask${rect ? ' is-final' : ''}`} aria-hidden="true">
          <i style={{ left: 0, top: 0, width: '100%', height: Math.max(0, active.y) }} />
          <i style={{ left: 0, top: active.y + active.h, right: 0, bottom: 0 }} />
          <i
            style={{
              left: 0,
              top: active.y,
              width: Math.max(0, active.x),
              height: active.h,
            }}
          />
          <i
            style={{
              left: active.x + active.w,
              top: active.y,
              right: 0,
              height: active.h,
            }}
          />
        </div>
      )}

      {active && (
        <div
          className={`sel ${rect ? 'sel-final' : ''}`}
          style={{ left: active.x, top: active.y, width: active.w, height: active.h }}
        >
          <span className="sel-size">
            {Math.round(active.w * sf)} × {Math.round(active.h * sf)}
          </span>
          {rect && (
            <>
              {CORNER_HANDLES.map((edge) => (
                <i
                  key={edge}
                  className={`handle ${edge}`}
                  onPointerDown={(e) => beginResize(e, edge)}
                />
              ))}
            </>
          )}
        </div>
      )}

      {/* 顶部工具条：1:1 复刻参考图（48px 高 · 48px 节距 · 三段分组） */}
      {rect && (
        <>
          <div className="shot-bar" ref={barRef} onPointerDown={(e) => e.stopPropagation()}>
            {/* 第一组：标注工具 */}
            <div className="sb-group">
              <button
                data-tool="rect"
                className={`sbtn ${tool === 'rect' ? 'active' : ''}`}
                title="矩形"
                onClick={() => pickTool('rect')}
              >
                <RectIcon />
              </button>
              <button
                data-tool="ellipse"
                className={`sbtn ${tool === 'ellipse' ? 'active' : ''}`}
                title="椭圆"
                onClick={() => pickTool('ellipse')}
              >
                <EllipseIcon />
              </button>
              <button
                data-tool="sticker"
                className={`sbtn ${tool === 'sticker' ? 'active' : ''}`}
                title="贴纸（先在属性条选一个 emoji，再点画面落点）"
                onClick={() => pickTool('sticker')}
              >
                <StickerIcon />
              </button>
              <button
                data-tool="arrow"
                className={`sbtn ${tool === 'arrow' ? 'active' : ''}`}
                title="箭头"
                onClick={() => pickTool('arrow')}
              >
                <ArrowIcon />
              </button>
              <button
                data-tool="pen"
                className={`sbtn ${tool === 'pen' || tool === 'highlighter' ? 'active' : ''}`}
                title="画笔（可切换荧光笔）"
                onClick={() => pickTool('pen')}
              >
                <PenIcon />
              </button>
              <button
                data-tool="mosaic"
                className={`sbtn ${tool === 'mosaic' ? 'active' : ''}`}
                title="马赛克"
                onClick={() => pickTool('mosaic')}
              >
                <MosaicIcon />
              </button>
              <button
                data-tool="spotlight"
                className={`sbtn ${tool === 'spotlight' ? 'active' : ''}`}
                title="聚光灯"
                onClick={() => pickTool('spotlight')}
              >
                <SpotlightIcon />
              </button>
              <button
                data-tool="text"
                className={`sbtn ${tool === 'text' ? 'active' : ''}`}
                title="文字"
                onClick={() => pickTool('text')}
              >
                <TextIcon />
              </button>
              <button
                data-tool="badge"
                className={`sbtn ${tool === 'badge' ? 'active' : ''}`}
                title="序号"
                onClick={() => pickTool('badge')}
              >
                <SequenceIcon />
              </button>
              <button
                data-tool="eraser"
                className={`sbtn ${tool === 'eraser' ? 'active' : ''}`}
                title="橡皮擦（擦掉已画的标注）"
                onClick={() => pickTool('eraser')}
              >
                <EraserIcon />
              </button>
              <button
                data-tool="watermark"
                className={`sbtn ${tool === 'watermark' ? 'active' : ''}`}
                title="水印（拖选区域后输入文字）"
                onClick={() => pickTool('watermark')}
              >
                <WatermarkIcon />
              </button>
              {/* 放大镜（zoom）按钮按用户要求撤下：
                  工具本身的实现保留（Shape::Zoom / 手势代码仍在），
                  只是不在工具条上给入口，避免节距变宽。 */}
              <button
                data-tool="smartErase"
                className={`sbtn ${tool === 'smartErase' ? 'active' : ''}`}
                title="智能擦除（内容感知填充）"
                onClick={() => pickTool('smartErase')}
              >
                <SmartEraseIcon />
              </button>
            </div>

            <i className="sb-divider" />

            {/* 第二组：识别类 */}
            <div className="sb-group">
              <button
                className={`sbtn ${transBusy ? 'active' : ''}`}
                title="翻译（框选后一键识别并翻译，左原文右译文）"
                disabled={transBusy}
                onClick={(e) => void runTranslate(e.currentTarget)}
              >
                <TranslateIcon />
              </button>
              <button
                className={`sbtn ${ocrBusy || ocrError ? 'active' : ''}`}
                title={
                  modelsBusy
                    ? '正在下载 OCR 模型（约 15.6MB）…'
                    : ocrModelsOk === false
                      ? '文字识别（首次使用会自动下载模型，约 15.6MB）'
                      : '文字识别（识别当前选区内的文字，支持中文）'
                }
                onClick={(e) => void runOcr(e.currentTarget)}
              >
                <OcrIcon />
                {ocrModelsOk === false && <i className="sb-dot" title="模型缺失" />}
              </button>
              <button
                data-tool="qr"
                className={`sbtn ${tool === 'qr' ? 'active' : ''}`}
                title="二维码识别（拖选区域，支持一图多码）"
                onClick={() => pickTool('qr')}
              >
                <QrIcon />
              </button>
              <button
                className="sbtn"
                title="长截图（自动滚动拼接整个页面，Esc 停止）"
                disabled={longMode === 'running'}
                onClick={(e) => {
                  if (!rect) {
                    showHint('请先框选要长截图的区域', e.currentTarget, '')
                    return
                  }
                  void startLong()
                }}
              >
                <LongShotIcon />
              </button>
            </div>

            <i className="sb-divider" />

            {/* 第三组：通用动作 */}
            <div className="sb-group">
              <button
                className="sbtn"
                title="撤销 (Ctrl+Z)"
                disabled={shapes.length === 0}
                onClick={undoShape}
              >
                <UndoIcon />
              </button>
              <button
                className="sbtn"
                title="重做 (Ctrl+Shift+Z / Ctrl+Y)"
                disabled={redoCount === 0}
                onClick={redoShape}
              >
                <RedoIcon />
              </button>
              <button
                className="sbtn"
                title="保存到文件"
                disabled={busy}
                onClick={() => void finish('save')}
              >
                <SaveIcon />
              </button>
              <button
                className="sbtn"
                title="贴图到屏幕"
                disabled={busy}
                onClick={() => void finish('pin')}
              >
                <PinIcon />
              </button>
              {/* 分享按钮：按要求暂时注释掉，功能与「完成并复制」重复，保留代码便于恢复
              <button
                className="sbtn"
                title="复制并分享"
                disabled={busy}
                onClick={() => void finish('copy')}
              >
                <ShareIcon />
              </button>
              */}
              <button className="sbtn close" title="取消截图 (Esc)" onClick={() => void exitCapture()}>
                <CloseIcon />
              </button>
              <button
                className="sbtn confirm"
                title="完成并复制 (Enter)"
                disabled={busy}
                onClick={() => void finish('copy')}
              >
                <CheckIcon />
              </button>
            </div>
          </div>

          {/* 工具属性（参考图里没有：仅选中工具后出现在工具条下方，保持主条 1:1） */}
          {tool && (
            <div
              className={`sb-props${tool === 'sticker' ? ' pad-10' : ''}`}
              ref={propsRef}
              style={
                anchor && barBox
                  ? {
                      // 只对齐左边缘与顶边，**不设 width**：宽度由内容决定。
                      // 之前设成和一级条同宽，窄工具条后面就拖出一大截空白
                      // （贴纸面板搜索框右侧那片"多余"就是这么来的）。
                      left: barBox.left,
                      top: barBox.bottom + 6,
                    }
                  : undefined
              }
              onPointerDown={(e) => e.stopPropagation()}
            >
              <ToolProps
                tool={tool}
                color={color}
                width={width}
                fontSize={fontSize}
                spotDim={spotDim}
                mosaicBlock={mosaicBlock}
                eraserRadius={eraserRadius}
                zoomLevel={zoomLevel}
                onColor={setColor}
                onWidth={setWidth}
                onFontSize={setFontSize}
                onSpotDim={applySpotDim}
                onMosaicBlock={setMosaicBlock}
                onEraserRadius={setEraserRadius}
                onZoomLevel={setZoomLevel}
                sticker={sticker}
                onSticker={setSticker}
                stickerColor={stickerColor}
                onStickerColor={setStickerColor}
                stickerSize={stickerSize}
                onStickerSize={setStickerSize}
                onTool={setTool}
              />
            </div>
          )}

          {hint && (
            <div
              className="sb-hint"
              ref={hintRef}
              style={
                hintAnchor
                  ? { left: anchorX(hintAnchor.x, hintW || 170), top: hintAnchor.bottom + 6 }
                  : undefined
              }
            >
              {hint}
            </div>
          )}

          {/* 取色器气泡：跟随指针，左侧色块 + HEX + RGB。
              指针靠近右/下边缘时翻到另一侧，避免被屏幕裁掉。 */}
          {tool === 'pick' && pick && (
            <div
              className="pick-chip"
              style={{
                left: pick.x + 18 + 132 > window.innerWidth ? pick.x - 150 : pick.x + 18,
                top: pick.y + 18 + 64 > window.innerHeight ? pick.y - 70 : pick.y + 18,
              }}
            >
              <i className="pick-swatch" style={{ background: pick.hex }} />
              <span className="pick-hex">{pick.hex}</span>
              <span className="pick-rgb">
                {pick.rgb[0]},{pick.rgb[1]},{pick.rgb[2]}
              </span>
            </div>
          )}
          {tool === 'pick' && copiedHex && <div className="pick-toast">已复制 {copiedHex}</div>}

          {/* 二维码识别结果面板：贴在选区下方（空间不足时翻到上方）。
              空结果与错误都明确告知，不做静默失败。 */}
          {(qrBusy || qrCodes !== null || qrError) && rect && (
            <div
              className="qr-panel"
              style={{
                left: Math.max(0, Math.min(rect.x, window.innerWidth - 300)),
                top:
                  rect.y + rect.h + 8 + 220 > window.innerHeight
                    ? Math.max(0, rect.y - 228)
                    : rect.y + rect.h + 8,
              }}
            >
              <div className="qr-head">
                <span>{qrBusy ? '识别中…' : `识别到 ${qrCodes?.length ?? 0} 个码`}</span>
                <button
                  className="qr-close"
                  title="关闭"
                  onClick={() => {
                    setQrCodes(null)
                    setQrError(null)
                  }}
                >
                  ×
                </button>
              </div>
              {qrError && <div className="qr-error">{qrError}</div>}
              {!qrBusy && !qrError && qrCodes?.length === 0 && (
                <div className="qr-empty">未识别到二维码 / 条码</div>
              )}
              {qrCodes?.map((c, i) => (
                <div className="qr-row" key={`${c.format}-${i}`}>
                  <span className="qr-format">{c.format}</span>
                  <span className="qr-text" title={c.text}>
                    {c.text}
                  </span>
                  <button
                    className="qr-btn"
                    onClick={() => void copyText(c.text).catch((e) => uiLog(`copy failed: ${String(e)}`))}
                  >
                    复制
                  </button>
                  {/^https?:\/\//i.test(c.text) && (
                    <button className="qr-btn" onClick={() => void openUrl(c.text).catch(() => {})}>
                      打开
                    </button>
                  )}
                </div>
              ))}
            </div>
          )}

          {/* 长截图状态条（滚动拼接中，M5）：缩略图实时长高 + 高度/步数 + Esc 提示。
              此时浮层整体光标穿透（Rust 侧设置），工具条点不到，故常驻此条。 */}
          {longMode === 'running' && rect && (
            <div
              className="long-bar"
              onPointerDown={(e) => e.stopPropagation()}
              style={{
                left: Math.max(0, Math.min(rect.x, window.innerWidth - 300)),
                top:
                  rect.y + rect.h + 8 + 120 > window.innerHeight
                    ? Math.max(0, rect.y - 128)
                    : rect.y + rect.h + 8,
              }}
            >
              {longProgress?.thumb ? (
                <img
                  className="long-thumb"
                  src={`${imgSrc(longProgress.thumb)}?t=${longProgress.steps}`}
                  alt=""
                  draggable={false}
                />
              ) : (
                <div className="long-thumb long-thumb-empty" />
              )}
              <div className="long-info">
                <span className="long-msg">{longProgress?.message ?? '正在启动…'}</span>
                <span className="long-sub">
                  {longProgress
                    ? `${longProgress.height} px · ${longProgress.steps} 步`
                    : '准备中'}{' '}
                  · 按 Esc 停止
                </span>
              </div>
            </div>
          )}

          {/* 等「辅助功能」授权（macOS）：Rust 已把浮层取消置顶并打开系统设置，
              用户勾选后轮询自动继续。卡片给出三个出口：打开设置 / 立即重试 / 退出。 */}
          {longMode === 'blocked' && (
            <div className="long-result long-wait" onPointerDown={(e) => e.stopPropagation()}>
              <div className="long-head">
                <span>需要「辅助功能」权限</span>
                <button
                  className="long-close"
                  title="退出截图 (Esc)"
                  onClick={() => void exitCapture()}
                >
                  ×
                </button>
              </div>
              <div className="long-msg-row">
                长截图的自动滚动需要这项权限。已在系统设置打开「辅助功能」，勾选 SnapPin
                后这里会<strong>自动开始</strong>，不需要重启。若列表里已有 SnapPin
                且显示已勾选，请先点「−」删掉再重新添加（换版本后旧条目会失效）。
              </div>
              <div className="long-actions">
                <button
                  className="long-btn"
                  onClick={() => void accessibilitySettingsOpen().catch(() => {})}
                >
                  打开系统设置
                </button>
                <button
                  className="long-btn"
                  title="列表里已有 SnapPin 且已勾选却仍无效时用这个：清掉旧授权记录再重新勾选"
                  onClick={() => {
                    uiLog('long: reset accessibility grant')
                    void accessibilityReset().catch((e) =>
                      uiLog(`accessibility reset failed: ${String(e)}`),
                    )
                  }}
                >
                  重置授权
                </button>
                <button
                  className="long-btn"
                  onClick={() => {
                    uiLog('long: manual recheck accessibility')
                    void startLong()
                  }}
                >
                  我已授权，重试
                </button>
              </div>
            </div>
          )}

          {/* 长截图结果面板（M5）：缩略大图 + 保存/复制/贴图 + 退出。
              到底 / Esc 停止 / 出错都会到这里；出错时仍可保存已拼部分。 */}
          {longMode === 'result' && rect && (
            <div
              className="long-result"
              onPointerDown={(e) => e.stopPropagation()}
            >
              <div className="long-head">
                <span>
                  {longProgress?.phase === 'error'
                    ? '长截图出错'
                    : `长截图完成 · ${longProgress?.height ?? 0} px`}
                </span>
                <button
                  className="long-close"
                  title="退出截图 (Esc)"
                  onClick={() => void exitCapture()}
                >
                  ×
                </button>
              </div>
              {longProgress?.thumb && (
                <img
                  className="long-thumb-big"
                  src={`${imgSrc(longProgress.thumb)}?t=done`}
                  alt=""
                  draggable={false}
                />
              )}
              <div className="long-msg-row">{longProgress?.message ?? '长截图已结束'}</div>
              {longActError && <div className="long-error">{longActError}</div>}
              <div className="long-actions">
                <button
                  className="long-btn"
                  disabled={longCommit !== null}
                  onClick={() => void commitLong('save')}
                >
                  {longCommit === 'save' ? '保存中…' : '保存'}
                </button>
                <button
                  className="long-btn"
                  disabled={longCommit !== null}
                  onClick={() => void commitLong('copy')}
                >
                  {longCommit === 'copy' ? '复制中…' : '复制'}
                </button>
                <button
                  className="long-btn"
                  disabled={longCommit !== null}
                  onClick={() => void commitLong('pin')}
                >
                  {longCommit === 'pin' ? '贴图中…' : '贴图'}
                </button>
              </div>
            </div>
          )}

          {actError && <div className="sb-hint error">{actError}</div>}
        </>
      )}
    </div>
  )
}
