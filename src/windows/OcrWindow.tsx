import { useCallback, useEffect, useState } from 'react'
import {
  copyText,
  imgSrc,
  ocrClose,
  ocrResult,
  onOcrOpen,
  transStart,
  uiLog,
  type OcrLine,
} from '../lib/api'
import '../styles/textwin.css'

/** `ocr_result` 的返回体：识别行 + 原图路径（左栏用）。 */
interface OcrPayload {
  lines: OcrLine[]
  image: string | null
}

/**
 * OCR 结果窗（独立窗口）。
 *
 * 布局 1:1 对标参考图：**左图右文**——左栏显示裁剪出的原图，右栏是可选中
 * 纯文本。一行一个块级元素，元素里只有文字本身（行号/置信度/每行复制按钮
 * 全部撤掉：曾经那套让框选选到一堆杂质，用户原话"一条一条复制太麻烦了"）。
 * 置信度改到 hover 提示里，不占版面。
 *
 * 数据不在 URL 上传递（长文本 + 编码坑），而是 Rust 侧存一份、窗口挂载 /
 * 收到 `ocr-open` 事件时来取，因此重复识别时已开着的窗口内容会跟着刷新。
 */
export function OcrWindow() {
  const [data, setData] = useState<OcrPayload | null>(null)
  const [copied, setCopied] = useState(false)
  /** 左栏图片加载失败（文件被清理等）时显示占位提示 */
  const [imgBroken, setImgBroken] = useState(false)

  const reload = useCallback(() => {
    setImgBroken(false)
    ocrResult()
      .then(setData)
      .catch(() => setData({ lines: [], image: null }))
  }, [])

  useEffect(() => {
    uiLog('ocr mounted')
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

  const lines = data?.lines ?? null
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
      {/* 顶栏：标题 + 行数 + 关闭。整条兼作窗口拖拽区。 */}
      <header className="tw-bar" data-tauri-drag-region>
        <div className="tw-title">
          文字识别
          {lines && lines.length > 0 && <span className="tw-count">{lines.length} 行</span>}
        </div>
        <button className="tw-btn ghost" onClick={() => ocrClose().catch(() => {})}>
          关闭
        </button>
      </header>

      {/* 双栏：左原图 / 右文本 */}
      <div className="ocr-panes">
        <section className="ocr-img-pane">
          {data?.image && !imgBroken ? (
            <img
              className="ocr-img"
              src={imgSrc(data.image)}
              alt="识别原图"
              draggable={false}
              onError={() => setImgBroken(true)}
            />
          ) : (
            <div className="tw-empty">
              {imgBroken ? '原图文件已被清理，重新识别即可' : lines === null ? '读取中…' : '没有原图'}
            </div>
          )}
        </section>

        <section className="ocr-text-pane">
          {lines === null && <div className="tw-empty">读取中…</div>}
          {lines !== null && lines.length === 0 && (
            <div className="tw-empty">没有识别结果。重新截图识别后再来。</div>
          )}
          {lines && lines.length > 0 && (
            <div className="ocr-text-area">
              {lines.map((l, i) => (
                <div
                  className="ocr-para"
                  key={`${i}-${l.minY}-${l.minX}`}
                  title={`置信度 ${(l.score * 100).toFixed(0)}%`}
                >
                  {l.text}
                </div>
              ))}
            </div>
          )}
        </section>
      </div>

      {/* 底栏：参考图同款的底部图标按钮组 + 左侧操作提示 */}
      <footer className="tw-footbar">
        <span className="tw-hint">拖动鼠标选中任意内容，Ctrl+C 复制</span>
        <div className="tw-foot-actions">
          <button className="tw-act" onClick={copyAll} disabled={!text}>
            <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.7">
              <rect x="9" y="9" width="11" height="11" rx="2" />
              <path d="M5 15V5a2 2 0 0 1 2-2h10" />
            </svg>
            {copied ? '已复制' : '复制全文'}
          </button>
          <button className="tw-act" onClick={translateAll} disabled={!text}>
            <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.7">
              <path d="M4 5h9M8.5 5c0 4-2 7-4.5 8.5M6 9c1.5 3 4 5 6.5 6" />
              <path d="M13 20l4-9 4 9M14.4 17.5h5.2" />
            </svg>
            翻译
          </button>
        </div>
      </footer>
    </main>
  )
}
