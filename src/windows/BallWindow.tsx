import { useEffect, useRef, useState, type CSSProperties, type PointerEvent as ReactPointerEvent, type MouseEvent as ReactMouseEvent } from 'react'
import { ballCommand, ballMenu, dragWindow, getConfig, uiLog, type AppConfig } from '../lib/api'
import '../styles/ball.css'

const DEFAULT_COLOR = '#12A5D8'

/** 品牌 S 的中心线（与 scripts/make_icons.py 同一套 4 段三次贝塞尔）。 */
const S_PATH =
  'M700 300 C560 210 420 215 375 330 C340 430 480 470 600 500 C680 520 690 590 648 655 C600 730 470 745 360 700'

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

function SGlyph() {
  return (
    <svg viewBox="0 0 1024 1024" aria-hidden>
      <defs>
        <linearGradient id="snS" x1="0" y1="0" x2="0" y2="1">
          <stop offset="0%" stopColor="#FFD9B5" />
          <stop offset="46%" stopColor="#FF9E63" />
          <stop offset="100%" stopColor="#F5772A" />
        </linearGradient>
      </defs>
      <g fill="none" strokeLinecap="round" strokeLinejoin="round">
        {/* 投影 */}
        <path d={S_PATH} transform="translate(0 26)" stroke="rgba(0,0,0,.55)" strokeWidth="112" />
        {/* 下缘暗边：制造体积感 */}
        <path d={S_PATH} transform="translate(0 12)" stroke="#B4420F" strokeWidth="108" />
        {/* 主体：品牌橙垂直渐变 */}
        <path d={S_PATH} stroke="url(#snS)" strokeWidth="104" />
        {/* 顶部细高光 */}
        <path d={S_PATH} transform="translate(0 -14)" stroke="rgba(255,255,255,.26)" strokeWidth="30" />
      </g>
    </svg>
  )
}

/**
 * 悬浮球窗口。
 * 交互：单击截图；双击截图；拖动移动（>4px 后交给 OS 原生拖拽循环）；
 * 右键弹出原生菜单（截图/剪贴板贴图/隐藏悬浮球/退出）。
 * 说明：WebView2 默认浏览器右键菜单（含"打印"）已被初始化脚本抑制，
 * 右键由 ballMenu 命令弹出应用原生菜单。
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

  // 右键：抑制默认菜单（双保险，初始化脚本已在 window 层抑制），
  // 弹出 Rust 原生菜单。事件挂在 ball-root 上：窗口留白处右键同样有效。
  const onContextMenu = (e: ReactMouseEvent<HTMLDivElement>) => {
    e.preventDefault()
    uiLog('ball contextmenu -> ballMenu')
    ballMenu().catch(() => {})
  }

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
      uiLog('ball pointerup -> capture')
      ballCommand('capture').catch((e) => uiLog(`ball capture error: ${String(e)}`))
    }
  }

  return (
    <div className="ball-root" onContextMenu={onContextMenu}>
      <div
        className="ball"
        title="单击截图 · 右键菜单 · 拖动移动"
        style={
          {
            '--ball-color': color,
            '--ball-light': shade(color, 55),
            '--ball-size': `${size}px`,
          } as CSSProperties
        }
        onPointerDown={onPointerDown}
        onPointerMove={onPointerMove}
        onPointerUp={onPointerUp}
        onDoubleClick={() => {
          uiLog('ball dblclick -> capture')
          ballCommand('capture').catch((e) => uiLog(`ball dblclick capture error: ${String(e)}`))
        }}
      >
        <SGlyph />
      </div>
    </div>
  )
}
