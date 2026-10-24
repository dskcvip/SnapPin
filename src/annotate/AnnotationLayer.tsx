import { useEffect, useRef, useState, type PointerEvent as ReactPointerEvent } from 'react'
import {
  BADGE_DIAMETER_RATIO,
  HIGHLIGHTER_COLOR,
  SPOTLIGHT_RADIUS,
  TEXT_FONT_STACK,
  TEXT_LINE_HEIGHT,
  WATERMARK_DEFAULT_TEXT,
  WATERMARK_GAP,
  isActionTool,
  nextBadgeNo,
  toolMeta,
  type AnnotateTool,
  type Rgba,
  type ShapeDraft,
} from './tools'

/** 模块级复用的马赛克源图（按物理像素 1:1 提取冻结图，再做块平均）。 */
const mosaicSrc = document.createElement('canvas')

/** 模块级复用的聚光灯遮罩画布（尺寸随主画布，见 drawSpotlightPreview）。 */
const dimCanvas = document.createElement('canvas')

/** 模块级复用的智能擦除边界采样条（2 列 × 区域高，见 drawSmartErasePreview）。 */
const stripCanvas = document.createElement('canvas')

/** 拖拽成形的最小尺寸（逻辑像素），小于此视为单击忽略。 */
const MIN_DRAG = 3

interface Props {
  /** 冻结图物理像素尺寸 */
  naturalWidth: number
  naturalHeight: number
  /** DPI 缩放因子（物理/逻辑） */
  scaleFactor: number
  /** 冻结图 <img> 元素（**可能污染画布**，仅在位图未就绪时兜底） */
  imageEl: HTMLImageElement | null
  /**
   * 冻结图的 ImageBitmap（fetch → Blob → 解码）。
   *
   * 为什么必须单独传：`asset.localhost` 的 <img> 画进 canvas 会让画布
   * **跨域污染**，`getImageData` 直接抛 SecurityError。而马赛克预览正是在
   * 渲染路径里读像素 —— 抛在这个位置会把整棵 React 树卸掉（实测：一用
   * 马赛克浮层就消失）。位图来自 Blob，读取合法。
   */
  bitmap: ImageBitmap | null
  /** null = 未选工具（选择态）：画布不拦截鼠标，可在选区内重新框选 */
  tool: AnnotateTool | null
  color: Rgba
  width: number
  mosaicBlock: number
  /** 文字字号（逻辑像素） */
  fontSize: number
  /** 聚光灯压暗强度（0-255） */
  spotlightDim: number
  /** 橡皮擦半径（逻辑像素） */
  eraserRadius: number
  /** 放大镜倍数 */
  zoomLevel: number
  /** 当前选中的贴纸字形（emoji）；tool=sticker 时单击画布贴它 */
  stickerGlyph: string
  shapes: ShapeDraft[]
  onShapesChange: (shapes: ShapeDraft[]) => void
  /** 取色器：指针在选区内移动时持续回调（局部逻辑像素坐标）。
   *  由外层负责采样像素与渲染取色气泡——本层只管把手势坐标递出去。 */
  onPickSample?: (x: number, y: number) => void
  /** 取色器：松开指针时回调一次（外层此时把 HEX 写进剪贴板）。 */
  onPickEnd?: (x: number, y: number) => void
  /** 二维码：拖出区域后回调一次（外层调 Rust 识别并展示结果）。 */
  onQrRegion?: (x: number, y: number, w: number, h: number) => void
  /** 二维码识别结果的高亮框。定位点为**选区内**逻辑像素坐标
   *  （Rust 侧已从物理像素换算回来），与本层画布同坐标系。
   *  null = 不高亮（未识别 / 面板已关）。 */
  qrHighlights?: { points: [number, number][] }[] | null
}

/** 文字输入会话（textarea 的临时状态，提交后转为 bitmap 图形）。 */
interface TextEdit {
  /** 会话代号：区分"新开一个输入框"与"旧输入框迟到的 blur" */
  id: number
  x: number
  y: number
  value: string
  color: Rgba
  fontSize: number
}

/**
 * 标注层：覆盖在冻结图上的 Canvas，负责标注的渲染与鼠标交互。
 *
 * 画法与 Rust 光栅化（core-algo::draw）刻意对齐：
 * 圆头笔帽（lineCap/lineJoin=round）、无平滑的折线铅笔、
 * 25° 箭头两翼、块平均马赛克。Canvas 自带抗锯齿的细微差异可接受。
 */
