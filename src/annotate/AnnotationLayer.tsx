import { useEffect, useRef, useState, type PointerEvent as ReactPointerEvent } from 'react'
import {
  BADGE_DIAMETER_RATIO,
  HIGHLIGHTER_COLOR,
  SPOTLIGHT_RADIUS,
  TEXT_FONT_STACK,
  TEXT_LINE_HEIGHT,
  nextBadgeNo,
  toolMeta,
  type AnnotateTool,
  type Rgba,
  type ShapeDraft,
} from './tools'

/** 模块级复用的马赛克缩略画布（避免每次渲染新建）。 */
const mosaicSmall = document.createElement('canvas')
const mosaicSmallCtx = mosaicSmall.getContext('2d')

/** 模块级复用的聚光灯遮罩画布（尺寸随主画布，见 drawSpotlightPreview）。 */
const dimCanvas = document.createElement('canvas')

/** 拖拽成形的最小尺寸（逻辑像素），小于此视为单击忽略。 */
const MIN_DRAG = 3

interface Props {
  /** 冻结图物理像素尺寸 */
  naturalWidth: number
  naturalHeight: number
  /** DPI 缩放因子（物理/逻辑） */
  scaleFactor: number
  /** 冻结图 <img> 元素（马赛克预览的像素来源） */
  imageEl: HTMLImageElement | null
  /** null = 未选工具（选择态）：画布不拦截鼠标，可在选区内重新框选 */
  tool: AnnotateTool | null
  color: Rgba
  width: number
  mosaicBlock: number
  /** 文字字号（逻辑像素） */
  fontSize: number
  /** 聚光灯压暗强度（0-255） */
  spotlightDim: number
  shapes: ShapeDraft[]
  onShapesChange: (shapes: ShapeDraft[]) => void
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
    tool,
    color,
    width,
    mosaicBlock,
    fontSize,
    spotlightDim,
    shapes,
    onShapesChange,
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
  const [editing, setEditingState] = useState<TextEdit | null>(null)
  const editSeq = useRef(0)
  const taRef = useRef<HTMLTextAreaElement | null>(null)
  const setEditing = (v: TextEdit | null) => {
    editingRef.current = v
    setEditingState(v)
  }

  /** 在 (x, y) 开一个新的文字输入会话（逻辑像素） */
  const startText = (x: number, y: number) => {
    editSeq.current += 1
    setEditing({ id: editSeq.current, x, y, value: '', color, fontSize })
  }

