/**
 * M3 标注工具：类型定义与工具元数据。
 *
 * 坐标约定（与 Rust 契约对齐）：
 * - 所有 shape 坐标均为**逻辑像素**，原点 = 裁剪图左上角（即选区左上角）；
 * - 颜色为 RGBA 四元组（0-255）；
 * - 前端 Canvas 预览与 Rust 光栅化（core-algo::draw）共用同一份数据，
 *   Rust 侧统一乘 DPI 缩放因子转物理像素。
 */

export type Rgba = [number, number, number, number]

/** 标注工具种类（与 Rust ShapePayload 的 tag 一一对应）。
 *
 *  后两类是**动作类**工具（`pick` / `qr`）：不产生标注图形，只做一次
 *  取色或识别，因此没有对应的 ShapeDraft / ShapePayload 变体。 */
export type AnnotateTool =
  | 'rect'
  | 'ellipse'
  | 'arrow'
  | 'pen'
  | 'highlighter'
  | 'mosaic'
  | 'spotlight'
  | 'text'
  | 'badge'
  | 'eraser'
  | 'watermark'
  | 'zoom'
  | 'smartErase'
  | 'pick'
  | 'qr'

/** 已确认的标注（含工具判别字段，序列化后直传 confirm_selection）。 */
export type ShapeDraft =
  | { type: 'rect'; x: number; y: number; w: number; h: number; color: Rgba; width: number }
  | { type: 'ellipse'; x: number; y: number; w: number; h: number; color: Rgba; width: number }
  | { type: 'arrow'; x1: number; y1: number; x2: number; y2: number; color: Rgba; width: number }
  | { type: 'pen'; points: [number, number][]; color: Rgba; width: number }
  | { type: 'highlighter'; points: [number, number][]; color: Rgba; width: number }
  | { type: 'mosaic'; x: number; y: number; w: number; h: number; block: number }
  /** 聚光灯：区域外压暗。`radius` 圆角半径、`dim` 压暗强度（0-255），
   *  两者均为逻辑像素/无量纲，与框选出的 x/y/w/h 一起发给 Rust。 */
  | {
      type: 'spotlight'
      x: number
      y: number
      w: number
      h: number
      radius: number
      dim: number
    }
  | {
      /** 文字（UI 层光栅化后的位图，见 rasterizeText）。
       *  `x`/`y`/`w`/`h` 为逻辑像素；`png` 为 base64 PNG（物理像素，直传 Rust）；
       *  `canvas` 仅用于本地预览重绘，**发送前必须剥离**（DOM 节点不可序列化）。 */
      type: 'bitmap'
      x: number
      y: number
      w: number
      h: number
      png: string
      canvas?: HTMLCanvasElement
      /** 序列号标注的编号（1、2、3…）。仅为本地状态：用于推算下一个号
       *  以及撤销后重新编号，**发送前剥离**（Rust 只认 x/y/png）。 */
      badge?: number
    }
  /** 橡皮擦：轨迹区域还原成原图（擦掉已画上去的标注）。
   *  `radius` 为擦除半径（逻辑像素），轨迹点同。 */
  | { type: 'eraser'; points: [number, number][]; radius: number }
  /** 水印：单块文字位图在 (x,y,w,h) 区域内按 (stepX,stepY) 平铺。
   *  `png` 为 UI 层光栅化的**单块**水印（物理像素，直传 Rust）；
   *  `canvas`/`tileW`/`tileH` 仅用于本地预览，**发送前剥离**。 */
  | {
      type: 'watermark'
      x: number
      y: number
      w: number
      h: number
      stepX: number
      stepY: number
      png: string
      canvas?: HTMLCanvasElement
      /** 单块水印的逻辑像素尺寸 */
      tileW: number
      tileH: number
    }
  /** 区域放大（放大镜）：把原图内容按 `zoom` 倍放大画回同一区域，
   *  `color`/`width` 为放大区域边框。 */
  | {
      type: 'zoom'
      x: number
      y: number
      w: number
      h: number
      zoom: number
      color: Rgba
      width: number
    }
  /** 智能擦除：内容感知填充（Rust 侧零依赖波前扩散）。
   *  `iterations` 为 0 表示按区域尺寸自动收敛。 */
  | { type: 'smartErase'; x: number; y: number; w: number; h: number; iterations: number }

