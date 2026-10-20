import { useEffect, useRef, type PointerEvent as ReactPointerEvent } from 'react'
import {
  HIGHLIGHTER_COLOR,
  toolMeta,
  type AnnotateTool,
  type Rgba,
  type ShapeDraft,
} from './tools'

/** 模块级复用的马赛克缩略画布（避免每次渲染新建）。 */
const mosaicSmall = document.createElement('canvas')
const mosaicSmallCtx = mosaicSmall.getContext('2d')

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
  shapes: ShapeDraft[]
  onShapesChange: (shapes: ShapeDraft[]) => void
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
    drawingRef.current = true
    const p = localPoint(e)
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
    <canvas
      ref={canvasRef}
      className={`annotate-canvas ${tool ? '' : 'passthrough'}`}
      width={naturalWidth}
      height={naturalHeight}
      style={{ width: '100%', height: '100%' }}
      onPointerDown={onPointerDown}
      onPointerMove={onPointerMove}
      onPointerUp={onPointerUp}
    />
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

function drawShape(
  ctx: CanvasRenderingContext2D,
  s: ShapeDraft,
  imageEl: HTMLImageElement | null,
  sf: number,
) {
  // 马赛克没有描边色，只有几何参数
  if (s.type !== 'mosaic') {
    ctx.strokeStyle = rgba(s.color)
    ctx.fillStyle = rgba(s.color)
  }
  ctx.lineWidth = s.type === 'mosaic' ? 1 : s.width
  switch (s.type) {
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
  }
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