  /** 提交文字：光栅化成位图图形（空内容直接丢弃）。 */
  const commitText = (sessionId?: number) => {
    const ed = editingRef.current
    if (!ed) return
    // 旧输入框迟到的 blur（新会话已开始）不处理
    if (sessionId !== undefined && ed.id !== sessionId) return
    setEditing(null)
    const r = rasterizeText(ed.value, ed.color, ed.fontSize, props.scaleFactor)
    if (!r) return
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
    for (const s of shapes) drawShape(ctx, s, imageEl, scaleFactor)
    if (draftRef.current) drawShape(ctx, draftRef.current, imageEl, scaleFactor)
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
    // 文字：单击落点开输入框（不是拖拽成形）。
    // 已有输入框时先提交，并**延后一帧再开新框**——鼠标按下会让旧输入框失焦，
    // 若同帧内就把新会话塞进去，那条迟到的 blur 会把刚开的新输入框直接提交掉
    // （表现为"点第二下打不了字"）。先提交 → 引用置空 → blur 空转 → 再开新框。
    if (tool === 'text') {
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
    if (meta.trajectory) {
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
    const d = draftRef.current
    if (!d) return
    if (d.type === 'pen' || d.type === 'highlighter') {
      const last = d.points[d.points.length - 1]
      // 去抖：位移过小不记录（减小数据量与 IPC 体积）
      if (Math.abs(p.x - last[0]) < 0.75 && Math.abs(p.y - last[1]) < 0.75) return
      d.points.push([round2(p.x), round2(p.y)])
      redraw(props, draftRef.current, canvasRef.current, imageEl)
      return
    }
    // 拖拽成形的四种（rect/ellipse/arrow/mosaic）
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
    redraw(props, d, canvasRef.current, imageEl)
  }

  const onPointerUp = (e: ReactPointerEvent<HTMLCanvasElement>) => {
    if (!drawingRef.current) return
    e.stopPropagation()
    drawingRef.current = false
    startRef.current = null
    const d = draftRef.current
    draftRef.current = null
    if (!d) return
    if (d.type === 'pen' || d.type === 'highlighter') {
      if (d.points.length >= 2) onShapesChange([...shapes, d])
    } else if (d.type === 'arrow') {
      // 箭头用两端点距离判定，太短视为误触
      if (Math.hypot(d.x2 - d.x1, d.y2 - d.y1) >= MIN_DRAG) onShapesChange([...shapes, d])
    } else if (d.w >= MIN_DRAG && d.h >= MIN_DRAG) {
      onShapesChange([...shapes, d])
    }
    redraw(props, null, canvasRef.current, imageEl)
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
              // 只关输入框，不退出截图
              e.preventDefault()
              e.stopPropagation()
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

/** 全量重绘（草稿 pass=null 表示只画已确认 shapes）。 */
function redraw(
  props: Props,
  draft: ShapeDraft | null,
  canvas: HTMLCanvasElement | null,
  imageEl: HTMLImageElement | null,
) {
  if (!canvas) return
  const ctx = canvas.getContext('2d')
  if (!ctx) return
  ctx.setTransform(1, 0, 0, 1, 0, 0)
  ctx.clearRect(0, 0, canvas.width, canvas.height)
  ctx.setTransform(props.scaleFactor, 0, 0, props.scaleFactor, 0, 0)
  ctx.lineCap = 'round'
  ctx.lineJoin = 'round'
  for (const s of props.shapes) drawShape(ctx, s, imageEl, props.scaleFactor)
  if (draft) drawShape(ctx, draft, imageEl, props.scaleFactor)
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

export function drawShape(
  ctx: CanvasRenderingContext2D,
  s: ShapeDraft,
  imageEl: HTMLImageElement | null,
  sf: number,
) {
  // 马赛克 / 位图 / 聚光灯没有描边色与线宽，只有几何参数
  const hasStroke = s.type !== 'mosaic' && s.type !== 'bitmap' && s.type !== 'spotlight'
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
    case 'mosaic':
      drawMosaicPreview(ctx, s, imageEl, sf)
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

/** 马赛克预览：先把目标区域缩成 block 大小缩略图（缩小=平均），
 *  再关闭平滑放大回去（放大=块状）。与 Rust 块平均算法同效果。 */
function drawMosaicPreview(
  ctx: CanvasRenderingContext2D,
  s: Extract<ShapeDraft, { type: 'mosaic' }>,
  imageEl: HTMLImageElement | null,
  sf: number,
) {
  if (!imageEl || s.w <= 0 || s.h <= 0) return
  // 源坐标为物理像素（img 自然尺寸）
  const sx = s.x * sf
  const sy = s.y * sf
  const sw = s.w * sf
  const sh = s.h * sf
  if (sw < 1 || sh < 1) return
  const bw = Math.max(1, Math.ceil(sw / Math.max(1, s.block * sf)))
  const bh = Math.max(1, Math.ceil(sh / Math.max(1, s.block * sf)))
  mosaicSmall.width = bw
  mosaicSmall.height = bh
  if (!mosaicSmallCtx) return
  mosaicSmallCtx.imageSmoothingEnabled = true
  mosaicSmallCtx.clearRect(0, 0, bw, bh)
  mosaicSmallCtx.drawImage(imageEl, sx, sy, sw, sh, 0, 0, bw, bh)
  ctx.imageSmoothingEnabled = false
  ctx.drawImage(mosaicSmall, 0, 0, bw, bh, s.x, s.y, s.w, s.h)
  ctx.imageSmoothingEnabled = true
}