export interface ToolMeta {
  id: AnnotateTool
  label: string
  /** 工具条按钮字形（unicode 符号，贴近主流截图工具观感） */
  glyph: string
  /** 是否需要按住拖动画轨迹（否则是拖矩形） */
  trajectory: boolean
  /** 默认线宽（逻辑像素） */
  defaultWidth: number
  /** `shape` = 画标注（产生 ShapeDraft）；`action` = 一次性动作，不产生图形。
   *  动作类工具不进属性面板的线宽/颜色逻辑，也不需要发送给 Rust。 */
  kind: 'shape' | 'action'
}

/** 动作类工具集合（取色 / 二维码识别）。 */
export const ACTION_TOOLS: ReadonlySet<AnnotateTool> = new Set<AnnotateTool>(['pick', 'qr'])

export function isActionTool(t: AnnotateTool): boolean {
  return ACTION_TOOLS.has(t)
}

export const TOOLS: ToolMeta[] = [
  { id: 'rect', label: '矩形', glyph: '□', trajectory: false, defaultWidth: 2, kind: 'shape' },
  { id: 'ellipse', label: '椭圆', glyph: '○', trajectory: false, defaultWidth: 2, kind: 'shape' },
  { id: 'arrow', label: '箭头', glyph: '↗', trajectory: false, defaultWidth: 2, kind: 'shape' },
  { id: 'pen', label: '铅笔', glyph: '✎', trajectory: true, defaultWidth: 2, kind: 'shape' },
  {
    id: 'highlighter',
    label: '荧光笔',
    glyph: '▬',
    trajectory: true,
    defaultWidth: 14,
    kind: 'shape',
  },
  { id: 'mosaic', label: '马赛克', glyph: '▦', trajectory: false, defaultWidth: 12, kind: 'shape' },
  // 聚光灯不用线宽（defaultWidth 记的是圆角半径，见 SPOTLIGHT_RADIUS）
  {
    id: 'spotlight',
    label: '聚光灯',
    glyph: '◎',
    trajectory: false,
    defaultWidth: 12,
    kind: 'shape',
  },
  // 文字不走"拖拽成形"，defaultWidth 对它是**字号**（见 TEXT_FONT_SIZE）
  { id: 'text', label: '文字', glyph: 'T', trajectory: false, defaultWidth: 16, kind: 'shape' },
  // 序号：单击落点盖一个编号圆点，与文字共用字号（决定圆直径）
  { id: 'badge', label: '序号', glyph: '①', trajectory: false, defaultWidth: 16, kind: 'shape' },
  // 橡皮擦是轨迹类；defaultWidth 记的是**擦除半径**（见 ERASER_RADIUS）
  {
    id: 'eraser',
    label: '橡皮擦',
    glyph: '⌫',
    trajectory: true,
    defaultWidth: 16,
    kind: 'shape',
  },
  // 水印：先拖区域，再在弹出的输入框里写文字（文字光栅化成平铺块）
  {
    id: 'watermark',
    label: '水印',
    glyph: '◫',
    trajectory: false,
    defaultWidth: 16,
    kind: 'shape',
  },
  // 放大镜：拖区域即放大；defaultWidth 记的是边框线宽
  { id: 'zoom', label: '放大镜', glyph: '🔍', trajectory: false, defaultWidth: 2, kind: 'shape' },
  // 智能擦除：拖区域即内容感知填充，无参数
  {
    id: 'smartErase',
    label: '智能擦除',
    glyph: '✥',
    trajectory: false,
    defaultWidth: 0,
    kind: 'shape',
  },
  // 取色器：动作类。在选区内移动即取色，松开复制 HEX
  { id: 'pick', label: '取色器', glyph: '⬗', trajectory: false, defaultWidth: 2, kind: 'action' },
  // 二维码识别：动作类。拖选区域后调 Rust 识别（支持一图多码）
  { id: 'qr', label: '二维码', glyph: '▤', trajectory: false, defaultWidth: 2, kind: 'action' },
]

/** 文字字体栈：优先系统 UI 字体，再回退到三大平台的中文字体。
 *  不能用内置字体——中文/emoji 只有系统字体才全，且字体文件不该进安装包。 */
export const TEXT_FONT_STACK =
  'system-ui, "Segoe UI", "Microsoft YaHei", "PingFang SC", "Hiragino Sans GB", "Noto Sans CJK SC", sans-serif'

