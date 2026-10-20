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

/** 标注工具种类（与 Rust ShapePayload 的 tag 一一对应）。 */
export type AnnotateTool =
  | 'rect'
  | 'ellipse'
  | 'arrow'
  | 'pen'
  | 'highlighter'
  | 'mosaic'

/** 已确认的标注（含工具判别字段，序列化后直传 confirm_selection）。 */
export type ShapeDraft =
  | { type: 'rect'; x: number; y: number; w: number; h: number; color: Rgba; width: number }
  | { type: 'ellipse'; x: number; y: number; w: number; h: number; color: Rgba; width: number }
  | { type: 'arrow'; x1: number; y1: number; x2: number; y2: number; color: Rgba; width: number }
  | { type: 'pen'; points: [number, number][]; color: Rgba; width: number }
  | { type: 'highlighter'; points: [number, number][]; color: Rgba; width: number }
  | { type: 'mosaic'; x: number; y: number; w: number; h: number; block: number }

export interface ToolMeta {
  id: AnnotateTool
  label: string
  /** 工具条按钮字形（unicode 符号，贴近主流截图工具观感） */
  glyph: string
  /** 是否需要按住拖动画轨迹（否则是拖矩形） */
  trajectory: boolean
  /** 默认线宽（逻辑像素） */
  defaultWidth: number
}

export const TOOLS: ToolMeta[] = [
  { id: 'rect', label: '矩形', glyph: '□', trajectory: false, defaultWidth: 2 },
  { id: 'ellipse', label: '椭圆', glyph: '○', trajectory: false, defaultWidth: 2 },
  { id: 'arrow', label: '箭头', glyph: '↗', trajectory: false, defaultWidth: 2 },
  { id: 'pen', label: '铅笔', glyph: '✎', trajectory: true, defaultWidth: 2 },
  { id: 'highlighter', label: '荧光笔', glyph: '▬', trajectory: true, defaultWidth: 14 },
  { id: 'mosaic', label: '马赛克', glyph: '▦', trajectory: false, defaultWidth: 12 },
]

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

export function toolMeta(id: AnnotateTool): ToolMeta {
  return TOOLS.find((t) => t.id === id) ?? TOOLS[0]
}

/** 把 CSS 像素坐标序列约束为两位小数（减小 IPC 体积）。
 *  逻辑像素坐标在传到 Rust 前统一过此函数。 */
export function quantize(v: number): number {
  return Math.round(v * 100) / 100
}
