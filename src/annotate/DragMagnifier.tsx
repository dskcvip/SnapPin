/**
 * 选区阶段的放大镜（拖选 / 悬停时跟随光标）。
 *
 * 需求来源（用户反馈 + 参考竞品）："开始截图"那一步就应能看到放大镜，
 * 方便选取内容比较小、不易分辨的地方；同时这一步直接显示取色信息，
 * 不用再多一步截图操作。
 *
 * 实现要点：
 * - 画面来自**同一份位图数据**：优先用父组件传入的 ImageBitmap（由 asset
 *   图片 fetch 成 Blob 再解码而来——canvas 读取这种来源**不会**被浏览器
 *   判为跨域污染，而直接用 `http://asset.localhost` 的 <img> 会）；
 *   位图还没就绪时回退 <img>（仅绘制不读取时两者等价）。
 * - 源区域 = MAG_SIZE / MAG_ZOOM **逻辑像素**（DPI 无关：用户要放大的是
 *   "内容大小"，逻辑像素才是内容的度量）；
 * - 取样区域要能装下**一整个字**：常见正文 12-14px，取 39 逻辑像素
 *   （≈2.5 个字宽）。早期用 140/8=17.5px + 关闭平滑，结果只顾着"像素
 *   块好看"，用户实际看到的是半个字的马赛克、字都认不出（2026-10-21 反馈）；
 * - 放大**开启平滑**：4x 最近邻会把抗锯齿笔画放大成方块，字形反而更难认。
 *   像素级的精确对位由中心十字 + 底部 HEX 读数保证，不依赖方块边界。
 * - 光标贴边时源区域做边界收拢（不允许越界），同时气泡整体翻到光标
 *   另一侧，避免被屏幕裁掉；
 * - 颜色由父组件采样后传入（与取色器共用一条取样链路），本组件只展示。
 */
import { useEffect, useRef } from 'react'

/** 气泡可视区边长（CSS px）。放大镜要看清小字，略大于参考图的 120-150。 */
const MAG_SIZE = 156
/** 放大倍率：可视区 / 源区域（逻辑像素）。4x → 源区域 39 逻辑像素。 */
const MAG_ZOOM = 4
/** 底部色条高度（CSS px）。 */
const MAG_FOOT = 26
/** 气泡与光标的间距（CSS px）。 */
const MAG_GAP = 16

export interface DragMagnifierProps {
  /** 光标位置（浮层窗口坐标 = 显示器逻辑像素） */
  cursor: { x: number; y: number }
  /** 冻结图元素（位图未就绪时的回退绘制源；不参与像素读取） */
  imageEl: HTMLImageElement | null
  /** 冻结图的 ImageBitmap（fetch→Blob→解码；canvas 读取无跨域污染） */
  bitmap: ImageBitmap | null
  /** 显示器缩放因子（逻辑 → 物理像素） */
  scaleFactor: number
  /** 光标处颜色 HEX（null = 取样失败） */
  hex: string | null
  /** 刚刚复制成功的 HEX（null = 无提示） */
  copiedHex: string | null
}

export function DragMagnifier({
  cursor,
  imageEl,
  bitmap,
  scaleFactor,
  hex,
  copiedHex,
}: DragMagnifierProps) {
  const canvasRef = useRef<HTMLCanvasElement | null>(null)
  const source = bitmap ?? imageEl

  // 光标每动一帧重绘一次：drawImage 一次，成本可忽略
  useEffect(() => {
    const c = canvasRef.current
    if (!c || !source) return
    const nw = 'naturalWidth' in source ? source.naturalWidth : source.width
    const nh = 'naturalHeight' in source ? source.naturalHeight : source.height
    if (nw === 0 || nh === 0) return
    // 源区域（物理像素）：光标周围 MAG_SIZE/MAG_ZOOM 逻辑像素
    const sw = (MAG_SIZE / MAG_ZOOM) * scaleFactor
    if (sw > nw || sw > nh) return
    let sx = cursor.x * scaleFactor - sw / 2
    let sy = cursor.y * scaleFactor - sw / 2
    // 边界收拢：贴边时显示贴边内容，而不是留黑边
    sx = Math.min(Math.max(sx, 0), nw - sw)
    sy = Math.min(Math.max(sy, 0), nh - sw)
    //  backing store 按 DPR 放大，保证放大后的像素边缘不发虚
    const dpr = window.devicePixelRatio || scaleFactor || 1
    const size = Math.round(MAG_SIZE * dpr)
    if (c.width !== size || c.height !== size) {
      c.width = size
      c.height = size
    }
    const g = c.getContext('2d')
    if (!g) return
    // 平滑放大：文字笔画保持连续，小号字也能读出字形
    g.imageSmoothingEnabled = true
    g.imageSmoothingQuality = 'high'
    g.clearRect(0, 0, c.width, c.height)
    g.drawImage(source, sx, sy, sw, sw, 0, 0, c.width, c.height)
  }, [cursor, source, scaleFactor])

  // 位置：默认在光标右下；贴近右/下边缘时翻到另一侧
  const flipX = cursor.x + MAG_GAP + MAG_SIZE + 2 > window.innerWidth
  const flipY = cursor.y + MAG_GAP + MAG_SIZE + MAG_FOOT + 2 > window.innerHeight
  const left = flipX ? cursor.x - MAG_GAP - MAG_SIZE - 2 : cursor.x + MAG_GAP
  const top = flipY ? cursor.y - MAG_GAP - MAG_SIZE - MAG_FOOT - 2 : cursor.y + MAG_GAP

  return (
    <>
      <div className="drag-mag" style={{ left, top }}>
        <div className="drag-mag-view">
          <canvas ref={canvasRef} style={{ width: MAG_SIZE, height: MAG_SIZE }} />
        </div>
        <div className="drag-mag-foot">
          {hex ? (
            <>
              <i className="pick-swatch" style={{ background: hex }} />
              <span className="pick-hex">{hex}</span>
            </>
          ) : (
            <span className="drag-mag-tip">按 C 复制颜色</span>
          )}
        </div>
      </div>
      {copiedHex && (
        <div
          className="drag-mag-copied"
          style={{ left, top: flipY ? top - 28 : top + MAG_SIZE + MAG_FOOT + 6 }}
        >
          已复制 {copiedHex}
        </div>
      )}
    </>
  )
}
