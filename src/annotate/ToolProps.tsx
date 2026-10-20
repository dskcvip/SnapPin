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
  MOSAIC_BLOCKS,
  PRESET_COLORS,
  SPOTLIGHT_DIM,
  TEXT_FONT_SIZE,
  type AnnotateTool,
  type Rgba,
} from './tools'

interface Props {
  tool: AnnotateTool
  color: Rgba
  width: number
  fontSize: number
  /** 聚光灯压暗强度（0-255） */
  spotDim: number
  mosaicBlock: number
  onColor: (v: Rgba) => void
  onWidth: (v: number) => void
  onFontSize: (v: number) => void
  onSpotDim: (v: number) => void
  onMosaicBlock: (v: number) => void
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
}) {
  const { label, title, min, max, value, suffix = '', format, onChange } = props
  return (
    <label className="sb-width" title={title}>
      {label && <span className="sb-cap">{label}</span>}
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

  const isTextLike = tool === 'text' || tool === 'badge'

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
    </>
  )
}
