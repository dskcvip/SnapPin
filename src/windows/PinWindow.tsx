import { useEffect, useState, type PointerEvent as ReactPointerEvent } from 'react'
import {
  dragWindow,
  imgSrc,
  pinClose,
  pinContext,
  pinCopy,
  pinSave,
  type PinContext,
} from '../lib/api'
import '../styles/pin.css'

/**
 * 贴图窗口：无边框置顶透明窗，展示一张截图。
 * 交互：拖动移动；Ctrl+滚轮缩放；Esc 关闭；悬停显示工具条。
 * 上下文（图片路径/显示尺寸）通过 pin_context 命令按窗口标签取回。
 */
export function PinWindow() {
  const [ctx, setCtx] = useState<PinContext | null>(null)
  const [opacity, setOpacity] = useState(1)
  const [scale, setScale] = useState(1)

  useEffect(() => {
    pinContext()
      .then(setCtx)
      .catch(() => setCtx(null))
  }, [])

  useEffect(() => {
    const onWheel = (e: WheelEvent) => {
      if (!e.ctrlKey) return
      e.preventDefault()
      setScale((s) => Math.min(8, Math.max(0.1, s * (e.deltaY < 0 ? 1.1 : 0.9))))
    }
    window.addEventListener('wheel', onWheel, { passive: false })
    return () => window.removeEventListener('wheel', onWheel)
  }, [])

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape' && ctx) {
        e.preventDefault()
        pinClose(ctx.label).catch(() => {})
      }
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [ctx])

  if (!ctx) return <div className="pin-root" />

  const onDragStart = (e: ReactPointerEvent<HTMLImageElement>) => {
    if (e.button === 0) dragWindow().catch(() => {})
  }

  return (
    <div className="pin-root" style={{ width: ctx.width, height: ctx.height }}>
      <img
        className="pin-img"
        src={imgSrc(ctx.src)}
        style={{ opacity, transform: `scale(${scale})` }}
        onPointerDown={onDragStart}
        draggable={false}
        alt=""
      />
      <div className="pin-toolbar" onPointerDown={(e) => e.stopPropagation()}>
        <button onClick={() => pinCopy(ctx.label).catch(() => {})}>复制</button>
        <button onClick={() => pinSave(ctx.label).catch(() => {})}>保存</button>
        <label className="pin-opacity">
          <span>透明度</span>
          <input
            type="range"
            min={0.15}
            max={1}
            step={0.05}
            value={opacity}
            onChange={(e) => setOpacity(Number(e.target.value))}
          />
        </label>
        <button className="danger" onClick={() => pinClose(ctx.label).catch(() => {})}>
          关闭
        </button>
      </div>
    </div>
  )
}
