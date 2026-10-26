import {
  useCallback,
  useEffect,
  useRef,
  useState,
  type CSSProperties,
  type PointerEvent as ReactPointerEvent,
} from 'react'
import { listen } from '@tauri-apps/api/event'
import {
  menuAction,
  menuClose,
  menuFit,
  menuState,
  pinFlags,
  pinGroupOf,
  pinGroups,
  pinSetOpacity,
  uiLog,
  type PinFlags,
} from '../lib/api'
import '../styles/menu.css'

/**
 * 与 Rust 侧 `menuwin::MENU_SHADOW_PAD` / `.menu-stage` padding 必须一致。
 * 窗口 = 面板 + 四周留白（放阴影）；量尺寸时要把留白加回去。
 */
const SHADOW_PAD = 18

/**
 * 菜单节点。
 * - `hint`：右侧灰色小字（快捷键）
 * - `children`：带 ▸ 的子菜单
 * - `checked`：有值才渲染左侧勾选列（对齐参考图：只有能勾的项才占位）
 * - `sep` / `warn`：分组分隔线、快捷键注册失败告警行（都不是菜单项）
 * - `slider`：整行滑块（贴图菜单第一项「不透明度」），拖动时连续回调
 */
type MenuNode = {
  id: string
  label?: string
  hint?: string
  checked?: boolean
  children?: MenuNode[]
  sep?: boolean
  warn?: boolean
  slider?: {
    value: number
    min: number
    max: number
    step: number
    /** 拖动中每一帧都调（0~1 的原始值） */
    onInput: (v: number) => void
  }
}

/** 勾选的默认值（与贴图窗初始状态一致）。
 *  快照还没到达前用它渲染，避免勾选标记闪变。 */
const PIN_FLAG_DEFAULTS: PinFlags = {
  topmost: true,
  locked: false,
  shadow: true,
  toolbar: true,
  opacity: 1,
  rotate: 0,
  flipH: false,
  flipV: false,
  gray: false,
  invert: false,
  alpha: 1,
  brightness: 1,
}

/** 浮点比较（快照里的 alpha 是序列化回来的 0.75，不能直接 ===）。 */
const near = (a: number, b: number) => Math.abs(a - b) < 0.001

const SEP = (id: string): MenuNode => ({ id, sep: true })

/**
 * 托盘 / 悬浮球菜单（应用级动作）。
 *
 * `failed` 非空时在最前面插一条告警——热键注册失败时这是用户唯一的可见线索。
 */
function appMenu(failed: string[]): MenuNode[] {
  return [
    ...(failed.length > 0
      ? [{ id: 'warn', label: `${failed.join(' / ')} 已被其他程序占用`, warn: true }]
      : []),
    { id: 'capture', label: '截图', hint: 'Ctrl+1' },
    { id: 'pin-clipboard', label: '剪贴板贴图', hint: 'Ctrl+2' },
    { id: 'toggle-ball', label: '显示 / 隐藏悬浮球' },
    SEP('sep-1'),
    { id: 'pin-manager', label: '贴图管理…' },
    { id: 'settings', label: '设置' },
    { id: 'about', label: '关于 SnapPin' },
    { id: 'open-log', label: '打开运行日志' },
    // macOS 专属：屏幕录制授权"勾着却无效"时的自救入口
    // （清掉 TCC 记录，下次启动才会重新弹系统授权框）
    ...(typeof navigator !== 'undefined' && /Mac/i.test(navigator.userAgent)
      ? [
          { id: 'reset-perm', label: '重置屏幕录制授权…' },
          // 长截图自动滚动的前置权限：未授权时系统会**静默丢弃**滚动事件
          // （页面不动、也不报错），所以给它一个显式入口 + 重置入口。
          { id: 'open-access', label: '辅助功能权限…' },
          { id: 'reset-access', label: '重置辅助功能授权…' },
        ]
      : []),
    SEP('sep-2'),
    { id: 'quit', label: '退出' },
  ]
}