export function AnnotationLayer(props: Props) {
  const {
    naturalWidth,
    naturalHeight,
    scaleFactor,
    imageEl,
    bitmap,
    tool,
    color,
    width,
    mosaicBlock,
    fontSize,
    spotlightDim,
    eraserRadius,
    zoomLevel,
    shapes,
    onShapesChange,
    onPickSample,
    onPickEnd,
    onQrRegion,
    qrHighlights,
  } = props
  const canvasRef = useRef<HTMLCanvasElement | null>(null)
  /** 正在拖拽中的草稿（未提交） */
  const draftRef = useRef<ShapeDraft | null>(null)
  const drawingRef = useRef(false)
  /** 本次拖拽的起点（逻辑像素）：矩形/椭圆/马赛克需要它做规范化，
   *  否则向左上拖时 x/y 被逐步改写，宽高会一直为 0。 */
  const startRef = useRef<{ x: number; y: number } | null>(null)
  /** 文字输入框状态。用 ref 做唯一真源：提交路径有两条（blur / 点击画布），
   *  两条都会先清空 ref，保证不会重复提交同一段文字。 */
  const editingRef = useRef<TextEdit | null>(null)
  /** 待生成的水印区域（水印工具拖出区域后开输入框，提交时消费）。
   *  与 editingRef 分离：输入框可以随时被 Esc 关掉，区域状态要跟着一起清。 */
  const pendingWmRef = useRef<{ x: number; y: number; w: number; h: number } | null>(null)
  const [editing, setEditingState] = useState<TextEdit | null>(null)
  const editSeq = useRef(0)
  const taRef = useRef<HTMLTextAreaElement | null>(null)
  /** 二维码拖选中的区域预览。动作类工具不产生 ShapeDraft，
   *  预览框由本层自己画（虚线，与水印拖拽中的虚线框同一观感）。 */
  const [actionRect, setActionRect] = useState<{
    x: number
    y: number
    w: number
    h: number
  } | null>(null)
  const setEditing = (v: TextEdit | null) => {
    editingRef.current = v
    setEditingState(v)
  }

  /** 在 (x, y) 开一个新的文字输入会话（逻辑像素）。
   *  `preset` 为预填文字（水印工具用，省去用户先想写什么的一步）。 */
  const startText = (x: number, y: number, preset = '') => {
    editSeq.current += 1
    setEditing({ id: editSeq.current, x, y, value: preset, color, fontSize })
  }

  /** 提交文字：光栅化成位图图形（空内容直接丢弃）。
   *  若存在 pendingWmRef（水印工具拖出的区域），则生成水印图形而不是文字位图。 */
  const commitText = (sessionId?: number) => {
    const ed = editingRef.current
    if (!ed) return
    // 旧输入框迟到的 blur（新会话已开始）不处理
    if (sessionId !== undefined && ed.id !== sessionId) return
    setEditing(null)
    const wm = pendingWmRef.current
    pendingWmRef.current = null
    const r = rasterizeText(ed.value, ed.color, ed.fontSize, props.scaleFactor)
    if (!r) return
    if (wm) {
      // 水印：同一块文字位图按步进平铺进整个区域（Rust 侧只做重复 blit）
      onShapesChange([
        ...shapes,
        {
          type: 'watermark',
          x: wm.x,
          y: wm.y,
          w: wm.w,
          h: wm.h,
          stepX: round2(r.w + WATERMARK_GAP.x),
          stepY: round2(r.h + WATERMARK_GAP.y),
          png: r.png,
          canvas: r.canvas,
          tileW: round2(r.w),
          tileH: round2(r.h),
        },
      ])
      return
    }
    onShapesChange([
      ...shapes,
      {
        type: 'bitmap',
        x: round2(ed.x - r.padX),
        y: round2(ed.y - r.padY),
        w: round2(r.w),
        h: round2(r.h),
        png: r.png,
        canvas: r.canvas,
      },
    ])
  }

  /** 文字输入框自适应内容大小并保持焦点 */
  useEffect(() => {
    const el = taRef.current
    if (!el) return
    el.style.height = 'auto'
    el.style.height = `${el.scrollHeight}px`
    el.style.width = 'auto'
    el.style.width = `${Math.max(140, el.scrollWidth + 2)}px`
    if (document.activeElement !== el) {
      el.focus()
      const n = el.value.length
      el.setSelectionRange(n, n)
    }
  }, [editing?.value, editing?.x, editing?.y])

  // 重绘：shapes 全量 + 当前草稿
  useEffect(() => {
    const canvas = canvasRef.current
    if (!canvas) return
    const ctx = canvas.getContext('2d')
    if (!ctx) return
    ctx.setTransform(1, 0, 0, 1, 0, 0)
    ctx.clearRect(0, 0, canvas.width, canvas.height)
    // 之后全部用逻辑像素坐标 API
    ctx.setTransform(scaleFactor, 0, 0, scaleFactor, 0, 0)
    ctx.lineCap = 'round'
    ctx.lineJoin = 'round'
    for (const s of shapes) drawShape(ctx, s, imageEl, scaleFactor, bitmap)
    if (draftRef.current) drawShape(ctx, draftRef.current, imageEl, scaleFactor, bitmap)
    if (actionRect) drawDashRect(ctx, actionRect)
    if (qrHighlights) drawQrHighlights(ctx, qrHighlights)
  })

  const localPoint = (e: ReactPointerEvent<HTMLCanvasElement>) => {
    const rect = e.currentTarget.getBoundingClientRect()
    return { x: e.clientX - rect.left, y: e.clientY - rect.top }
  }

  const onPointerDown = (e: ReactPointerEvent<HTMLCanvasElement>) => {
    if (e.button !== 0 || !tool) return
    e.stopPropagation()
    e.preventDefault()
    const p = localPoint(e)
    // 动作类工具（取色 / 二维码）：不产生 ShapeDraft，只把手势坐标交给外层。
    // 提前分流，避免落到下面"按工具造草稿"的分支里被当成 rect 之类处理。
    if (isActionTool(tool)) {
      drawingRef.current = true
      startRef.current = { x: round2(p.x), y: round2(p.y) }
      setActionRect(null)
      // 取色器按下即取一次色：单击（不拖动）也能取到色
      if (tool === 'pick') onPickSample?.(p.x, p.y)
      return
    }
    // 文字：单击落点开输入框（不是拖拽成形）。
    // 已有输入框时先提交，并**延后一帧再开新框**——鼠标按下会让旧输入框失焦，
    // 若同帧内就把新会话塞进去，那条迟到的 blur 会把刚开的新输入框直接提交掉
    // （表现为"点第二下打不了字"）。先提交 → 引用置空 → blur 空转 → 再开新框。
    if (tool === 'text') {
      // 切到普通文字：作废可能残留的水印待生成区域（防御，正常路径已消费）
      pendingWmRef.current = null
      if (editingRef.current) {
        commitText()
        const nx = round2(p.x)
        const ny = round2(p.y)
        window.setTimeout(() => {
          if (!editingRef.current) startText(nx, ny)
        }, 0)
      } else {
        startText(round2(p.x), round2(p.y))
      }
      return
    }
    // 贴纸：单击落点贴一个 emoji。与序号同一套位图通道，同样按圆心落点。
    if (tool === 'sticker') {
      const r = rasterizeSticker(props.stickerGlyph, fontSize, props.scaleFactor)
      if (!r) return
      onShapesChange([
        ...shapes,
        {
          type: 'bitmap',
          x: round2(p.x - r.w / 2),
          y: round2(p.y - r.h / 2),
          w: round2(r.w),
          h: round2(r.h),
          png: r.png,
          canvas: r.canvas,
        },
      ])
      return
    }
    // 序号：单击即在落点盖一个编号（编号 = 现有最大号 + 1，撤销后自动回填）。
    // 不做拖拽——徽标是"点标"而不是"框选"，按下即出结果反馈最快。
    if (tool === 'badge') {
      const n = nextBadgeNo(shapes)
      const r = rasterizeBadge(n, color, fontSize, props.scaleFactor)
      if (!r) return
      onShapesChange([
        ...shapes,
        {
          type: 'bitmap',
          // 落点 = 圆心（与文字"落点 = 左上角"不同，见 rasterizeBadge 注释）
          x: round2(p.x - r.w / 2),
          y: round2(p.y - r.h / 2),
          w: round2(r.w),
          h: round2(r.h),
          png: r.png,
          canvas: r.canvas,
          badge: n,
        },
      ])
      return
    }
    drawingRef.current = true
    const meta = toolMeta(tool)
    startRef.current = { x: round2(p.x), y: round2(p.y) }
    if (tool === 'eraser') {
      // 橡皮擦走轨迹链路，但半径独立于线宽（擦除半径通常比笔触粗）
      draftRef.current = {
        type: 'eraser',
        points: [[round2(p.x), round2(p.y)]],
        radius: eraserRadius,
      }
    } else if (meta.trajectory) {
      draftRef.current = {
        type: tool === 'pen' ? 'pen' : 'highlighter',
        points: [[round2(p.x), round2(p.y)]],
        color: tool === 'pen' ? color : HIGHLIGHTER_COLOR,
        width,
      }
    } else if (tool === 'mosaic') {
      draftRef.current = {
        type: 'mosaic',
        x: round2(p.x),
        y: round2(p.y),
        w: 0,
        h: 0,
        block: mosaicBlock,
      }
    } else if (tool === 'spotlight') {
      // 聚光灯复用矩形拖拽链路（同样用 startRef 做规范化，支持任意方向拖）
      draftRef.current = {
        type: 'spotlight',
        x: round2(p.x),
        y: round2(p.y),
        w: 0,
        h: 0,
        radius: SPOTLIGHT_RADIUS,
        dim: spotlightDim,
      }
    } else if (tool === 'zoom') {
      // 放大镜：拖出区域即放大（预览与 Rust 都从**原图**取样）
      draftRef.current = {
        type: 'zoom',
        x: round2(p.x),
        y: round2(p.y),
        w: 0,
        h: 0,
        zoom: zoomLevel,
        color,
        width,
      }
    } else if (tool === 'smartErase') {
      // 智能擦除：iterations=0 表示 Rust 侧按区域尺寸自动收敛
      draftRef.current = {
        type: 'smartErase',
        x: round2(p.x),
        y: round2(p.y),
        w: 0,
        h: 0,
        iterations: 0,
      }
    } else if (tool === 'watermark') {
      // 水印：先拖区域（此时还没有文字位图，预览只画虚线框），
      // 抬起后再开输入框，见 onPointerUp
      draftRef.current = {
        type: 'watermark',
        x: round2(p.x),
        y: round2(p.y),
        w: 0,
        h: 0,
        stepX: 0,
        stepY: 0,
        png: '',
        tileW: 0,
        tileH: 0,
      }
    } else if (tool === 'arrow') {
      draftRef.current = {
        type: 'arrow',
        x1: round2(p.x),
        y1: round2(p.y),
        x2: round2(p.x),
        y2: round2(p.y),
        color,
        width,
      }
    } else {
      // 剩余：rect / ellipse（拖拽成形，宽高从 0 开始）
      draftRef.current = {
        type: tool as 'rect' | 'ellipse',
        x: round2(p.x),
        y: round2(p.y),
        w: 0,
        h: 0,
        color,
        width,
      }
    }
  }

  const onPointerMove = (e: ReactPointerEvent<HTMLCanvasElement>) => {
    if (!drawingRef.current) return
    e.stopPropagation()
    const p = localPoint(e)
    // 动作类工具没有草稿图形，提前分流
    if (tool === 'pick') {
      onPickSample?.(p.x, p.y)
      return
    }
    if (tool === 'qr') {
      const s0 = startRef.current ?? { x: p.x, y: p.y }
      setActionRect({
        x: Math.min(s0.x, round2(p.x)),
        y: Math.min(s0.y, round2(p.y)),
        w: Math.abs(round2(p.x) - s0.x),
        h: Math.abs(round2(p.y) - s0.y),
      })
      return
    }
    const d = draftRef.current
    if (!d) return
    if (d.type === 'pen' || d.type === 'highlighter' || d.type === 'eraser') {
      const last = d.points[d.points.length - 1]
      // 去抖：位移过小不记录（减小数据量与 IPC 体积）
      if (Math.abs(p.x - last[0]) < 0.75 && Math.abs(p.y - last[1]) < 0.75) return
      d.points.push([round2(p.x), round2(p.y)])
      redraw(props, draftRef.current, canvasRef.current, imageEl, bitmap)
      return
    }
    // 拖拽成形的（rect/ellipse/arrow/mosaic/spotlight/zoom/smartErase/watermark）
    if (d.type === 'arrow') {
      d.x2 = round2(p.x)
      d.y2 = round2(p.y)
    } else {
      // 用起点重算，支持任意方向拖拽（含向左上）
      const s0 = startRef.current ?? { x: d.x, y: d.y }
      const cx = round2(p.x)
      const cy = round2(p.y)
      d.x = Math.min(s0.x, cx)
      d.y = Math.min(s0.y, cy)
      d.w = Math.abs(cx - s0.x)
      d.h = Math.abs(cy - s0.y)
    }
    redraw(props, d, canvasRef.current, imageEl, bitmap)
  }

  const onPointerUp = (e: ReactPointerEvent<HTMLCanvasElement>) => {
    if (!drawingRef.current) return
    e.stopPropagation()
    drawingRef.current = false
    const p = localPoint(e)
    const s0 = startRef.current
    startRef.current = null
    // 动作类工具收尾
    if (tool === 'pick') {
      setActionRect(null)
      // 松开即复制：取色器的完整交互是"按住移动看颜色，松手定下并复制"
      onPickEnd?.(p.x, p.y)
      return
    }
    if (tool === 'qr') {
      const w = Math.abs(p.x - (s0?.x ?? p.x))
      const h = Math.abs(p.y - (s0?.y ?? p.y))
      setActionRect(null)
      // 单击（区域过小）不触发识别：多半只是切工具时误点
      if (s0 && w >= MIN_DRAG && h >= MIN_DRAG) {
        onQrRegion?.(Math.min(s0.x, p.x), Math.min(s0.y, p.y), w, h)
      }
      return
    }
    const d = draftRef.current
    draftRef.current = null
    if (!d) return
    if (d.type === 'pen' || d.type === 'highlighter') {
      if (d.points.length >= 2) onShapesChange([...shapes, d])
    } else if (d.type === 'eraser') {
      // 单击也成立：单点擦一个小圆点（与 Rust eraser_region 的单点圆盘一致）
      onShapesChange([...shapes, d])
    } else if (d.type === 'arrow') {
      // 箭头用两端点距离判定，太短视为误触
      if (Math.hypot(d.x2 - d.x1, d.y2 - d.y1) >= MIN_DRAG) onShapesChange([...shapes, d])
    } else if (d.type === 'watermark') {
      // 区域够大：记住区域并开文字输入框（预填默认文字，用户直接改写）。
      // 提交逻辑在 commitText：文字光栅化成平铺块，生成水印图形。
      if (d.w >= MIN_DRAG && d.h >= MIN_DRAG) {
        pendingWmRef.current = { x: d.x, y: d.y, w: d.w, h: d.h }
        startText(d.x, d.y, WATERMARK_DEFAULT_TEXT)
      }
    } else if (d.w >= MIN_DRAG && d.h >= MIN_DRAG) {
      onShapesChange([...shapes, d])
    }
    redraw(props, null, canvasRef.current, imageEl, bitmap)
  }

  return (
    <>
      <canvas
        ref={canvasRef}
        className={`annotate-canvas ${tool ? '' : 'passthrough'} ${
          tool === 'text' ? 'text-mode' : ''
        }`}
        width={naturalWidth}
        height={naturalHeight}
        style={{ width: '100%', height: '100%' }}
        onPointerDown={onPointerDown}
        onPointerMove={onPointerMove}
        onPointerUp={onPointerUp}
      />
      {/* 文字输入框：样式刻意与光栅化结果对齐（同字体栈/同字号/同行高），
          这样"打字时看到的位置"就是"最终图上的位置"。用 textarea 而非
          contenteditable 是为了中文输入法的候选框与组合态天然可用。 */}
      {editing && (
        <textarea
          ref={taRef}
          key={editing.id}
          className="text-editor"
          value={editing.value}
          spellCheck={false}
          autoComplete="off"
          autoCorrect="off"
          autoCapitalize="off"
          style={{
            left: editing.x,
            top: editing.y,
            color: rgba(editing.color),
            fontSize: editing.fontSize,
            lineHeight: TEXT_LINE_HEIGHT,
          }}
          onChange={(e) => {
            const cur = editingRef.current
            if (cur) setEditing({ ...cur, value: e.target.value })
          }}
          onPointerDown={(e) => e.stopPropagation()}
          onBlur={() => commitText(editing.id)}
          onKeyDown={(e) => {
            if (e.key === 'Escape') {
              // 只关输入框，不退出截图；水印的待生成区域一并作废
              e.preventDefault()
              e.stopPropagation()
              pendingWmRef.current = null
              setEditing(null)
            } else if (e.key === 'Enter' && (e.ctrlKey || e.metaKey)) {
              e.preventDefault()
              commitText()
            }
          }}
        />
      )}
    </>
  )
}

