import { listen } from '@tauri-apps/api/event'
import {
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
  type CSSProperties,
  type PointerEvent as ReactPointerEvent,
} from 'react'
import {
  cancelCapture,
  closeSelf,
  confirmSelection,
  imgSrc,
  overlayContext,
  uiLog,
  type OverlayContext,
  type Rect,
  type ShotAction,
} from '../lib/api'
import { AnnotationLayer } from '../annotate/AnnotationLayer'
import {
  ArrowIcon,
  CheckIcon,
  CloseIcon,
  EllipseIcon,
  LongShotIcon,
  MosaicIcon,
  OcrIcon,
  PenIcon,
  PinIcon,
  RectIcon,
  SaveIcon,
  ShareIcon,
  StickerIcon,
  TextIcon,
  TranslateIcon,
  UndoIcon,
} from '../annotate/icons'
import {
  PRESET_COLORS,
  MOSAIC_BLOCKS,
  toolMeta,
  type AnnotateTool,
  type Rgba,
  type ShapeDraft,
} from '../annotate/tools'
import '../styles/overlay.css'

const MIN_SIZE = 3

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
  const [busy, setBusy] = useState(false)
  const [actError, setActError] = useState<string | null>(null)
  // 标注状态
  const [shapes, setShapes] = useState<ShapeDraft[]>([])
  /** null = 未选工具（"选择"态）：此时画布不拦截鼠标，可在选区内重新框选 */
  const [tool, setTool] = useState<AnnotateTool | null>(null)
  const [color, setColor] = useState<Rgba>(PRESET_COLORS[0])
  const [width, setWidth] = useState(2)
  const [mosaicBlock, setMosaicBlock] = useState(MOSAIC_BLOCKS[1])
  /** 未实现工具的提示气泡（1.8s 自动消失） */
  const [hint, setHint] = useState<string | null>(null)
  const hintTimer = useRef<number | undefined>(undefined)
  /** 二级菜单锚点：跟随当前选中的一级按钮（x 居中、bottom = 按钮底边） */
  const barRef = useRef<HTMLDivElement | null>(null)
  const propsRef = useRef<HTMLDivElement | null>(null)
  const hintRef = useRef<HTMLDivElement | null>(null)
  const [anchor, setAnchor] = useState<{ x: number; bottom: number } | null>(null)
  const [hintAnchor, setHintAnchor] = useState<{ x: number; bottom: number } | null>(null)
  const [propsW, setPropsW] = useState(0)
  const [hintW, setHintW] = useState(0)
  const imgRef = useRef<HTMLImageElement | null>(null)
  const dragRef = useRef<{ x: number; y: number } | null>(null)
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
      setShapes([])
      setTool(null)
      setHint(null)
      setCursor(null)
      setBusy(false)
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
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        e.preventDefault()
        void exitCapture()
      } else if (e.key === 'Enter' && rect && !busy) {
        e.preventDefault()
        void finish('copy')
      } else if (e.ctrlKey && e.key.toLowerCase() === 'z') {
        e.preventDefault()
        setShapes((s) => s.slice(0, -1))
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
      await confirmSelection(ctx.monitorId, rect, action, shapes, ctx.sessionId)
      // Rust 已关全部浮层；自关兜底（双屏下某块浮层漏关时由③轮询兜底）
      closeSelf().catch(() => {})
    } catch (e) {
      console.error('confirm selection failed', e)
      setActError(`${ACTION_LABEL[action]}失败：${String(e)}`)
      setBusy(false)
    }
  }

  // 未实现工具：给出短提示，不做静默失败（用户点了要有反馈）
  const showHint = (text: string, el?: HTMLElement | null) => {
    if (el) {
      const r = el.getBoundingClientRect()
      setHintAnchor({ x: r.left + r.width / 2, bottom: r.bottom })
    }
    setHint(`${text} 正在开发中`)
    if (hintTimer.current !== undefined) window.clearTimeout(hintTimer.current)
    hintTimer.current = window.setTimeout(() => setHint(null), 1800)
  }

  /** 选工具：再次点击已选中的工具则回到"选择"态（工具条无高亮）。 */
  const pickTool = (t: AnnotateTool) => {
    const same = tool === t || (t === 'pen' && tool === 'highlighter')
    setTool(same ? null : t)
    if (!same) setWidth(toolMeta(t).defaultWidth)
  }

  /** 二级菜单贴着当前工具按钮：取按钮矩形的水平中心与底边 */
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
    // 同帧量出新面板的实际宽度（切换工具时内容宽度不同），供边界收拢使用
    const w = propsRef.current?.offsetWidth ?? 0
    if (w && w !== propsW) setPropsW(w)
  }, [tool, propsW])

  // 提示气泡宽度同样实测，避免收拢时用到偏大的估计值
  useLayoutEffect(() => {
    const w = hintRef.current?.offsetWidth ?? 0
    if (w && w !== hintW) setHintW(w)
  }, [hint, hintW])

  /** 二级面板居中于按钮，但左右两侧不得超出一级工具条的左右边界 */
  const anchorX = (x: number, w: number) => {
    const bar = barRef.current?.getBoundingClientRect()
    const half = w / 2
    if (!bar) return Math.min(Math.max(x, half + 8), window.innerWidth - half - 8)
    // 面板比工具条还宽：没有收拢余地，直接与工具条左对齐
    if (bar.width <= w) return bar.left + half
    return Math.min(Math.max(x, bar.left + half), bar.right - half)
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
      className="overlay-root"
      onPointerDown={onPointerDown}
      onPointerMove={onPointerMove}
      onPointerUp={onPointerUp}
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
            tool={tool}
            color={color}
            width={width}
            mosaicBlock={mosaicBlock}
            shapes={shapes}
            onShapesChange={setShapes}
          />
        </div>
      )}

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
                className="sbtn"
                title="贴纸"
                onClick={(e) => showHint('贴纸工具', e.currentTarget)}
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
                className="sbtn"
                title="文字"
                onClick={(e) => showHint('文字工具', e.currentTarget)}
              >
                <TextIcon />
              </button>
            </div>

            <i className="sb-divider" />

            {/* 第二组：识别类（尚未实现，按参考图置灰） */}
            <div className="sb-group">
              <button className="sbtn" disabled title="翻译（即将支持）">
                <TranslateIcon />
              </button>
              <button className="sbtn" disabled title="文字识别（即将支持）">
                <OcrIcon />
              </button>
              <button className="sbtn" title="长截图" onClick={(e) => showHint('长截图', e.currentTarget)}>
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
                onClick={() => setShapes((s) => s.slice(0, -1))}
              >
                <UndoIcon />
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
              <button
                className="sbtn"
                title="复制并分享"
                disabled={busy}
                onClick={() => void finish('copy')}
              >
                <ShareIcon />
              </button>
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
              className="sb-props"
              ref={propsRef}
              style={
                anchor
                  ? { left: anchorX(anchor.x, propsW || 260), top: anchor.bottom + 6 }
                  : undefined
              }
              onPointerDown={(e) => e.stopPropagation()}
            >
              {(tool === 'pen' || tool === 'highlighter') && (
                <div className="sb-seg">
                  <button className={tool === 'pen' ? 'on' : ''} onClick={() => setTool('pen')}>
                    画笔
                  </button>
                  <button
                    className={tool === 'highlighter' ? 'on' : ''}
                    onClick={() => setTool('highlighter')}
                  >
                    荧光笔
                  </button>
                </div>
              )}
              {tool === 'mosaic' ? (
                <div className="sb-seg">
                  {MOSAIC_BLOCKS.map((b, i) => (
                    <button
                      key={b}
                      className={mosaicBlock === b ? 'on' : ''}
                      onClick={() => setMosaicBlock(b)}
                    >
                      {['小', '中', '大'][i]}
                    </button>
                  ))}
                </div>
              ) : (
                <>
                  <div className="sb-colors">
                    {PRESET_COLORS.map((c) => (
                      <button
                        key={c.join(',')}
                        className={`swatch ${color === c ? 'active' : ''}`}
                        style={{ background: `rgb(${c[0]},${c[1]},${c[2]})` }}
                        onClick={() => setColor(c)}
                        title={`颜色 rgba(${c.join(',')})`}
                      />
                    ))}
                  </div>
                  <i className="sb-sep" />
                  <label className="sb-width" title={`线宽 ${width}px`}>
                    <input
                      type="range"
                      min={1}
                      max={24}
                      step={1}
                      value={width}
                      onChange={(e) => setWidth(Number(e.target.value))}
                    />
                  </label>
                </>
              )}
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
          {actError && <div className="sb-hint error">{actError}</div>}
        </>
      )}
    </div>
  )
}
