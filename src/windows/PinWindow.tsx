import { useEffect, useRef, useState, type PointerEvent as ReactPointerEvent } from 'react'
import {
  dragWindow,
  imgSrc,
  onPinMenuAction,
  onPinSetOpacity,
  pinApplyEdit,
  pinClose,
  pinContext,
  pinCopy,
  pinMenuOpen,
  pinPublishFlags,
  pinSave,
  pinSetIgnoreCursorEvents,
  pinSetTopmost,
  readImageBytes,
  uiLog,
  type PinContext,
} from '../lib/api'
import '../styles/pin.css'

/** 鼠标穿透自动恢复时长（ms）。
 *  穿透 = 窗口对鼠标透明（工具条也点不到），无法从窗内主动退出，
 *  故用定时恢复；需要长时间穿透的场景用"持续锁定"以外的临时查看。 */
const PASS_THROUGH_TTL_MS = 5000

/** 图像处理参数。**始终从原图重算**，不做累积叠加——
 *  逐次叠加亮度/旋转会让像素反复重采样、越处理越糊。 */
interface EditParams {
  /** 顺时针 0 / 90 / 180 / 270 */
  rotate: number
  flipH: boolean
  flipV: boolean
  gray: boolean
  invert: boolean
  /** 图像 alpha（"图像处理 → 不透明度"），与窗口不透明度是两回事 */
  alpha: number
  /** 亮度系数，1 = 原始 */
  brightness: number
}

const DEFAULT_EDIT: EditParams = {
  rotate: 0,
  flipH: false,
  flipV: false,
  gray: false,
  invert: false,
  alpha: 1,
  brightness: 1,
}

/** 亮度单步与上下限（菜单"调整亮度"每次一步，快捷键同） */
const BRIGHT_STEP = 0.1
const BRIGHT_MIN = 0.4
const BRIGHT_MAX = 2

const isDefaultEdit = (e: EditParams) =>
  e.rotate === 0 && !e.flipH && !e.flipV && !e.gray && !e.invert && e.alpha === 1 && e.brightness === 1

/** 一张已解码的原图 + 它对应的逻辑显示尺寸（物理→逻辑的换算基准）。 */
interface Origin {
  bitmap: ImageBitmap
  /** 原图在贴图窗里的逻辑显示宽（首次打开时的 ctx.width） */
  logicalW: number
  logicalH: number
}

/**
 * 贴图窗口：无边框置顶透明窗，展示一张截图。
 * 交互：
 * - 拖动移动（锁定后禁用）；Ctrl+滚轮缩放；Alt+滚轮调透明度
 * - 悬停显示工具条；**右键弹出自绘菜单**（与参考图一致）
 * - Esc 关闭；Y 边框阴影 / T 置顶 / L 锁定 / H 显示隐藏工具条；
 *   1 右旋 / 2 左旋 / 3 水平翻转 / 4 垂直翻转 / 5 灰度 / 6 反色 / 0 重置处理
 * 上下文（图片路径/显示尺寸）通过 pin_context 命令按窗口标签取回。
 *
 * 图像处理的实现方式（关键设计）：
 * - 前端持有**原图** ImageBitmap，每次操作都是"原图 + 参数 → canvas"重新光栅化，
 *   再把结果 PNG 交给 Rust 落盘并替换贴图源图（`pin_apply_edit`）。
 * - 因此 `复制图像` / `保存图像`（Rust 读的正是源图文件）自动等于屏幕上看到的样子，
 *   不需要给这两条路径各写一套处理逻辑。
 * - 旋转 90°/270° 时宽高互换，Rust 会按新尺寸调整窗口并保持中心不动。
 */