function round2(v: number): number {
  return Math.round(v * 100) / 100
}

/** 动作类工具的拖选预览：虚线框（二维码选区用）。 */
function drawDashRect(
  ctx: CanvasRenderingContext2D,
  r: { x: number; y: number; w: number; h: number },
) {
  if (r.w <= 0 || r.h <= 0) return
  ctx.save()
  ctx.setLineDash([4, 3])
  ctx.strokeStyle = 'rgba(120,120,120,0.9)'
  ctx.lineWidth = 1
  ctx.strokeRect(r.x, r.y, r.w, r.h)
  ctx.restore()
}

/** 二维码高亮框：定位点的外接框 + 序号圆点（序号与结果面板的行序一致）。
 *
 *  为什么要画框：识别结果面板只告诉用户"找到了什么"，框才告诉用户
 *  "在哪里"——一屏多个码时，没有框根本分不清哪条内容对应哪个码。
 *  画在标注之上（本函数在 redraw 的最后调用），所以不会被标注盖住。 */
function drawQrHighlights(
  ctx: CanvasRenderingContext2D,
  codes: { points: [number, number][] }[],
) {
  codes.forEach((c, i) => {
    if (c.points.length === 0) return
    let minX = Infinity
    let minY = Infinity
    let maxX = -Infinity
    let maxY = -Infinity
    for (const [x, y] of c.points) {
      if (x < minX) minX = x
      if (y < minY) minY = y
      if (x > maxX) maxX = x
      if (y > maxY) maxY = y
    }
    const w = maxX - minX
    const h = maxY - minY
    if (w <= 0 || h <= 0) return
    const pad = 3
    ctx.save()
    ctx.strokeStyle = '#2563eb'
    ctx.lineWidth = 1.6
    ctx.setLineDash([])
    ctx.strokeRect(minX - pad, minY - pad, w + pad * 2, h + pad * 2)
    // 序号圆点：半径按逻辑像素固定，位置在框的左上角外侧
    const r = 7
    const cx = minX - pad
    const cy = minY - pad
    ctx.beginPath()
    ctx.arc(cx, cy, r, 0, Math.PI * 2)
    ctx.fillStyle = '#2563eb'
    ctx.fill()
    ctx.fillStyle = '#fff'
    ctx.font = `600 ${Math.round(r * 1.3)}px system-ui, "Segoe UI", sans-serif`
    ctx.textAlign = 'center'
    ctx.textBaseline = 'middle'
    ctx.fillText(String(i + 1), cx, cy + 0.5)
    ctx.restore()
  })
}

