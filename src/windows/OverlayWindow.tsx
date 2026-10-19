import { useEffect, useRef, useState, type CSSProperties, type PointerEvent as ReactPointerEvent } from 'react'
import {
  cancelCapture,
  confirmSelection,
  imgSrc,
  overlayContext,
  type OverlayContext,
  type Rect,
  type ShotAction,
} from '../lib/api'
import '../styles/overlay.css'

const MIN_SIZE = 3

/**
 * 截图选区浮层窗口（每个显示器一个）。
 * 流程：Rust 截取全部显示器 → 打开等尺寸透明浮层 → 前端展示冻结图 →
 * 用户拖拽选区 → 确认动作（复制/贴图/保存）→ Rust 裁剪并执行，随后关闭全部浮层。
 */
export function OverlayWindow() {
  const [ctx, setCtx] = useState<OverlayContext | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [rect, setRect] = useState<Rect | null>(null)
  const [draft, setDraft] = useState<Rect | null>(null)
  const [cursor, setCursor] = useState<{ x: number; y: number } | null>(null)
  const [busy, setBusy] = useState(false)
  const dragRef = useRef<{ x: number; y: number } | null>(null)

  useEffect(() => {
    overlayContext()
      .then(setCtx)
      .catch((e) => setError(String(e)))
  }, [])

  // 键盘：Esc 取消，Enter 复制
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        e.preventDefault()
        cancelCapture().catch(() => {})
      } else if (e.key === 'Enter' && rect && !busy) {
        e.preventDefault()
        void act('copy')
      }
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  })

  const act = async (action: ShotAction) => {
    if (!ctx || !rect || busy) return
    setBusy(true)
    try {
      await confirmSelection(ctx.monitorId, rect, action)
      // 成功时 Rust 端关闭全部浮层；兜底：
      cancelCapture().catch(() => {})
    } catch (e) {
      console.error('confirm selection failed', e)
      setBusy(false)
    }
  }

  const onPointerDown = (e: ReactPointerEvent<HTMLDivElement>) => {
    if (e.button !== 0 || !ctx) return
    dragRef.current = { x: e.clientX, y: e.clientY }
    setRect(null)
    setDraft({ x: e.clientX, y: e.clientY, w: 0, h: 0 })
  }

  const onPointerMove = (e: ReactPointerEvent<HTMLDivElement>) => {
    const d = dragRef.current
    if (!d) return
    setDraft({
      x: Math.min(d.x, e.clientX),
      y: Math.min(d.y, e.clientY),
      w: Math.abs(e.clientX - d.x),
      h: Math.abs(e.clientY - d.y),
    })
    setCursor({ x: e.clientX, y: e.clientY })
  }

  const onPointerUp = () => {
    dragRef.current = null
    setCursor(null)
    setDraft((d) => {
      if (d && d.w >= MIN_SIZE && d.h >= MIN_SIZE) setRect(d)
      return null
    })
  }

  if (error) {
    return <div className="overlay-error">{error}</div>
  }
  if (!ctx) {
    return <div className="overlay-loading" />
  }

  const active = draft ?? rect
  const sf = ctx.scaleFactor
  const style = {
    width: ctx.naturalWidth,
    height: ctx.naturalHeight,
    transform: sf !== 1 ? `scale(${1 / sf})` : undefined,
    transformOrigin: 'top left',
  } as CSSProperties

  // 工具栏默认挂在选区下方；靠近屏幕底边时翻到上方
  const viewportH = ctx.naturalHeight / sf
  const toolbarAbove = rect ? rect.y + rect.h + 56 > viewportH : false

  return (
    <div
      className="overlay-root"
      onPointerDown={onPointerDown}
      onPointerMove={onPointerMove}
      onPointerUp={onPointerUp}
      onContextMenu={(e) => {
        e.preventDefault()
        if (rect) setRect(null)
        else cancelCapture().catch(() => {})
      }}
    >
      <img className="frozen" src={imgSrc(ctx.imagePath)} style={style} draggable={false} alt="" />

      {cursor && draft && (
        <>
          <div className="crosshair-v" style={{ left: cursor.x }} />
          <div className="crosshair-h" style={{ top: cursor.y }} />
        </>
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
              <i className="handle tl" />
              <i className="handle tr" />
              <i className="handle bl" />
              <i className="handle br" />
              <div
                className={`toolbar ${toolbarAbove ? 'above' : 'below'}`}
                onPointerDown={(e) => e.stopPropagation()}
              >
                <span className="toolbar-hint">
                  {Math.round(rect.w * sf)} × {Math.round(rect.h * sf)}
                </span>
                <button className="tbtn primary" disabled={busy} onClick={() => void act('copy')}>
                  复制 <kbd>Enter</kbd>
                </button>
                <button className="tbtn" disabled={busy} onClick={() => void act('pin')}>
                  贴图
                </button>
                <button className="tbtn" disabled={busy} onClick={() => void act('save')}>
                  保存
                </button>
                <button
                  className="tbtn ghost"
                  onClick={() => cancelCapture().catch(() => {})}
                >
                  取消 <kbd>Esc</kbd>
                </button>
              </div>
            </>
          )}
        </div>
      )}
    </div>
  )
}
