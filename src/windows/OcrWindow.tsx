import { useCallback, useEffect, useState } from 'react'
import {
  copyText,
  ocrClose,
  ocrResult,
  onOcrOpen,
  transStart,
  uiLog,
  type OcrLine,
} from '../lib/api'
import '../styles/textwin.css'

/**
 * OCR 结果窗（独立窗口）。
 *
 * 需求对应：识别后弹出新窗口 → 右侧文本可拖动选择 → 「全部复制」→ 顶部「翻译」。
 * 数据不在 URL 上传递（长文本 + 编码坑），而是 Rust 侧存一份、窗口挂载 /
 * 收到 `ocr-open` 事件时来取，因此重复识别时已开着的窗口内容会跟着刷新。
 */
export function OcrWindow() {
  const [lines, setLines] = useState<OcrLine[] | null>(null)
  const [copied, setCopied] = useState(false)

  const reload = useCallback(() => {
    ocrResult()
      .then(setLines)
      .catch(() => setLines([]))
  }, [])

  useEffect(() => {
    reload()
    // 重复识别 → Rust 广播 ocr-open，这里重取内容
    let off: (() => void) | null = null
    onOcrOpen(reload)
      .then((f) => {
        off = f
      })
      .catch(() => {})
    return () => off?.()
  }, [reload])

  const text = (lines ?? []).map((l) => l.text).join('\n')

  const copyAll = () => {
    if (!text) return
    copyText(text)
      .then(() => {
        setCopied(true)
        setTimeout(() => setCopied(false), 1500)
      })
      .catch((e) => uiLog(`ocr copy failed: ${String(e)}`))
  }

  const translateAll = () => {
    if (!text) return
    // 翻译窗自己会打开；这里不 await 结果（网络请求可能几秒）
    transStart(text).catch((e) => uiLog(`trans start failed: ${String(e)}`))
  }

  return (
    <main className="tw-root">
      <header className="tw-bar">
        <div className="tw-title">
          文字识别
          {lines && lines.length > 0 && <span className="tw-count">{lines.length} 行</span>}
        </div>
        <div className="tw-actions">
          <button className="tw-btn" onClick={translateAll} disabled={!text}>
            <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8">
              <path d="M4 5h9M8.5 5c0 4-2 7-4.5 8.5M6 9c1.5 3 4 5 6.5 6" />
              <path d="M13 20l4-9 4 9M14.4 17.5h5.2" />
            </svg>
            翻译
          </button>
          <button className="tw-btn" onClick={copyAll} disabled={!text}>
            {copied ? '已复制' : '全部复制'}
          </button>
          <button className="tw-btn ghost" onClick={() => ocrClose().catch(() => {})}>
            关闭
          </button>
        </div>
      </header>

      <div className="tw-body">
        {lines === null && <div className="tw-empty">读取中…</div>}
        {lines !== null && lines.length === 0 && (
          <div className="tw-empty">没有识别结果。重新截图识别后再来。</div>
        )}
        {lines && lines.length > 0 && (
          <div className="ocr-lines">
            {lines.map((l, i) => (
              <div className="ocr-line" key={`${i}-${l.minY}-${l.minX}`}>
                <span className="ocr-idx">{i + 1}</span>
                {/* 关键：可选中。user-select + 不拦截 pointerdown，浏览器原生选区 */}
                <span className="ocr-text">{l.text}</span>
                <button
                  className="ocr-copy"
                  title="复制这一行"
                  onClick={() => copyText(l.text).catch(() => {})}
                >
                  复制
                </button>
              </div>
            ))}
          </div>
        )}
      </div>

      <footer className="tw-foot">
        提示：直接用鼠标拖动即可选中任意内容，Ctrl+C 复制；「翻译」会把全文送进翻译窗。
      </footer>
    </main>
  )
}