/** 全量重绘（草稿 pass=null 表示只画已确认 shapes）。 */
function redraw(
  props: Props,
  draft: ShapeDraft | null,
  canvas: HTMLCanvasElement | null,
  imageEl: HTMLImageElement | null,
  bitmap: ImageBitmap | null,
) {
  if (!canvas) return
  const ctx = canvas.getContext('2d')
  if (!ctx) return
  ctx.setTransform(1, 0, 0, 1, 0, 0)
  ctx.clearRect(0, 0, canvas.width, canvas.height)
  ctx.setTransform(props.scaleFactor, 0, 0, props.scaleFactor, 0, 0)
  ctx.lineCap = 'round'
  ctx.lineJoin = 'round'
  for (const s of props.shapes) drawShape(ctx, s, imageEl, props.scaleFactor, bitmap)
  if (draft) drawShape(ctx, draft, imageEl, props.scaleFactor, bitmap)
}

function rgba(color: Rgba): string {
  return `rgba(${color[0]},${color[1]},${color[2]},${color[3] / 255})`
}

/** 光栅化结果。`w`/`h`/`padX`/`padY` 已换算回逻辑像素，方便与 shape 坐标对齐。 */
export interface TextRaster {
  canvas: HTMLCanvasElement
  /** base64 PNG（无 data: 前缀），物理像素 */
  png: string
  w: number
  h: number
  /** 墨迹相对位图左上的内缩（逻辑像素） */
  padX: number
  padY: number
}

