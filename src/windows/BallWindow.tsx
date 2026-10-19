import { useEffect, useRef, useState, type CSSProperties, type PointerEvent as ReactPointerEvent } from 'react'
import { ballCommand, dragWindow, getConfig, type AppConfig } from '../lib/api'
import '../styles/ball.css'

const DEFAULT_COLOR = '#12A5D8'

/** 把 #rrggbb 调亮/调暗 amount（-255..255）。 */
function shade(hex: string, amount: number): string {
  const raw = hex.replace('#', '')
  const full =
    raw.length === 3
      ? raw
          .split('')
          .map((c) => c + c)
          .join('')
      : raw
  const num = parseInt(full, 16)
  if (Number.isNaN(num)) return DEFAULT_COLOR
  const clamp = (v: number) => Math.max(0, Math.min(255, v))
  const r = clamp((num >> 16) + amount)
  const g = clamp(((num >> 8) & 0xff) + amount)
  const b = clamp((num & 0xff) + amount)
  return `#${((r << 16) | (g << 8) | b).toString(16).padStart(6, '0')}`
}

function PinGlyph() {
  return (
    <svg viewBox="0 0 24 24" fill="none" aria-hidden>
      <path
        d="M12 2.5c-3.6 0-6.5 2.9-6.5 6.5 0 4.8 6.5 12.5 6.5 12.5s6.5-7.7 6.5-12.5c0-3.6-2.9-6.5-6.5-6.5z"
        fill="#ffffff"
      />
      <circle cx="12" cy="9" r="2.4" fill="var(--ball-color, #12A5D8)" />
    </svg>
  )
}

/**
 * 悬浮球窗口。
 * 交互：单击截图；双击截图；拖动移动（>4px 后交给 OS 原生拖拽循环）。
 * 说明：右键菜单暂由托盘菜单承担（见 ROADMAP M3）。
 */
export function BallWindow() {
  const [cfg, setCfg] = useState<AppConfig | null>(null)
  const dragRef = useRef<{ x: number; y: number; dragging: boolean } | null>(null)

  useEffect(() => {
    getConfig()
      .then(setCfg)
      .catch(() => setCfg(null))
  }, [])

  const color = cfg?.themeColor ?? DEFAULT_COLOR
  const size = cfg?.ballSize ?? 56

  const onPointerDown = (e: ReactPointerEvent<HTMLDivElement>) => {
    if (e.button !== 0) return
    dragRef.current = { x: e.clientX, y: e.clientY, dragging: false }
  }

  const onPointerMove = (e: ReactPointerEvent<HTMLDivElement>) => {
    const d = dragRef.current
    if (!d || d.dragging) return
    if (Math.abs(e.clientX - d.x) > 4 || Math.abs(e.clientY - d.y) > 4) {
      d.dragging = true
      dragWindow().catch(() => {})
    }
  }

  const onPointerUp = () => {
    const d = dragRef.current
    dragRef.current = null
    if (d && !d.dragging) {
      ballCommand('capture').catch(() => {})
    }
  }

  return (
    <div className="ball-root" style={{ width: size, height: size }}>
      <div
        className="ball"
        title="单击截图 · 拖动移动"
        style={
          {
            '--ball-color': color,
            '--ball-light': shade(color, 55),
          } as CSSProperties
        }
        onPointerDown={onPointerDown}
        onPointerMove={onPointerMove}
        onPointerUp={onPointerUp}
        onDoubleClick={() => ballCommand('capture').catch(() => {})}
      >
        <PinGlyph />
      </div>
    </div>
  )
}