/** 字号范围与默认值（逻辑像素）。 */
export const TEXT_FONT_SIZE = { min: 10, max: 72, default: 16 }
/** 行高倍数：与 textarea 的 line-height 必须一致，否则输入框和最终渲染会错位。 */
export const TEXT_LINE_HEIGHT = 1.32

/** 序号徽标：圆直径 = 字号 × 该倍数（单字时的下限）。
 *  两位数/三位数会按文字宽度自适应放大，保证数字不贴边。 */
export const BADGE_DIAMETER_RATIO = 1.9

/** 发送给 Rust 前剥离本地字段（canvas 无法序列化，留着只会变成 `{}`；
 *  `badge` 是编号状态，Rust 不需要）。Tauri 的 IPC 会把对象 JSON 化，
 *  把 Canvas 元素混进去虽然不致命，但会给 Rust 侧载荷增加无意义字段。 */
export function toShapePayloads(shapes: ShapeDraft[]): unknown[] {
  return shapes.map((s) => {
    if (s.type === 'bitmap') {
      const { canvas: _canvas, badge: _badge, ...rest } = s
      return rest
    }
    if (s.type === 'watermark') {
      // 水印的 canvas（预览重绘用）与 tile 尺寸（Rust 从 PNG 自解码）都剥离
      const { canvas: _canvas, tileW: _tileW, tileH: _tileH, ...rest } = s
      return rest
    }
    return s
  })
}

/** 下一个序号：取现有编号最大值 + 1。
 *  用"扫描现有标注"而不是递增计数器，是为了让撤销天然安全
 *  —— 删掉 3 之后，下一个仍然是 3，不会跳号。 */
export function nextBadgeNo(shapes: ShapeDraft[]): number {
  let max = 0
  for (const s of shapes) {
    if (s.type === 'bitmap' && typeof s.badge === 'number' && s.badge > max) max = s.badge
  }
  return max + 1
}

/** 工具条预设颜色（含荧光笔专用半透明黄）。 */
export const PRESET_COLORS: Rgba[] = [
  [230, 45, 45, 255], // 红
  [255, 150, 40, 255], // 橙
  [250, 204, 21, 255], // 黄
  [34, 197, 94, 255], // 绿
  [59, 130, 246, 255], // 蓝
  [255, 255, 255, 255], // 白
  [15, 23, 42, 255], // 黑
]

/** 荧光笔的固定外观：半透明黄。 */
export const HIGHLIGHTER_COLOR: Rgba = [250, 204, 21, 90]

/** 马赛克块边长档位（逻辑像素）。 */
export const MOSAIC_BLOCKS = [6, 12, 20]

/** 聚光灯压暗强度（0-255 的 RGB 压暗量）：默认 150 ≈ 压掉 59%，与主流
 *  截图工具的聚光灯观感接近；下限留 40 保证"能看出压过"，上限 230 留一点
 *  底图轮廓，不给到 255（全黑就变成纯遮挡了）。 */
export const SPOTLIGHT_DIM = { min: 40, max: 230, default: 150 }

/** 聚光灯圆角半径（逻辑像素），随 DPR 转物理像素。 */
export const SPOTLIGHT_RADIUS = 12

/** 橡皮擦半径（逻辑像素）。下限 4 保证"看得见在擦"，上限 48 避免
 *  一眨眼擦掉半张图（撤销成本高，feedback 优先保守）。 */
export const ERASER_RADIUS = { min: 4, max: 48, default: 16 }

/** 放大镜倍数档位（逻辑倍率，Rust 侧最近邻取样，与前端预览一致）。 */
export const ZOOM_LEVELS = [1.5, 2, 3, 4, 6]

/** 水印平铺步进 = 单块尺寸 + 固定间隙（逻辑像素）。
 *  横向间隙小于纵向：文字水印按行读，行距大一点更像"背景水印"而不是网格。 */
export const WATERMARK_GAP = { x: 14, y: 22 }

/** 水印输入框的预填文字：用户直接改写即可，省去"先想写什么"的一步。 */
export const WATERMARK_DEFAULT_TEXT = 'SnapPin'

export function toolMeta(id: AnnotateTool): ToolMeta {
  return TOOLS.find((t) => t.id === id) ?? TOOLS[0]
}

/** 把 CSS 像素坐标序列约束为两位小数（减小 IPC 体积）。
 *  逻辑像素坐标在传到 Rust 前统一过此函数。 */
export function quantize(v: number): number {
  return Math.round(v * 100) / 100
}