/**
 * 把一段文字光栅化成 RGBA 位图（**物理像素**，即按 DPR 放大后绘制）。
 *
 * 为什么在 UI 层做而不是 Rust 层：见 core-algo::shapes::Shape::Bitmap 的文档
 * ——核心层必须零依赖，字体与中文回退只能靠系统。这里用 Canvas 的
 * `fillText` 拿到的是"系统真实字体渲染结果"，因此中文、emoji、任意
 * 已安装字体都可用，且与屏幕上看到的完全一致。
 *
 * 返回 `null` 表示没有可见内容（全空白），调用方应直接丢弃。
 */
export function rasterizeText(
  text: string,
  color: Rgba,
  fontSize: number,
  sf: number,
): TextRaster | null {
  const scale = sf > 0 ? sf : 1
  const lines = text.replace(/\r\n?/g, '\n').split('\n')
  // 尾部空行不撑高图片（用户习惯性回车很常见）
  while (lines.length > 1 && lines[lines.length - 1].trim() === '') lines.pop()
  if (!lines.some((l) => l.trim() !== '')) return null

  const px = Math.max(1, fontSize * scale)
  const lineH = px * TEXT_LINE_HEIGHT
  const padX = px * 0.12
  const padY = (lineH - px) / 2

  const probe = document.createElement('canvas').getContext('2d')
  if (!probe) return null
  probe.font = `${px}px ${TEXT_FONT_STACK}`
  let textW = 0
  for (const l of lines) textW = Math.max(textW, probe.measureText(l).width)

  // 上限保护：超长文本不该生成超大位图（内存与 IPC 都要留余量）
  const cw = Math.min(8192, Math.max(1, Math.ceil(textW + padX * 2)))
  const ch = Math.min(8192, Math.max(1, Math.ceil(lineH * lines.length)))

  const canvas = document.createElement('canvas')
  canvas.width = cw
  canvas.height = ch
  const ctx = canvas.getContext('2d')
  if (!ctx) return null
  ctx.font = `${px}px ${TEXT_FONT_STACK}`
  ctx.textBaseline = 'top'
  ctx.fillStyle = rgba(color)
  lines.forEach((l, i) => {
    if (l) ctx.fillText(l, padX, i * lineH + padY)
  })

  const url = canvas.toDataURL('image/png')
  const png = url.slice(url.indexOf(',') + 1)
  if (!png) return null

  return {
    canvas,
    png,
    w: cw / scale,
    h: ch / scale,
    padX: padX / scale,
    padY: padY / scale,
  }
}

/**
 * 把编号光栅化成"实心圆 + 数字"的位图（**物理像素**）。
 *
 * 为什么同样走 UI 层位图：数字也用系统字体渲染（与文字标注同一套字体栈），
 * 核心层零依赖的约束不用为数字再开一次口子；且序列号与文字共用
 * `Shape::Bitmap` + `blit_bitmap` 通道，Rust 侧零改动。
 *
 * `anchor` 语义：调用方按**圆心**落点（本方返回的 `w`/`h` 供其对半回退）。
 */
export function rasterizeBadge(
  n: number,
  color: Rgba,
  fontSize: number,
  sf: number,
): TextRaster | null {
  const scale = sf > 0 ? sf : 1
  const px = Math.max(1, fontSize * scale)
  const label = String(n)

  const probe = document.createElement('canvas').getContext('2d')
  if (!probe) return null
  const fontSpec = `700 ${px}px ${TEXT_FONT_STACK}`
  probe.font = fontSpec
  const m = probe.measureText(label)
  // 用"墨迹外接框"而不是字宽：数字的字形左右边距不对称（"1"尤其明显），
  // 按字宽居中在圆里看起来会偏左。inkW 同时用于兜住两位数不溢出。
  const inkW = m.actualBoundingBoxLeft + m.actualBoundingBoxRight
  /** 墨迹中心相对绘制锚点的偏移（水平/垂直），用于光学居中 */
  const inkDx = (m.actualBoundingBoxRight - m.actualBoundingBoxLeft) / 2
  const inkDy = (m.actualBoundingBoxDescent - m.actualBoundingBoxAscent) / 2

  // 直径：单字用固定比例，多位数按墨迹宽度放大（否则"10""100"会贴到圆边）
  const d = Math.ceil(Math.max(px * BADGE_DIAMETER_RATIO, Math.max(m.width, inkW) * 1.5))
  const pad = Math.ceil(px * 0.12) // 抗锯齿余量，避免圆边被裁
  const cw = d + pad * 2

  const canvas = document.createElement('canvas')
  canvas.width = cw
  canvas.height = cw
  const ctx = canvas.getContext('2d')
  if (!ctx) return null

  const c = cw / 2
  ctx.fillStyle = rgba(color)
  ctx.beginPath()
  ctx.arc(c, c, d / 2, 0, Math.PI * 2)
  ctx.fill()

  // 数字颜色：深色底用白字；底色本身很亮（白/黄）时用深灰，
  // 否则白字压白底会"看不见编号"。阈值取人眼亮度 200。
  const lum = 0.299 * color[0] + 0.587 * color[1] + 0.114 * color[2]
  ctx.fillStyle = lum > 200 ? 'rgb(17,24,39)' : 'rgb(255,255,255)'
  ctx.font = fontSpec
  // 锚点用 left/alphabetic（可精确推算墨迹位置），再按墨迹中心回推，
  // 这样数字的视觉重心才真的落在圆心，而不是"字宽中心"落在圆心。
  ctx.textAlign = 'left'
  ctx.textBaseline = 'alphabetic'
  ctx.fillText(label, c - inkDx, c - inkDy)

  const url = canvas.toDataURL('image/png')
  const png = url.indexOf(',') >= 0 ? url.slice(url.indexOf(',') + 1) : ''
  if (!png) return null

  return { canvas, png, w: cw / scale, h: cw / scale, padX: 0, padY: 0 }
}

