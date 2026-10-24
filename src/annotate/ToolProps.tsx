import { useEffect, useRef, useState } from 'react'
import { uiLog } from '../lib/api'

/**
 * 工具属性面板的内容（工具条下方的补充条）。
 *
 * 单独拆成组件有两个原因：
 * 1. OverlayWindow 已经很长，这块 UI 是自洽的，抽出来更好维护；
 * 2. 它能脱离 Tauri 独立挂载 —— 验收页可以真实渲染它、真实拖动滑条，
 *    从而验证"拖动时数值真的在变"，而不是只靠读代码。
 *
 * 所有状态都由父组件持有（受控组件），这里只负责渲染与回调。
 */
import {
  ERASER_RADIUS,
  STICKER_COLORS,
  STICKER_COLOR_NAMES,
  MOSAIC_BLOCKS,
  PRESET_COLORS,
  SPOTLIGHT_DIM,
  TEXT_FONT_SIZE,
  STICKER_GLYPHS,
  ZOOM_LEVELS,
  type AnnotateTool,
  type Rgba,
} from './tools'
import {
  filterStickerNames,
  loadStickerManifest,
  loadStickerSvg,
  recolorSvg,
  type StickerPick,
} from '../stickers/library'
import { zhLabel } from '../stickers/zh'
import { ColorPickerGlyph } from './icons'

/**
 * 贴纸面板（单行）：emoji 全平铺 + 颜色下拉 + 大小下拉 + 搜索框。
 *
 * 颜色/大小都做成**竖排下拉**（用户 2026-10-24 要求）：
 * - 颜色：按钮用用户给的喷罐图标，展开后一列"色块 + 中文色名"
 * - 大小：按钮显示当前值，展开后一列尺寸档位（替代原来的连续滑条）
 * 两个下拉都在组件外点击时收起（document 级监听），
 * 不然点别处会穿透到浮层根部、把选区给清了。
 */
function StickerPanel(p: Props) {
  const [openColor, setOpenColor] = useState(false)
  const [openSize, setOpenSize] = useState(false)
  const colorRef = useRef<HTMLDivElement>(null)
  const sizeRef = useRef<HTMLDivElement>(null)

  // 组件外点击 → 收起下拉（不能用 overlay 的 pointerdown：那会连带清选区）
  useEffect(() => {
    if (!openColor && !openSize) return
    const onDown = (e: PointerEvent) => {
      const t = e.target as Node
      if (openColor && colorRef.current && !colorRef.current.contains(t)) setOpenColor(false)
      if (openSize && sizeRef.current && !sizeRef.current.contains(t)) setOpenSize(false)
    }
    document.addEventListener('pointerdown', onDown, true)
    return () => document.removeEventListener('pointerdown', onDown, true)
  }, [openColor, openSize])

  return (
    <div className="sb-sticker-row">
      {/* 16 个 emoji 全平铺（不滚动，一屏看全） */}
      <div className="sb-emoji-row">
        {STICKER_GLYPHS.map((g) => (
          <button
            key={g}
            className={`sb-emoji ${
              p.sticker.kind === 'emoji' && p.sticker.value === g ? 'on' : ''
            }`}
            title={`表情 ${g}`}
            onClick={() => p.onSticker({ kind: 'emoji', value: g })}
          >
            {g}
          </button>
        ))}
      </div>

      {/* 颜色下拉（竖排） */}
      <div className="sb-dd" ref={colorRef}>
        <button
          className={`sb-dd-btn ${openColor ? 'on' : ''}`}
          title={`贴纸颜色：${STICKER_COLOR_NAMES[p.stickerColor] ?? p.stickerColor}`}
          onClick={() => {
            setOpenColor((v) => !v)
            setOpenSize(false)
          }}
        >
          <ColorPickerGlyph />
        </button>
        {openColor && (
          <div className="sb-dd-panel" onPointerDown={(e) => e.stopPropagation()}>
            {STICKER_COLORS.map((c) => (
              <button
                key={c}
                className={`sb-dd-item ${p.stickerColor === c ? 'on' : ''}`}
                onClick={() => {
                  p.onStickerColor(c)
                  setOpenColor(false)
                }}
              >
                <span className="sb-dd-dot" style={{ background: c }} />
                {STICKER_COLOR_NAMES[c] ?? c}
              </button>
            ))}
          </div>
        )}
      </div>

      {/* 大小下拉（竖排） */}
      <div className="sb-dd" ref={sizeRef}>
        <button
          className={`sb-dd-btn sb-dd-size ${openSize ? 'on' : ''}`}
          title={`贴纸尺寸 ${p.stickerSize}px`}
          onClick={() => {
            setOpenSize((v) => !v)
            setOpenColor(false)
          }}
        >
          {p.stickerSize}px
        </button>
        {openSize && (
          <div className="sb-dd-panel sb-dd-size" onPointerDown={(e) => e.stopPropagation()}>
            <div className="sb-dd-size-val">{p.stickerSize}px</div>
            {/* 竖排滑块：writing-mode + direction 让"值越大越靠上"
                （Chromium 121+ 原生支持，WebView2 现行版本都带）。 */}
            <input
              className="sb-dd-vslider"
              type="range"
              min={16}
              max={160}
              step={1}
              value={p.stickerSize}
              title={`贴纸尺寸 ${p.stickerSize}px`}
              onChange={(e) => p.onStickerSize(Number(e.target.value))}
            />
          </div>
        )}
      </div>

      <StickerSearch color={p.stickerColor} pick={p.sticker} onPick={p.onSticker} />
    </div>
  )
}

