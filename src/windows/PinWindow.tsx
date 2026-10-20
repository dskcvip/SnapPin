import { useEffect, useRef, useState, type PointerEvent as ReactPointerEvent } from 'react'
import {
  dragWindow,
  imgSrc,
  pinClose,
  pinContext,
  pinCopy,
  pinSave,
  pinSetIgnoreCursorEvents,
  pinSetTopmost,
  type PinContext,
} from '../lib/api'
import '../styles/pin.css'

/** 鼠标穿透自动恢复时长（ms）。
 *  穿透 = 窗口对鼠标透明（工具条也点不到），无法从窗内主动退出，
 *  故用定时恢复；需要长时间穿透的场景用"持续锁定"以外的临时查看。 */
const PASS_THROUGH_TTL_MS = 5000

/**
 * 贴图窗口：无边框置顶透明窗，展示一张截图。
 * 交互：
 * - 拖动移动（锁定后禁用）；Ctrl+滚轮缩放；Alt+滚轮调透明度
 * - Esc 关闭
 * - 工具条：复制 / 保存 / 透明度 / 置顶切换 / 鼠标穿透 / 锁定 / 关闭
 * 上下文（图片路径/显示尺寸）通过 pin_context 命令按窗口标签取回。
 */
export function PinWindow() {
  const [ctx, setCtx] = useState<PinContext | null>(null)
  const [opacity, setOpacity] = useState(1)
  const [scale, setScale] = useState(1)
  const [topmost, setTopmost] = useState(true)
  const [locked, setLocked] = useState(false)
  const [passThrough, setPassThrough] = useState(false)
  const passTimer = useRef<number | undefined>(undefined)

  useEffect(() => {
    pinContext()
      .then(setCtx)
      .catch(() => setCtx(null))
  }, [])

  // 卸载/ctx 变化时清理穿透定时器，避免回调打到已关窗口
  useEffect(() => {
    return () => {
      if (passTimer.current !== undefined) window.clearTimeout(passTimer.current)
    }
  }, [])

  useEffect(() => {
    const onWheel = (e: WheelEvent) => {
      if (e.ctrlKey) {
        e.preventDefault()
        setScale((s) => Math.min(8, Math.max(0.1, s * (e.deltaY < 0 ? 1.1 : 0.9))))
      } else if (e.altKey) {
        // Alt+滚轮：透明度（0.15 ~ 1）
        e.preventDefault()
        setOpacity((o) => Math.min(1, Math.max(0.15, o + (e.deltaY < 0 ? 0.05 : -0.05))))
      }
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

  // 开启鼠标穿透：窗口对鼠标透明 + 定时自动恢复
  const enablePassThrough = () => {
    if (!ctx) return
    setPassThrough(true)
    pinSetIgnoreCursorEvents(ctx.label, true).catch(() => {})
    if (passTimer.current !== undefined) window.clearTimeout(passTimer.current)
    passTimer.current = window.setTimeout(() => {
      passTimer.current = undefined
      setPassThrough(false)
      pinSetIgnoreCursorEvents(ctx.label, false).catch(() => {})
    }, PASS_THROUGH_TTL_MS)
  }

  if (!ctx) return <div className="pin-root" />

  // 拖动放在整个窗口上（窗口比图片大一圈，留白处也应可拖动）；
  // 工具栏已 stopPropagation，不会误触拖动。锁定后禁用拖动。
  const onDragStart = (e: ReactPointerEvent<HTMLDivElement>) => {
    if (e.button === 0 && !locked) dragWindow().catch(() => {})
  }

  const toggleTopmost = () => {
    const next = !topmost
    setTopmost(next)
    pinSetTopmost(ctx.label, next).catch(() => setTopmost(topmost))
  }

  const rootCls = ['pin-root', locked ? 'locked' : '', passThrough ? 'pass-through' : '']
    .filter(Boolean)
    .join(' ')

  return (
    <div className={rootCls} onPointerDown={onDragStart}>
      <img
        className="pin-img"
        src={imgSrc(ctx.src)}
        style={{
          width: ctx.width,
          height: ctx.height,
          opacity,
          transform: `scale(${scale})`,
        }}
        draggable={false}
        alt=""
      />
      {passThrough && (
        <div className="pin-toast">
          鼠标穿透中 · {PASS_THROUGH_TTL_MS / 1000} 秒后自动恢复
        </div>
      )}
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
        <button className={topmost ? 'active' : ''} onClick={toggleTopmost}>
          置顶
        </button>
        <button onClick={enablePassThrough}>穿透</button>
        <button
          className={locked ? 'active' : ''}
          onClick={() => setLocked((l) => !l)}
        >
          锁定
        </button>
        <button className="danger" onClick={() => pinClose(ctx.label).catch(() => {})}>
          关闭
        </button>
      </div>
    </div>
  )
}