/**
 * 把贴纸字形（emoji）光栅化成位图（**物理像素**）。
 *
 * 与 `rasterizeText` 的区别：emoji 是彩色字形，**不能加粗**（彩色字体忽略
 * font-weight，反而可能 fallback 到单色字形），所以字体串里不带 700；
 * 也不用 measureText 的墨迹框（彩色 emoji 的 metrics 在部分平台是空的），
 * 直接按字号给一块正方形画布、`center/middle` 居中。
 *
 * `anchor` 语义同 `rasterizeBadge`：调用方按**中心**落点。
 */
export function rasterizeSticker(glyph: string, fontSize: number, sf: number): TextRaster | null {
  const scale = sf > 0 ? sf : 1
  // 贴纸比同字号文字大一圈才像"贴纸"（文字 16px 时贴纸约 32px）
  const px = Math.max(8, fontSize * scale * 2)
  const side = Math.ceil(px * 1.3)

  const canvas = document.createElement('canvas')
  canvas.width = side
  canvas.height = side
  const ctx = canvas.getContext('2d')
  if (!ctx) return null

  ctx.font = `${px}px ${TEXT_FONT_STACK}`
  ctx.textAlign = 'center'
  ctx.textBaseline = 'middle'
  // 轻微的白色描边：深色截图（终端/代码）上没有它，彩色 emoji 边缘会糊住
  ctx.lineWidth = Math.max(1, px * 0.04)
  ctx.strokeStyle = 'rgba(255,255,255,0.9)'
  ctx.strokeText(glyph, side / 2, side / 2)
  ctx.fillText(glyph, side / 2, side / 2)

  const url = canvas.toDataURL('image/png')
  const png = url.indexOf(',') >= 0 ? url.slice(url.indexOf(',') + 1) : ''
  if (!png) return null
  return { canvas, png, w: side / scale, h: side / scale, padX: 0, padY: 0 }
}

export function drawShape(
  ctx: CanvasRenderingContext2D,
  s: ShapeDraft,
  imageEl: HTMLImageElement | null,
  sf: number,
  bitmap?: ImageBitmap | null,
) {
  // 马赛克 / 位图 / 聚光灯 / 橡皮擦 / 水印 / 智能擦除没有描边色与线宽
  const hasStroke =
    s.type !== 'mosaic' &&
    s.type !== 'bitmap' &&
    s.type !== 'spotlight' &&
    s.type !== 'eraser' &&
    s.type !== 'watermark' &&
    s.type !== 'smartErase'
  if (hasStroke) {
    ctx.strokeStyle = rgba(s.color)
    ctx.fillStyle = rgba(s.color)
    ctx.lineWidth = s.width
  } else {
    ctx.lineWidth = 1
  }
  switch (s.type) {
    case 'bitmap':
      // 位图是物理像素尺寸，按逻辑尺寸画到 sf 缩放过的画布上 = 1:1 无重采样
      if (s.canvas) ctx.drawImage(s.canvas, s.x, s.y, s.w, s.h)
      break
    case 'rect':
      ctx.strokeRect(s.x, s.y, s.w, s.h)
      break
    case 'ellipse':
      ctx.beginPath()
      ctx.ellipse(
        s.x + s.w / 2,
        s.y + s.h / 2,
        s.w / 2,
        s.h / 2,
        0,
        0,
        Math.PI * 2,
      )
      ctx.stroke()
      break
    case 'arrow':
      drawArrow(ctx, s.x1, s.y1, s.x2, s.y2, s.width)
      break
    case 'pen':
    case 'highlighter':
      if (s.points.length === 1) {
        // 单点：圆点
        ctx.beginPath()
        ctx.arc(s.points[0][0], s.points[0][1], s.width / 2, 0, Math.PI * 2)
        ctx.fill()
      } else {
        ctx.beginPath()
        ctx.moveTo(s.points[0][0], s.points[0][1])
        for (const p of s.points.slice(1)) ctx.lineTo(p[0], p[1])
        ctx.stroke()
      }
      break
    case 'eraser': {
      // 预览用 destination-out 在标注画布上"打洞"，露出下方的冻结图
      // ——与 Rust「从 base 还原像素」观感等价：画布上本来就只有标注，
      // 洞外露出的就是原图。按 shapes 顺序绘制，天然获得
      // "擦掉之前的、保留之后的"语义（与 Rust 的按序光栅化一致）。
      ctx.save()
      ctx.globalCompositeOperation = 'destination-out'
      ctx.fillStyle = '#000'
      if (s.points.length === 1) {
        ctx.beginPath()
        ctx.arc(s.points[0][0], s.points[0][1], s.radius / 2, 0, Math.PI * 2)
        ctx.fill()
      } else {
        ctx.lineWidth = s.radius
        ctx.beginPath()
        ctx.moveTo(s.points[0][0], s.points[0][1])
        for (const p of s.points.slice(1)) ctx.lineTo(p[0], p[1])
        ctx.stroke()
      }
      ctx.restore()
      break
    }
    case 'zoom': {
      if (s.w <= 0 || s.h <= 0) break
      if (imageEl) {
        const iw = imageEl.naturalWidth
        const ih = imageEl.naturalHeight
        // 源矩形（物理像素）= 区域按 zoom 缩小，与 Rust 的最近邻映射同源
        let sx = Math.round(s.x * sf)
        let sy = Math.round(s.y * sf)
        let sw = Math.round(s.w * sf) / s.zoom
        let sh = Math.round(s.h * sf) / s.zoom
        // 区域伸出画布时按比例裁源矩形，不越界取样
        if (sx < 0) {
          sw += sx
          sx = 0
        }
        if (sy < 0) {
          sh += sy
          sy = 0
        }
        if (sx + sw > iw) sw = iw - sx
        if (sy + sh > ih) sh = ih - sy
        if (sw > 0 && sh > 0) {
          ctx.save()
          // 关掉平滑 = 最近邻，与 Rust 的 zoom_region 取样方式一致
          ctx.imageSmoothingEnabled = false
          ctx.drawImage(imageEl, sx, sy, sw, sh, s.x, s.y, s.w, s.h)
          ctx.restore()
        }
      }
      // 边框最后画，避免被放大内容盖住（与 Rust 一致）
      ctx.strokeRect(s.x, s.y, s.w, s.h)
      break
    }
    case 'smartErase':
      if (s.w > 0 && s.h > 0 && imageEl) drawSmartErasePreview(ctx, s, imageEl, sf)
      break
    case 'watermark':
      if (s.w <= 0 || s.h <= 0) break
      if (s.canvas) {
        // 单块位图按步进平铺（与 Rust watermark_region 同一网格）
        for (let ty = s.y; ty < s.y + s.h; ty += s.stepY) {
          for (let tx = s.x; tx < s.x + s.w; tx += s.stepX) {
            ctx.drawImage(s.canvas, tx, ty, s.tileW, s.tileH)
          }
        }
      } else {
        // 拖拽中（还没有文字）：虚线框标出将要平铺的区域
        ctx.save()
        ctx.setLineDash([4, 3])
        ctx.strokeStyle = 'rgba(120,120,120,0.9)'
        ctx.strokeRect(s.x, s.y, s.w, s.h)
        ctx.restore()
      }
      break
    case 'mosaic':
      drawMosaicPreview(ctx, s, bitmap ?? imageEl, sf)
      break
    case 'spotlight':
      drawSpotlightPreview(ctx, s, sf)
      break
  }
}