/**
 * SVG 图标搜索 + 结果网格（贴纸面板的第 ④ 块）。
 *
 * 为什么用 `<img>` 而不是把 2079 个全列出来：库 5.1MB，全渲染会直接把
 * 属性条撑爆。这里按搜索词过滤后**最多渲染 60 个**，滚动加载更多。
 */
function StickerSearch(props: {
  color: string
  pick: StickerPick
  onPick: (s: StickerPick) => void
}) {
  const [names, setNames] = useState<string[]>([])
  const [query, setQuery] = useState('')
  const [limit, setLimit] = useState(60)
  const [loading, setLoading] = useState(true)
  const [open, setOpen] = useState(false)

  useEffect(() => {
    let alive = true
    loadStickerManifest()
      .then((n) => {
        if (alive) setNames(n)
      })
      .catch((e) => uiLog(`sticker manifest failed: ${String(e)}`))
      .finally(() => {
        if (alive) setLoading(false)
      })
    return () => {
      alive = false
    }
  }, [])

  const hits = filterStickerNames(names, query).slice(0, limit)

  return (
    <div className="sb-sticker-search">
      <input
        className="sb-sticker-input"
        type="text"
        value={query}
        placeholder={`搜索 ${names.length} 个图标`}
        onFocus={() => setOpen(true)}
        onChange={(e) => {
          setQuery(e.target.value)
          setLimit(60)
          setOpen(true)
        }}
      />
      {/* 下拉浮层：不占那一行的高度，点外面收起 */}
      {open && (
        <div className="sb-sticker-drop" onPointerDown={(e) => e.stopPropagation()}>
      <div className="sb-sticker-grid">
        {loading && <div className="sb-sticker-hint">加载中…</div>}
        {!loading && hits.length === 0 && (
          <div className="sb-sticker-hint">没找到「{query}」</div>
        )}
        {hits.map((n) => (
          <StickerCell
            key={n}
            name={n}
            color={props.color}
            active={props.pick.kind === 'svg' && props.pick.value === n}
            onPick={() => props.onPick({ kind: 'svg', value: n })}
          />
        ))}
      </div>
      {hits.length >= limit && (
        <button className="sb-sticker-more" onClick={() => setLimit((l: number) => l + 120)}>
          显示更多…
        </button>
      )}
      </div>
      )}
    </div>
  )
}