/**
 * 贴图右键菜单。
 *
 * 顺序：**不透明度滑块（第一项）** → 图像处理 ▸ → 复制/保存 → 鼠标穿透 →
 * 开关项 → 关闭。工具条上的每个固定功能在这里都有对应入口，因此右键菜单
 * 可以完全替代工具条。
 *
 * 为什么滑块放最前：透明度是贴图最常调的一项（贴在别的窗口上时几乎每次都要
 * 压一点），放子菜单里要点两级；提出来用滑块后一次拖动就能找到合适的值。
 *
 * 勾选态来自贴图窗上报的快照（`PinFlags`）；菜单只负责显示与转发 id，
 * 真正的动作在 PinWindow 的 actionsRef 里。**滑块是例外**：它需要连续值，
 * 所以走 `pinSetOpacity`（Rust 转 `pin-set-opacity` 事件给贴图窗）。
 *
 * 注意「不透明度」（顶部滑块 = 窗口整体 alpha，等同工具条滑条）与
 * 「图像处理 → 图像不透明度」（把透明**烤进图片像素**，复制/保存出去也半透明）
 * 是两件事，命名刻意区分。
 */
function pinMenu(
  flags: PinFlags,
  opacity: number,
  onSlide: (v: number) => void,
  groups: string[],
  curGroup: string,
): MenuNode[] {
  return [
    {
      id: 'pin-opacity',
      label: '不透明度',
      slider: { value: opacity, min: 0.15, max: 1, step: 0.05, onInput: onSlide },
    },
    SEP('sep-0'),
    // 多贴图分组管理：换归属 / 只看本组 / 隐藏本组 / 显示全部。
    // 动作 id 由 Rust 的 menu_action 直接处理（全局操作，不转发给贴图窗）。
    {
      id: 'pin-group',
      label: '分组',
      children: [
        ...groups.map((g) => ({
          id: `pin-group:${g}`,
          label: g,
          checked: g === curGroup,
        })),
        SEP('sep-group-0'),
        { id: 'pin-group-only', label: `只看「${curGroup || '默认'}」组` },
        { id: 'pin-group-hide', label: `隐藏「${curGroup || '默认'}」组` },
        { id: 'pin-group-show-all', label: '显示全部贴图' },
      ],
    },
    {
      id: 'pin-image',
      label: '图像处理',
      children: [
        { id: 'pin-rotate-right', label: '向右旋转', hint: '1' },
        { id: 'pin-rotate-left', label: '向左旋转', hint: '2' },
        SEP('sep-flip'),
        { id: 'pin-flip-h', label: '水平翻转', hint: '3', checked: flags.flipH },
        { id: 'pin-flip-v', label: '垂直翻转', hint: '4', checked: flags.flipV },
        SEP('sep-color'),
        { id: 'pin-grayscale', label: '灰度化', hint: '5', checked: flags.gray },
        { id: 'pin-invert', label: '反色', hint: '6', checked: flags.invert },
        SEP('sep-alpha'),
        {
          id: 'pin-alpha',
          label: '图像不透明度',
          children: [
            { id: 'pin-alpha-100', label: '100%', checked: near(flags.alpha, 1) },
            { id: 'pin-alpha-75', label: '75%', checked: near(flags.alpha, 0.75) },
            { id: 'pin-alpha-50', label: '50%', checked: near(flags.alpha, 0.5) },
            { id: 'pin-alpha-25', label: '25%', checked: near(flags.alpha, 0.25) },
          ],
        },
        {
          id: 'pin-bright',
          label: '调整亮度',
          children: [
            { id: 'pin-bright-up', label: '变亮' },
            { id: 'pin-bright-down', label: '变暗' },
            { id: 'pin-bright-reset', label: '重置亮度' },
            SEP('sep-bright'),
            { id: 'pin-bright-note', label: `当前 ${Math.round(flags.brightness * 100)}%` },
          ],
        },
        SEP('sep-reset'),
        { id: 'pin-reset-edit', label: '重置所有处理', hint: '0' },
      ],
    },
    { id: 'pin-copy', label: '复制图像' },
    { id: 'pin-save', label: '保存图像' },
    SEP('sep-3'),
    { id: 'pin-pass-through', label: '鼠标穿透' },
    SEP('sep-4'),
    { id: 'pin-shadow', label: '边框阴影', hint: 'Y', checked: flags.shadow },
    { id: 'pin-topmost', label: '窗口置顶', hint: 'T', checked: flags.topmost },
    { id: 'pin-lock', label: '锁定', hint: 'L', checked: flags.locked },
    { id: 'pin-toolbar', label: '工具栏', hint: 'H', checked: flags.toolbar },
    SEP('sep-5'),
    { id: 'pin-close', label: '关闭', hint: 'Esc' },
  ]
}

