/**
 * SVG 贴纸素材库（Bootstrap Icons，2083 个）。
 *
 * 装载方式：全部图标由 `scripts/gen-stickers.mjs` 在构建前打成**一个**
 * `public/stickers.json`（{ 名字: svg原文 }，约 2.3MB），首次用到时整包 fetch
 * 一次，之后全是内存查表。
 *
 * 为什么不放 2079 个散文件（最初的做法，已推翻）：
 *   1. Vite 每次 build 都要 emptyOutDir —— 删 2000+ 个文件又慢又脆，
 *      在受控环境里直接因"批量删除"被拦，第二次构建起必失败（实测踩到）；
 *   2. 2079 个散文件 = 2079 次请求才能铺出网格，首帧卡；
 *   3. 整包查表后切颜色/换图标都是本地操作，零延迟。
 *
 * 与 emoji 贴纸的关系：两者最终都走「光栅化成 PNG → Shape::Bitmap」通道
 * （见 AnnotationLayer.rasterizeSticker），Rust 侧零改动。
 */

import { CN_SYNONYMS, zhLabel } from './zh'

export type StickerKind = 'emoji' | 'svg'

/** 当前选中的贴纸素材：emoji 存字形，svg 存图标名 */
export interface StickerPick {
  kind: StickerKind
  value: string
}

/** 整包缓存：{ 图标名: svg 原文 }。null = 还没加载。 */
let bundleCache: Record<string, string> | null = null

/** 加载整包（只加载一次）。失败会抛，由调用方提示。 */
async function loadBundle(): Promise<Record<string, string>> {
  if (bundleCache) return bundleCache
  const base = import.meta.env.BASE_URL || '/'
  const res = await fetch(`${base}stickers.json`)
  if (!res.ok) throw new Error(`stickers bundle HTTP ${res.status}`)
  bundleCache = (await res.json()) as Record<string, string>
  return bundleCache
}

/** 图标名清单（整包的 key，已排序）。 */
export async function loadStickerManifest(): Promise<string[]> {
  const b = await loadBundle()
  return Object.keys(b)
}

/**
 * 按关键词过滤图标名：**中英双语**。
 *
 * 英文名本身（`arrow-up-right`）+ 中文名（`箭头上右`，见 zh.ts）都参与匹配，
 * 用户输"箭头""arrow""对勾""check" 任一种都能搜到。
 */
export function filterStickerNames(names: string[], query: string): string[] {
  const raw = query.trim()
  if (!raw) return names
  const q = raw.toLowerCase()
  // 中文同义词展开：用户搜"设置"时去匹配 gear/sliders/tools
  const synonyms = CN_SYNONYMS[raw] ?? []
  const out: string[] = []
  for (const n of names) {
    if (n.includes(q) || zhLabel(n).includes(raw)) {
      out.push(n)
      continue
    }
    if (synonyms.some((w) => n.includes(w))) out.push(n)
  }
  return out
}

/** 取一个图标的 SVG 原文（整包内存查表，无 IO）。 */
export async function loadStickerSvg(name: string): Promise<string> {
  const b = await loadBundle()
  const svg = b[name]
  if (!svg) throw new Error(`sticker not found: ${name}`)
  return svg
}

/**
 * 把 SVG 的填充/描边改成指定颜色。
 *
 * Bootstrap Icons 的做法是根节点 `fill="currentColor"`、子节点不带 fill
 * （靠继承）——所以改根节点就够。少数写了死颜色的（几个品牌 logo）逐节点覆盖。
 * 多色图标会被拍成单色，这是**预期行为**：调用方想要原色就别调这个函数。
 */
export function recolorSvg(svgText: string, color: string): string {
  const doc = new DOMParser().parseFromString(svgText, 'image/svg+xml')
  if (doc.documentElement.nodeName === 'parsererror') return svgText
  const root = doc.documentElement
  root.setAttribute('fill', color)
  root.removeAttribute('class')
  for (const el of Array.from(root.querySelectorAll('*'))) {
    if (el.hasAttribute('fill')) el.setAttribute('fill', color)
    if (el.hasAttribute('stroke')) el.setAttribute('stroke', color)
    if (el.hasAttribute('stop-color')) el.setAttribute('stop-color', color)
  }
  return new XMLSerializer().serializeToString(root)
}

/**
 * 把 SVG 光栅化成 PNG（**物理像素**），返回与 rasterizeSticker 相同的结构。
 *
 * 为什么要走 Blob → Image 而不是把 SVG 文本塞进 data URL 直接 drawImage：
 * 两者都行，但 Blob 方式对超长 SVG 更稳（data URL 有长度上限，且部分平台
 * 对特殊字符转义挑剔）。SVG 是同源的 Blob，画进 canvas **不会污染画布**。
 */
export async function rasterizeSvgSticker(
  svgText: string,
  sizePx: number,
  dpr: number,
): Promise<{ png: string; w: number; h: number; canvas: HTMLCanvasElement } | null> {
  const scale = dpr > 0 ? dpr : 1
  const side = Math.max(8, Math.round(sizePx * scale))
  const url = URL.createObjectURL(new Blob([svgText], { type: 'image/svg+xml' }))
  try {
    const img = new Image()
    const ok = await new Promise<boolean>((resolve) => {
      img.onload = () => resolve(true)
      img.onerror = () => resolve(false)
      img.src = url
    })
    if (!ok || !img.naturalWidth) return null
    const canvas = document.createElement('canvas')
    canvas.width = side
    canvas.height = side
    const ctx = canvas.getContext('2d')
    if (!ctx) return null
    ctx.drawImage(img, 0, 0, side, side)
    const data = canvas.toDataURL('image/png')
    const png = data.slice(data.indexOf(',') + 1)
    if (!png) return null
    // canvas 必须一起返回：`Shape::Bitmap` 的渲染走
    // `ctx.drawImage(s.canvas, ...)`（见 AnnotationLayer.drawShape 的 bitmap 分支），
    // 只给 png 的话贴纸在画布上根本不显示（之前就栽在这个上面，日志显示
    // 光栅成功但画面空白）。
    return { png, w: side / scale, h: side / scale, canvas }
  } finally {
    // Blob URL 必须释放：用户点几百个图标就是不释放几百份
    URL.revokeObjectURL(url)
  }
}