/** 单个图标格：把 SVG 原文内联进来，这样能跟着选中色实时换色。 */
function StickerCell(props: {
  name: string
  color: string
  active: boolean
  onPick: () => void
}) {
  const [svg, setSvg] = useState('')
  useEffect(() => {
    let alive = true
    loadStickerSvg(props.name)
      .then((t) => {
        if (alive) setSvg(recolorSvg(t, props.color))
      })
      .catch(() => {})
    return () => {
      alive = false
    }
  }, [props.name, props.color])

  return (
    <button
      className={`sb-ic ${props.active ? 'on' : ''}`}
      title={`${zhLabel(props.name)}  ${props.name}`}
      onClick={props.onPick}
      // SVG 原文来自我们自己的包（public/stickers），不是用户输入，
      // 不存在注入风险；innerHTML 是让"换色"生效的唯一方式（<img> 不吃 CSS 颜色）。
      dangerouslySetInnerHTML={{ __html: svg }}
    />
  )
}

interface Props {
  tool: AnnotateTool
  color: Rgba
  width: number
  fontSize: number
  /** 聚光灯压暗强度（0-255） */
  spotDim: number
  mosaicBlock: number
  /** 橡皮擦半径（逻辑像素） */
  eraserRadius: number
  /** 放大镜倍数 */
  zoomLevel: number
  onColor: (v: Rgba) => void
  onWidth: (v: number) => void
  onFontSize: (v: number) => void
  onSpotDim: (v: number) => void
  onMosaicBlock: (v: number) => void
  onEraserRadius: (v: number) => void
  onZoomLevel: (v: number) => void
  /** 当前贴纸素材（emoji 字形 or svg 图标名） */
  sticker: StickerPick
  onSticker: (s: StickerPick) => void
  /** 贴纸颜色（对 SVG 生效） */
  stickerColor: string
  onStickerColor: (c: string) => void
  /** 贴纸显示尺寸（逻辑像素） */
  stickerSize: number
  onStickerSize: (v: number) => void
  /** 画笔 ↔ 荧光笔子模式切换 */
  onTool: (t: AnnotateTool) => void
}

/** 数值滑条：左侧量名（可选）+ 滑条 + 右侧实时数值。
 *
 *  滑条本身没有刻度，不给数字就是"盲调"；`tabular-nums` + 固定最小宽度
 *  保证拖动时数字变化不会把面板宽度撑得左右跳。 */
function Slider(props: {
  label?: string
  title: string
  min: number
  max: number
  value: number
  /** 数值右侧显示的单位后缀（如 px） */
  suffix?: string
  /** 自定义显示文本（如把 0-255 的暗度显示成百分比） */
  format?: (v: number) => string
  onChange: (v: number) => void
  /** 紧凑模式：不显示左侧量名（单行面板里放不下） */
  compact?: boolean
}) {
  const { label, title, min, max, value, suffix = '', format, onChange } = props
  return (
    <label className={`sb-width${props.compact ? ' compact' : ''}`} title={title}>
      {label && !props.compact && <span className="sb-cap">{label}</span>}
      <input
        type="range"
        min={min}
        max={max}
        step={1}
        value={value}
        onChange={(e) => onChange(Number(e.target.value))}
      />
      <output className="sb-val">{format ? format(value) : `${value}${suffix}`}</output>
    </label>
  )
}

