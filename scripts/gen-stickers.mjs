#!/usr/bin/env node
/**
 * 把 assets/stickers/ 下的 2079 个 SVG 打成一个 JSON 包（public/stickers.json）。
 *
 * 为什么必须打成一个文件（而不是让 public/ 原样带 2079 个散文件）：
 *   1. Vite 每次 build 都要 emptyOutDir —— 删 2000+ 个文件又慢又脆
 *      （在部分受控环境里直接因"批量删除"被拦，构建第二次起必失败，实测踩到）；
 *   2. 2079 个散文件 = 2079 次 HTTP 请求才能铺出图标网格，首帧明显卡；
 *   3. 打包后一次性 fetch、内存查表，切颜色/换图标都是本地操作，零延迟。
 *
 * 产物格式：{ "图标名": "<svg>原文</svg>", ... }（约 2.4MB）。
 * 前端只认这一个文件，见 src/stickers/library.ts。
 *
 * 用法：npm run gen:stickers（已挂到 prebuild，构建前自动跑）
 */
import { readdirSync, readFileSync, writeFileSync, mkdirSync, existsSync, rmSync } from 'node:fs'
import { join, dirname } from 'node:path'
import { fileURLToPath } from 'node:url'

const root = join(dirname(fileURLToPath(import.meta.url)), '..')
const srcDir = join(root, 'assets', 'stickers')
const outDir = join(root, 'public')
const outFile = join(outDir, 'stickers.json')

if (!existsSync(srcDir)) {
  console.error(`[stickers] 源目录不存在：${srcDir}`)
  process.exit(1)
}

const names = readdirSync(srcDir)
  .filter((f) => f.toLowerCase().endsWith('.svg'))
  .map((f) => f.slice(0, -4))
  .sort()

const bundle = {}
let bytes = 0
for (const n of names) {
  const text = readFileSync(join(srcDir, `${n}.svg`), 'utf8')
  bundle[n] = text
  bytes += text.length
}

mkdirSync(outDir, { recursive: true })
writeFileSync(outFile, JSON.stringify(bundle), 'utf8')

// 清掉上一版的散文件目录（如果还在）—— 留着会让 vite 又去删 2000 个文件
const legacy = join(outDir, 'stickers')
if (existsSync(legacy)) rmSync(legacy, { recursive: true, force: true })

console.log(
  `[stickers] ${names.length} 个图标 → public/stickers.json（${(bytes / 1024 / 1024).toFixed(2)} MB）`,
)