/**
 * 自绘菜单窗。
 *
 * 生命周期由 Rust 驱动（三段式，见 menuwin.rs）：
 * 打开锚点 → 本组件渲染并量尺寸 → `menuFit` 让 Rust 定位显示。
 * 所以**首次挂载必须主动 fit 一次**：窗口刚建好时前端还没监听 `menu-open`，
 * 那次事件必然丢失。
 *
 * 两种菜单（按 `menu-open` 载荷的 `kind` 切换），**共用同一套浅色样式**：
 * - `app`：托盘 / 悬浮球（应用级动作）
 * - `pin`：贴图右键（图像处理子菜单 + 开关项勾选，动作转发给贴图窗）
 *
 * 子菜单的尺寸（关键）：子菜单只在悬停时可见。若等它出现再 resize 窗口，
 * 窗口会在指针下方跳动、hover 瞬间被打断 → 闪烁。因此 `fit` 时**把隐藏的
 * 子菜单一起量进去**（`visibility:hidden` 仍有布局盒与 rect），一次给足尺寸，
 * 之后展开子菜单不再动窗口。
 */
export function MenuWindow() {
  const rootRef = useRef<HTMLDivElement>(null)
  /** 所有子菜单面板（含隐藏的），用于量取最大外扩范围 */
  const subRefs = useRef<Set<HTMLDivElement>>(new Set())
  const [failed, setFailed] = useState<string[]>([])
  /** 非空 = 当前渲染贴图菜单（值为目标贴图窗 label） */
  const [pinLabel, setPinLabel] = useState<string | null>(null)
  /** 贴图状态快照（null = 还没读到，用默认值渲染） */
  const [pinOn, setPinOn] = useState<PinFlags | null>(null)
  /** 贴图分组：清单 + 当前贴图所在组（右键菜单打开时取） */
  const [pinGroupsList, setPinGroupsList] = useState<string[]>(['默认'])
  const [pinGroup, setPinGroup] = useState<string>('默认')
  /** 滑块拖动中的本地值（null = 还没拖过，用快照值）。
   *  拖动期间不能再跟随快照：贴图窗每帧都会回报新值，跟着走会与指针抢控制权。 */
  const [slideOp, setSlideOp] = useState<number | null>(null)
  /** 已展开的子菜单路径（节点 id 链）。空 = 全部收起。 */
  const [active, setActive] = useState<string[]>([])

  /** 切换菜单内容：app ↔ pin（pin 时顺带拉状态快照）。 */
  const applyKind = useCallback((kind: string, label: string | null) => {
    setActive([])
    // 每次打开都从快照重新取滑块位置（上次拖动过的值不能带过来）
    setSlideOp(null)
    if (kind === 'pin' && label) {
      setPinLabel(label)
      pinFlags(label)
        .then((f) => setPinOn(f))
        .catch(() => setPinOn(null))
      pinGroups()
        .then((g) => setPinGroupsList(g))
        .catch(() => {})
      pinGroupOf(label)
        .then((g) => setPinGroup(g))
        .catch(() => {})
    } else {
      setPinLabel(null)
      setPinOn(null)
    }
  }, [])

  /** 量出"根面板 + 所有子菜单"的最大外框 → 加上阴影留白 → 交给 Rust 定位。 */
  const fit = useCallback(() => {
    const root = rootRef.current
    if (!root) return
    const base = root.getBoundingClientRect()
    let right = base.width
    let bottom = base.height
    for (const el of subRefs.current) {
      const r = el.getBoundingClientRect()
      if (r.width === 0 || r.height === 0) continue
      right = Math.max(right, r.right - base.left)
      bottom = Math.max(bottom, r.bottom - base.top)
    }
    menuFit(Math.ceil(right) + SHADOW_PAD * 2, Math.ceil(bottom) + SHADOW_PAD * 2).catch((e) =>
      uiLog(`menu fit error: ${String(e)}`),
    )
  }, [])

  useEffect(() => {
    uiLog('menu mounted')
  }, [])

  useEffect(() => {
    let alive = true

    // 快捷键注册失败的告警行（会让面板变高，必须在 fit 之前拿到）
    menuState()
      .then((s) => {
        if (!alive) return
        setFailed(s.shortcutFailed)
        // 首次创建菜单窗时 menu-open 已经 emit 过了（那时还没监听），
        // 用服务端状态补齐菜单种类，否则第一次右键贴图会弹出托盘菜单。
        applyKind(s.kind, s.pinLabel)
      })
      .catch(() => {})

    // 每次打开都要重新 fit（窗口可能被上一次的尺寸留下，且锚点会变）。
    // 载荷带 kind/pinLabel：贴图菜单先读快照再量尺寸。
    const unlisten = listen<{ kind: string; pinLabel: string | null }>('menu-open', (e) => {
      if (!alive) return
      applyKind(e.payload.kind, e.payload.pinLabel)
      fit()
    })

    return () => {
      alive = false
      void unlisten.then((fn) => fn())
    }
  }, [applyKind, fit])

  // 首次挂载（窗口刚被创建）以及菜单内容/快照变化后：等一轮布局 + 字体就绪再量，
  // 避免量到 fallback 字体高度。内容变化（app ↔ pin、勾选到达）也要重新量。
  useEffect(() => {
    let cancelled = false
    const run = () => {
      if (!cancelled) fit()
    }
    const raf = requestAnimationFrame(() => requestAnimationFrame(run))
    void document.fonts?.ready.then(run)
    return () => {
      cancelled = true
      cancelAnimationFrame(raf)
    }
  }, [fit, failed.length, pinLabel, pinOn])

  // Esc 关闭（窗口失焦也会关，但键盘更符合习惯）
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        e.preventDefault()
        void menuClose()
      }
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [])

  // 点到面板外的透明留白：透明窗口仍会吃掉鼠标事件，
  // 不会自动失焦，所以这里主动关（否则用户会觉得"点旁边没反应"）。
  const onStagePointerDown = (e: ReactPointerEvent<HTMLDivElement>) => {
    if (!(e.target as HTMLElement).closest('.sp-menu')) {
      void menuClose()
    }
  }

  const runAction = (id: string) => {
    // Rust 侧先 hide 再执行动作（截图时菜单不能挡在浮层上）
    menuAction(id).catch((e) => uiLog(`menu action error: ${String(e)}`))
  }

  /**
   * 递归渲染一层菜单。
   *
   * `path` 是本层在 active 里的前缀，用来判断"这一项的子菜单该不该展开"；
   * 悬停任意一项就把 active 换成它的完整路径 —— 同级其它子菜单自动收起，
   * 而祖先链保持展开（三级菜单靠这一点）。
   */
  const renderPanel = (nodes: MenuNode[], path: string[], isRoot: boolean) => {
    // 同一面板里只要有一项带勾选，**所有**项都预留勾选列——否则"✓灰度化"
    // 比"向右旋转"右移 16px，标签左边缘参差不齐（参考图是全部对齐的）。
    const reserveCheck = nodes.some((n) => n.checked !== undefined)
    return (
      <div
        className={isRoot ? 'sp-menu sp-root' : 'sp-menu sp-sub'}
        role="menu"
        ref={(el) => {
          if (!el) return
          if (isRoot) {
            rootRef.current = el
            return
          }
          // 子面板随展开/收起增删：用 React 19 的 ref cleanup 精确增删，
          // 不要在渲染期清空集合——稳定挂载的面板不会重跑 ref，会被误删。
          subRefs.current.add(el)
          return () => {
            subRefs.current.delete(el)
          }
        }}
      >
        {nodes.map((node) => {
          if (node.sep) return <div key={node.id} className="sp-sep" />
          if (node.warn) {
            return (
              <div key={node.id} className="sp-warn">
                {node.label}
              </div>
            )
          }
          // 滑块行（不透明度）：不是可点项，标签在上、轨道在下占满整行。
          // 轨道左侧填充到当前值（`--p`），比单看滑块头更直观。
          if (node.slider) {
            const s = node.slider
            const pct = ((s.value - s.min) / (s.max - s.min)) * 100
            return (
              <div key={node.id} className="sp-slider">
                <div className="sp-slider-head">
                  <span className="sp-text">{node.label}</span>
                  <span className="sp-key">{Math.round(s.value * 100)}%</span>
                </div>
                <input
                  type="range"
                  className="sp-range"
                  min={s.min}
                  max={s.max}
                  step={s.step}
                  value={s.value}
                  aria-label={node.label}
                  style={{ '--p': `${pct}%` } as CSSProperties}
                  onChange={(e) => s.onInput(Number(e.target.value))}
                />
              </div>
            )
          }
          const full = [...path, node.id]
          const hasSub = !!node.children?.length
          const open = hasSub && active[path.length] === node.id
          const isNote = node.id.endsWith('-note')
          return (
            <div
              key={node.id}
              className="sp-node"
              onPointerEnter={() => setActive(hasSub ? full : path)}
            >
              <button
                type="button"
                className={`sp-item${hasSub ? ' has-sub' : ''}${open ? ' open' : ''}${
                  isNote ? ' is-note' : ''
                }`}
                disabled={isNote}
                onClick={() => {
                  if (hasSub) {
                    setActive(full)
                    return
                  }
                  runAction(node.id)
                }}
              >
                {reserveCheck || node.checked !== undefined ? (
                  <span className={node.checked ? 'sp-check on' : 'sp-check'}>
                    {node.checked ? '✓' : ''}
                  </span>
                ) : null}
                <span className="sp-text">{node.label}</span>
                {hasSub ? (
                  <span className="sp-arrow">▸</span>
                ) : node.hint ? (
                  <span className="sp-key">{node.hint}</span>
                ) : null}
              </button>
              {hasSub ? (
                <div className={open ? 'sp-subwrap on' : 'sp-subwrap'}>
                  {renderPanel(node.children!, full, false)}
                </div>
              ) : null}
            </div>
          )
        })}
      </div>
    )
  }

  const flags = pinOn ?? PIN_FLAG_DEFAULTS
  /** 滑块显示值：拖动中用本地值，否则用快照值 */
  const opVal = slideOp ?? flags.opacity
  const onSlide = (v: number) => {
    setSlideOp(v)
    if (pinLabel) pinSetOpacity(pinLabel, v).catch((e) => uiLog(`pin opacity error: ${String(e)}`))
  }
  const nodes = pinLabel ? pinMenu(flags, opVal, onSlide, pinGroupsList, pinGroup) : appMenu(failed)

  return (
    // is-pin：贴图菜单的根面板更窄（200px，见 menu.css），应用菜单保持 224px
    <div
      className={`menu-stage${pinLabel ? ' is-pin' : ''}`}
      onPointerDown={onStagePointerDown}
    >
      {renderPanel(nodes, [], true)}
    </div>
  )
}