export function ToolProps(p: Props) {
  const { tool } = p

  if (tool === 'mosaic') {
    return (
      <div className="sb-seg">
        {MOSAIC_BLOCKS.map((b, i) => (
          <button key={b} className={p.mosaicBlock === b ? 'on' : ''} onClick={() => p.onMosaicBlock(b)}>
            {['小', '中', '大'][i]}
          </button>
        ))}
      </div>
    )
  }

  // 聚光灯不选颜色（永远是黑色压暗），只调强度。
  // 底层值 0-255 对用户没有意义，显示成百分比。
  if (tool === 'spotlight') {
    return (
      <Slider
        label="暗度"
        title={`压暗强度 ${Math.round((p.spotDim / 255) * 100)}%`}
        min={SPOTLIGHT_DIM.min}
        max={SPOTLIGHT_DIM.max}
        value={p.spotDim}
        format={(v) => `${Math.round((v / 255) * 100)}%`}
        onChange={p.onSpotDim}
      />
    )
  }

  // 橡皮擦：只调半径（不涉及颜色——它还原的是原图像素）。
  // 必须给操作提示：它的语义是"擦掉已画的标注"，不是"擦掉截图内容"，
  // 不给提示用户会以为工具坏了（按住拖过空白处当然什么都不会变）。
  if (tool === 'eraser') {
    return (
      <>
        <Slider
          label="笔尖"
          title={`擦除半径 ${p.eraserRadius}px`}
          min={ERASER_RADIUS.min}
          max={ERASER_RADIUS.max}
          value={p.eraserRadius}
          suffix="px"
          onChange={p.onEraserRadius}
        />
        <div className="sb-tip">按住拖过已画的标注即可擦除（不动原图）</div>
      </>
    )
  }

  // 放大镜：倍数档位（与 Rust 最近邻取样一致的离散倍率）
  if (tool === 'zoom') {
    return (
      <div className="sb-seg">
        {ZOOM_LEVELS.map((z) => (
          <button key={z} className={p.zoomLevel === z ? 'on' : ''} onClick={() => p.onZoomLevel(z)}>
            {z}×
          </button>
        ))}
      </div>
    )
  }

  // 智能擦除无参数：给一句操作提示，避免面板空着让人以为没反应
  if (tool === 'smartErase') {
    return <div className="sb-tip">拖选要抹掉的区域，自动用周围画面补全</div>
  }

  // 取色器无参数：提示交互方式（按住移动看色，松手复制）
  if (tool === 'pick') {
    return <div className="sb-tip">按住移动取色，松开复制 HEX 到剪贴板</div>
  }

  // 二维码无参数：提示先拖区域
  if (tool === 'qr') {
    return <div className="sb-tip">拖选要识别的区域，支持一图多码</div>
  }

  // 贴纸面板（单行二级条）：
  //   16 个 emoji 全平铺 → 颜色下拉 → 大小下拉 → 搜索框
  if (tool === 'sticker') {
    return <StickerPanel {...p} />
  }
  const isTextLike = tool === 'text' || tool === 'badge' || tool === 'watermark'

  return (
    <>
      {(tool === 'pen' || tool === 'highlighter') && (
        <div className="sb-seg">
          <button className={tool === 'pen' ? 'on' : ''} onClick={() => p.onTool('pen')}>
            画笔
          </button>
          <button
            className={tool === 'highlighter' ? 'on' : ''}
            onClick={() => p.onTool('highlighter')}
          >
            荧光笔
          </button>
        </div>
      )}
      <div className="sb-colors">
        {PRESET_COLORS.map((c) => (
          <button
            key={c.join(',')}
            className={`swatch ${p.color === c ? 'active' : ''}`}
            style={{ background: `rgb(${c[0]},${c[1]},${c[2]})` }}
            onClick={() => p.onColor(c)}
            title={`颜色 rgba(${c.join(',')})`}
          />
        ))}
      </div>
      <i className="sb-sep" />
      {isTextLike ? (
        <Slider
          title={`${tool === 'badge' ? '序号大小' : '字号'} ${p.fontSize}px`}
          min={TEXT_FONT_SIZE.min}
          max={TEXT_FONT_SIZE.max}
          value={p.fontSize}
          suffix="px"
          onChange={p.onFontSize}
        />
      ) : (
        <Slider
          title={`线宽 ${p.width}px`}
          min={1}
          max={24}
          value={p.width}
          suffix="px"
          onChange={p.onWidth}
        />
      )}
      {/* 水印的交互藏在"拖选后才弹输入框"里，不给提示用户不知道下一步 */}
      {tool === 'watermark' && <div className="sb-tip">拖选区域后输入水印文字</div>}
    </>
  )
}