export function PinWindow() {
  const [ctx, setCtx] = useState<PinContext | null>(null)
  const [opacity, setOpacity] = useState(1)
  const [scale, setScale] = useState(1)
  const [topmost, setTopmost] = useState(true)
  const [locked, setLocked] = useState(false)
  const [shadow, setShadow] = useState(true)
  const [toolbar, setToolbar] = useState(true)
  const [passThrough, setPassThrough] = useState(false)
  const [edit, setEdit] = useState<EditParams>(DEFAULT_EDIT)
  const [origReady, setOrigReady] = useState(false)
  const passTimer = useRef<number | undefined>(undefined)
  /** 原图（只加载一次；后续处理都从它重算，绝不拿处理结果当新原图） */
  const origRef = useRef<Origin | null>(null)
  /** 是否已经做过至少一次处理（决定"默认参数"要不要真的重算一次——
   *  刚打开时不需要，重置回默认时需要把原图重新贴回去）。 */
  const bakedRef = useRef(false)
  /** 处理任务串行化：编码/落盘期间只保留**最后一次**请求，
   *  避免连点亮度时多个结果乱序覆盖窗口尺寸。 */
  const queueRef = useRef<EditParams | null>(null)
  const busyRef = useRef(false)
  /** 当前窗口 label（处理任务用；避免把 ctx 放进 effect 依赖里触发循环） */
  const labelRef = useRef<string | null>(null)
  /** 右键菜单动作表（每次渲染刷新一次）。
   *
   *  监听器只在挂载时注册一次（避免反复订阅/退订的竞态），但动作必须用
   *  **最新一次渲染**的状态——若在监听回调里直接闭包 setState/toggleTopmost，
   *  拿到的会是挂载时的旧值（例如先点工具条取消置顶，再从菜单点置顶会把它
   *  又置回去）。用 ref 间接调用即可两全。 */
  const actionsRef = useRef<Record<string, () => void>>({})

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

  // 原图只解码一次：处理链路与"复制/保存"都靠它保证所见即所得。
  // 依赖 label 而不是整个 ctx——每次处理都会换 src/尺寸，不能因此重新解码。
  const label = ctx?.label ?? null
  labelRef.current = label
  useEffect(() => {
    if (!label || !ctx) return
    let alive = true
    let bitmap: ImageBitmap | null = null
    // 走 Rust 读字节 → Blob → ImageBitmap：绕开 asset 协议的跨域限制，
    // 与浮层取色同一条链路（canvas 读取这种来源不会被判污染）。
    readImageBytes(ctx.src)
      .then((buf) => createImageBitmap(new Blob([buf], { type: 'image/png' })))
      .then((b) => {
        if (!alive) {
          b.close()
          return
        }
        bitmap = b
        origRef.current = { bitmap: b, logicalW: ctx.width, logicalH: ctx.height }
        setOrigReady(true)
      })
      .catch((e) => uiLog(`pin origin decode failed: ${String(e)}`))
    return () => {
      alive = false
      origRef.current = null
      setOrigReady(false)
      bitmap?.close()
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps -- 只在换图（label 变）时解码
  }, [label])

  // 界面状态上报：右键菜单（另一个 webview）要画勾选标记与当前档位，
  // 靠 Rust 缓存的这份快照。挂载后和每次变化都发一次。
  useEffect(() => {
    if (!label) return
    pinPublishFlags(label, {
      topmost,
      locked,
      shadow,
      toolbar,
      opacity,
      rotate: edit.rotate,
      flipH: edit.flipH,
      flipV: edit.flipV,
      gray: edit.gray,
      invert: edit.invert,
      alpha: edit.alpha,
      brightness: edit.brightness,
    }).catch(() => {})
  }, [label, topmost, locked, shadow, toolbar, opacity, edit])

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
      if (!label) return
      // 滑条获得焦点时方向键归它用；字母键不冲突也一并让行
      const tag = (e.target as HTMLElement | null)?.tagName
      if (tag === 'INPUT' && e.key !== 'Escape') return
      // 数字键 = 右键菜单"图像处理"里标注的快捷键，走同一套动作表
      const byDigit: Record<string, string> = {
        '1': 'pin-rotate-right',
        '2': 'pin-rotate-left',
        '3': 'pin-flip-h',
        '4': 'pin-flip-v',
        '5': 'pin-grayscale',
        '6': 'pin-invert',
        '0': 'pin-reset-edit',
      }
      const digitAction = byDigit[e.key]
      if (digitAction && !e.ctrlKey && !e.altKey && !e.metaKey) {
        e.preventDefault()
        actionsRef.current[digitAction]?.()
        return
      }
      if (e.key === 'Escape') {
        e.preventDefault()
        pinClose(label).catch(() => {})
      } else if (e.key === 'y' || e.key === 'Y') {
        e.preventDefault()
        setShadow((s) => !s)
      } else if (e.key === 't' || e.key === 'T') {
        e.preventDefault()
        actionsRef.current['pin-topmost']?.()
      } else if (e.key === 'l' || e.key === 'L') {
        e.preventDefault()
        setLocked((l) => !l)
      } else if (e.key === 'h' || e.key === 'H') {
        e.preventDefault()
        setToolbar((t) => !t)
      }
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  })

  /** 原图 + 参数 → PNG dataURL，并算出处理后的**逻辑**尺寸。
   *
   *  物理像素上做变换（原图是物理像素，缩放才能保持清晰），显示尺寸按
   *  首次打开时的 物理→逻辑 比例换算（高 DPI 下 1 物理像素 ≠ 1 逻辑像素）。
   */
  const renderEdit = (orig: Origin, p: EditParams) => {
    const bmp = orig.bitmap
    const swap = p.rotate === 90 || p.rotate === 270
    const outW = swap ? bmp.height : bmp.width
    const outH = swap ? bmp.width : bmp.height
    const canvas = document.createElement('canvas')
    canvas.width = outW
    canvas.height = outH
    const g = canvas.getContext('2d')
    if (!g) throw new Error('canvas 2d context unavailable')
    g.imageSmoothingEnabled = true
    g.imageSmoothingQuality = 'high'
    g.globalAlpha = p.alpha
    // 灰度/反色/亮度交给 canvas filter（Chromium 原生实现，比逐像素快得多）
    g.filter = `grayscale(${p.gray ? 1 : 0}) invert(${p.invert ? 1 : 0}) brightness(${p.brightness})`
    // 以画布中心为轴做旋转 + 翻转，省去按角度分情况算目标坐标
    g.translate(outW / 2, outH / 2)
    g.rotate((p.rotate * Math.PI) / 180)
    g.scale(p.flipH ? -1 : 1, p.flipV ? -1 : 1)
    g.drawImage(bmp, -bmp.width / 2, -bmp.height / 2)
    const ratio = orig.logicalW / bmp.width
    return {
      png: canvas.toDataURL('image/png'),
      width: Math.max(8, Math.round(outW * ratio)),
      height: Math.max(8, Math.round(outH * ratio)),
    }
  }

  /** 把当前参数应用一次（串行化 + 只保留最后一次请求）。 */
  const flushEdit = async () => {
    const orig = origRef.current
    const lab = labelRef.current
    if (!orig || !lab || busyRef.current) return
    busyRef.current = true
    try {
      while (queueRef.current) {
        const p = queueRef.current
        queueRef.current = null
        const out = renderEdit(orig, p)
        const next = await pinApplyEdit(lab, out.png, out.width, out.height)
        bakedRef.current = true
        // 只更新"显示用的"上下文（src/尺寸）；原图与原比例保持不变
        setCtx(next)
      }
    } catch (e) {
      uiLog(`pin edit failed: ${String(e)}`)
    } finally {
      busyRef.current = false
    }
  }

  // 参数变化 → 排队重算。原图还没解码好时先跳过：解码完成后 origReady 变化
  // 会再触发一次，那时的 edit 已经是最新值。
  useEffect(() => {
    if (!origReady) return
    if (isDefaultEdit(edit) && !bakedRef.current) return
    queueRef.current = edit
    void flushEdit()
    // eslint-disable-next-line react-hooks/exhaustive-deps -- flushEdit 用 ref 读最新状态
  }, [edit, origReady])

  // 右键菜单动作（Rust 从菜单窗转发过来）。只注册一次；具体动作查
  // actionsRef（下方按每次渲染的最新状态重建）。
  useEffect(() => {
    const unlisten = onPinMenuAction((id) => {
      actionsRef.current[id]?.()
    })
    return () => {
      void unlisten.then((fn) => fn())
    }
  }, [])

  // 菜单里的「不透明度」滑块：拖动过程中连续到达，直接落到显示用的 opacity。
  // 与工具条滑条 / Alt+滚轮是同一条链路（都只改 <img> 的 style.opacity）。
  useEffect(() => {
    const unlisten = onPinSetOpacity((v) => setOpacity(v))
    return () => {
      void unlisten.then((fn) => fn())
    }
  }, [])

  // 以下所有 hook 必须先执行完再早退（ctx 首帧为 null）：
  // 早退之后再写 hook 会导致两帧 hook 数量不一致而崩。
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

  // 开启鼠标穿透：窗口对鼠标透明 + 定时自动恢复
  const enablePassThrough = () => {
    setPassThrough(true)
    pinSetIgnoreCursorEvents(ctx.label, true).catch(() => {})
    if (passTimer.current !== undefined) window.clearTimeout(passTimer.current)
    passTimer.current = window.setTimeout(() => {
      passTimer.current = undefined
      setPassThrough(false)
      pinSetIgnoreCursorEvents(ctx.label, false).catch(() => {})
    }, PASS_THROUGH_TTL_MS)
  }

  const patchEdit = (f: (e: EditParams) => EditParams) => setEdit((e) => f(e))
  const roundBright = (v: number) =>
    Math.min(BRIGHT_MAX, Math.max(BRIGHT_MIN, Math.round(v * 100) / 100))

  // 动作表每次渲染重建：监听器（只注册一次）通过 ref 取到"当前这套闭包"，
  // 因此总是基于最新状态执行（旧闭包会把刚取消的置顶又置回去）。
  actionsRef.current = {
    'pin-copy': () => pinCopy(ctx.label).catch(() => {}),
    'pin-save': () => pinSave(ctx.label).catch(() => {}),
    'pin-pass-through': enablePassThrough,
    'pin-shadow': () => setShadow((s) => !s),
    'pin-topmost': toggleTopmost,
    'pin-lock': () => setLocked((l) => !l),
    'pin-toolbar': () => setToolbar((t) => !t),
    'pin-close': () => pinClose(ctx.label).catch(() => {}),
    // ---- 图像处理 ----
    'pin-rotate-right': () => patchEdit((e) => ({ ...e, rotate: (e.rotate + 90) % 360 })),
    'pin-rotate-left': () => patchEdit((e) => ({ ...e, rotate: (e.rotate + 270) % 360 })),
    'pin-flip-h': () => patchEdit((e) => ({ ...e, flipH: !e.flipH })),
    'pin-flip-v': () => patchEdit((e) => ({ ...e, flipV: !e.flipV })),
    'pin-grayscale': () => patchEdit((e) => ({ ...e, gray: !e.gray })),
    'pin-invert': () => patchEdit((e) => ({ ...e, invert: !e.invert })),
    'pin-reset-edit': () => setEdit({ ...DEFAULT_EDIT }),
    'pin-alpha-25': () => patchEdit((e) => ({ ...e, alpha: 0.25 })),
    'pin-alpha-50': () => patchEdit((e) => ({ ...e, alpha: 0.5 })),
    'pin-alpha-75': () => patchEdit((e) => ({ ...e, alpha: 0.75 })),
    'pin-alpha-100': () => patchEdit((e) => ({ ...e, alpha: 1 })),
    'pin-bright-up': () => patchEdit((e) => ({ ...e, brightness: roundBright(e.brightness + BRIGHT_STEP) })),
    'pin-bright-down': () => patchEdit((e) => ({ ...e, brightness: roundBright(e.brightness - BRIGHT_STEP) })),
    'pin-bright-reset': () => patchEdit((e) => ({ ...e, brightness: 1 })),
  }

  const rootCls = ['pin-root', locked ? 'locked' : '', passThrough ? 'pass-through' : '', shadow ? '' : 'no-shadow']
    .filter(Boolean)
    .join(' ')

  return (
    <div
      className={rootCls}
      onPointerDown={onDragStart}
      onContextMenu={(e) => {
        // 原生右键菜单已被 webview 初始化脚本抑制；这里弹应用自绘菜单
        // （Rust 记住目标为本窗，动作经 pin-menu-action 事件回来）。
        e.preventDefault()
        pinMenuOpen().catch(() => {})
      }}
    >
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
      {/* 工具条：右键菜单"工具栏"开关 / H 键可整体隐藏。
          关掉后贴图只剩图片（拖动、右键菜单照常可用）——给"贴在别处看内容"的场景让路。 */}
      {toolbar && (
        <div className="pin-toolbar" onPointerDown={(e) => e.stopPropagation()}>
          <button onClick={() => pinCopy(ctx.label).catch(() => {})}>复制</button>
          <button onClick={() => pinSave(ctx.label).catch(() => {})}>保存</button>
          <label className="pin-opacity" title="透明度（Alt+滚轮也可调）">
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
      )}
    </div>
  )
}