/** 圆角矩形路径（不用 Canvas 自带的 `roundRect`：WebView2 版本参差，
 *  且这里需要把半径 clamp 到短边一半，自己画更好控制）。 */
function roundRectPath(
  ctx: CanvasRenderingContext2D,
  x: number,
  y: number,
  w: number,
  h: number,
  r: number,
) {
  const rr = Math.max(0, Math.min(r, w / 2, h / 2))
  ctx.moveTo(x + rr, y)
  ctx.arcTo(x + w, y, x + w, y + h, rr)
  ctx.arcTo(x + w, y + h, x, y + h, rr)
  ctx.arcTo(x, y + h, x, y, rr)
  ctx.arcTo(x, y, x + w, y, rr)
  ctx.closePath()
}

/**
 * 聚光灯预览：整块压暗，再把圆角矩形"挖空"。
 *
 * 为什么必须走离屏画布：主画布上已经画好了其他标注，直接在主画布上
 * `destination-out` 挖洞会把它们一并擦掉。先在同尺寸离屏画布上完成
 * 「压暗 + 挖洞」，再整张贴回主画布，其他标注就不受影响。
 *
 * 遮罩是纯黑按 `dim` 做 alpha 填充，与 Rust 侧的乘性压暗在观感上等价。
 */
function drawSpotlightPreview(
  ctx: CanvasRenderingContext2D,
  s: Extract<ShapeDraft, { type: 'spotlight' }>,
  sf: number,
) {
  if (s.w <= 0 || s.h <= 0 || s.dim <= 0) return
  const W = ctx.canvas.width
  const H = ctx.canvas.height
  if (W <= 0 || H <= 0) return
  if (dimCanvas.width !== W || dimCanvas.height !== H) {
    dimCanvas.width = W
    dimCanvas.height = H
  }
  const g = dimCanvas.getContext('2d')
  if (!g) return

  g.setTransform(1, 0, 0, 1, 0, 0)
  g.clearRect(0, 0, W, H)
  g.setTransform(sf, 0, 0, sf, 0, 0)
  g.globalCompositeOperation = 'source-over'
  g.fillStyle = `rgba(0,0,0,${(s.dim / 255).toFixed(4)})`
  g.fillRect(0, 0, W / sf, H / sf)
  // 挖洞必须用**完全不透明**的填充色：destination-out 的擦除量等于源 alpha，
  // 若沿用 dim 的半透明黑，只会按比例擦掉一部分（dim=150 时洞内仍残留 41% 黑）。
  g.globalCompositeOperation = 'destination-out'
  g.fillStyle = '#000'
  g.beginPath()
  roundRectPath(g, s.x, s.y, s.w, s.h, s.radius)
  g.fill()
  g.globalCompositeOperation = 'source-over'

  ctx.save()
  ctx.setTransform(1, 0, 0, 1, 0, 0)
  ctx.drawImage(dimCanvas, 0, 0)
  ctx.restore()
}

function drawArrow(
  ctx: CanvasRenderingContext2D,
  x1: number,
  y1: number,
  x2: number,
  y2: number,
  width: number,
) {
  const dx = x2 - x1
  const dy = y2 - y1
  const len = Math.hypot(dx, dy)
  if (len < 0.5) return
  // 与 Rust 对齐：翼长 = max(8, width*4)，且不超过主干一半；夹角 25°
  const head = Math.min(Math.max(width * 4, 8), len * 0.5)
  const angle = (25 * Math.PI) / 180
  const ux = dx / len
  const uy = dy / len
  ctx.beginPath()
  ctx.moveTo(x1, y1)
  ctx.lineTo(x2, y2)
  ctx.stroke()
  for (const sign of [-1, 1]) {
    const a = angle * sign
    const cos = Math.cos(a)
    const sin = Math.sin(a)
    const wx = ux * cos - uy * sin
    const wy = ux * sin + uy * cos
    ctx.beginPath()
    ctx.moveTo(x2, y2)
    ctx.lineTo(x2 - wx * head, y2 - wy * head)
    ctx.stroke()
  }
}

/** 马赛克预览：**逐块求真平均**，与 core-algo::draw::mosaic_region 同算法。
 *
 *  早期版本用"把区域缩小到块大小（靠缩放插值近似平均）再关掉平滑放大"，
 *  看起来像块平均，实际上是错的：Chrome 的缩小过滤是**稀疏采样**，块越大
 *  越会漏掉笔画像素，结果整体偏白（黑字被跳过、只剩白底），与 Rust 输出的
 *  纯色块对不上。所以这里改成读像素、按块累加求均值、再逐块 fillRect。
 */
function drawMosaicPreview(
  ctx: CanvasRenderingContext2D,
  s: Extract<ShapeDraft, { type: 'mosaic' }>,
  /** ImageBitmap（合法可读）或 <img>（可能污染画布） */
  source: CanvasImageSource | null,
  sf: number,
) {
  if (!source || s.w <= 0 || s.h <= 0) return
  // 源坐标为物理像素（img 自然尺寸）
  const sx = Math.round(s.x * sf)
  const sy = Math.round(s.y * sf)
  const sw = Math.round(s.w * sf)
  const sh = Math.round(s.h * sf)
  if (sw < 1 || sh < 1) return

  mosaicSrc.width = sw
  mosaicSrc.height = sh
  const g = mosaicSrc.getContext('2d', { willReadFrequently: true })
  if (!g) return
  // 1:1 抠出原图区域，不能缩放（缩放会插值，就不是原像素了）
  g.imageSmoothingEnabled = false
  g.clearRect(0, 0, sw, sh)
  g.drawImage(source, sx, sy, sw, sh, 0, 0, sw, sh)
  let src: Uint8ClampedArray
  try {
    src = g.getImageData(0, 0, sw, sh).data
  } catch {
    // 画布被污染（拿到了 <img> 而不是 ImageBitmap）：不画预览，但**绝不抛** ——
    // 这个异常发生在渲染路径里，抛出去会把 React 根节点整个卸载掉
    // （实测：一拖马赛克，浮层直接消失）。
    return
  }

  // 块边长为物理像素（与截图 DPI 无关，和 Rust 侧换算保持一致）
  const block = Math.max(2, Math.round(s.block * sf))
  for (let by = 0; by < sh; by += block) {
    const bh = Math.min(block, sh - by)
    for (let bx = 0; bx < sw; bx += block) {
      const bw = Math.min(block, sw - bx)
      let r = 0
      let gg = 0
      let b = 0
      let n = 0
      for (let y = by; y < by + bh; y += 1) {
        let i = (y * sw + bx) * 4
        for (let x = 0; x < bw; x += 1) {
          // 跳过透明像素，避免把均值拉黑（与 Rust 一致）
          if (src[i + 3] !== 0) {
            r += src[i]
            gg += src[i + 1]
            b += src[i + 2]
            n += 1
          }
          i += 4
        }
      }
      if (n === 0) continue
      g.fillStyle = `rgb(${Math.round(r / n)},${Math.round(gg / n)},${Math.round(b / n)})`
      g.fillRect(bx, by, bw, bh)
    }
  }

  // 贴回主画布：物理尺寸 → 逻辑尺寸，关掉平滑保证块是纯色硬边
  ctx.imageSmoothingEnabled = false
  ctx.drawImage(mosaicSrc, 0, 0, sw, sh, s.x, s.y, s.w, s.h)
  ctx.imageSmoothingEnabled = true
}

/**
 * 智能擦除预览：**每行水平线性渐变**近似。
 *
 * 取区域左右边界外 1 列的原图像素，逐行在两者之间插值。这个近似对
 * ① 纯色背景（精确）② 水平/垂直/对角等一切"对 x 线性"的图像（精确）
 * 都是准的——日常要擦掉的文字/图标/水印几乎都长在这类背景上。
 * 只有真正二维纹理（照片、棋盘格）才会与 Rust 的波前扩散结果有偏差，
 * 而那种情况下预览本来也只能表达"这一片会被抹掉"。
 */
function drawSmartErasePreview(
  ctx: CanvasRenderingContext2D,
  s: Extract<ShapeDraft, { type: 'smartErase' }>,
  imageEl: HTMLImageElement,
  sf: number,
) {
  const iw = imageEl.naturalWidth
  const ih = imageEl.naturalHeight
  if (iw <= 0 || ih <= 0) return
  // 区域（物理像素）与图求交
  const rx = Math.max(0, Math.round(s.x * sf))
  const ry = Math.max(0, Math.round(s.y * sf))
  const rw = Math.min(iw, Math.round((s.x + s.w) * sf)) - rx
  const rh = Math.min(ih, Math.round((s.y + s.h) * sf)) - ry
  if (rw <= 0 || rh <= 0) return
  // 左右边界列：越界时钳回区域自身（Rust 侧则是该侧不参与扩散）
  const lx = Math.max(0, rx - 1)
  const rxp = Math.min(iw - 1, rx + rw)
  if (stripCanvas.height !== rh) stripCanvas.height = rh
  stripCanvas.width = 2
  const g = stripCanvas.getContext('2d', { willReadFrequently: true })
  if (!g) return
  g.clearRect(0, 0, 2, rh)
  g.drawImage(imageEl, lx, ry, 1, rh, 0, 0, 1, rh)
  g.drawImage(imageEl, rxp, ry, 1, rh, 1, 0, 1, rh)
  const src = g.getImageData(0, 0, 2, rh).data
  // 逐行水平渐变；行高留一点重叠，避免取整误差留下发丝缝
  const rowH = 1 / sf + 0.05
  for (let row = 0; row < rh; row += 1) {
    const li = row * 8
    const ri = row * 8 + 4
    const grad = ctx.createLinearGradient(s.x, 0, s.x + s.w, 0)
    grad.addColorStop(0, `rgb(${src[li]},${src[li + 1]},${src[li + 2]})`)
    grad.addColorStop(1, `rgb(${src[ri]},${src[ri + 1]},${src[ri + 2]})`)
    ctx.fillStyle = grad
    ctx.fillRect(s.x, s.y + row / sf, s.w, rowH)
  }
}
