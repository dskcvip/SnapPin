import { useCallback, useEffect, useRef, useState } from 'react'
import {
  copyText,
  onTransUpdate,
  transClose,
  transStart,
  transState,
  uiLog,
  type TransState,
} from '../lib/api'
import '../styles/textwin.css'

/**
 * 翻译窗（独立窗口，左原文 / 右译文）。
 *
 * 需求对应：截图内容新弹窗口，左侧原文右侧译文，两边都能拖动选择，也能全文选择。
 * 「全文选择」用 DOM Selection API 选中整个面板（比让用户手动拖长文靠谱）。
 *
 * 状态来自 Rust（trans_state + trans-update 事件）：翻译是阻塞线程里的网络
 * 请求，结果推过来即刷新，不轮询。
 */
export function TranslationWindow() {
  const [st, setSt] = useState<TransState | null>(null)
  const [copied, setCopied] = useState<'' | 'src' | 'dst'>('')
  const srcRef = useRef<HTMLDivElement>(null)
  const dstRef = useRef<HTMLDivElement>(null)

  const reload = useCallback(() => {
    transState().then(setSt).catch(() => {})
  }, [])

  useEffect(() => {
    reload()
    let off: (() => void) | null = null
    onTransUpdate(setSt)
      .then((f) => {
        off = f
      })
      .catch(() => {})
    return () => off?.()
  }, [reload])

  /** 全文选择：选中整个面板的文本（原生 Selection，选完可直接 Ctrl+C）。 */
  const selectAllIn = (el: HTMLElement | null) => {
    if (!el) return
    const sel = window.getSelection()
    if (!sel) return
    const range = document.createRange()
    range.selectNodeContents(el)
    sel.removeAllRanges()
    sel.addRange(range)
  }

  const copy = (which: 'src' | 'dst', text: string) => {
    if (!text) return
    copyText(text)
      .then(() => {
        setCopied(which)
        setTimeout(() => setCopied(''), 1500)
      })
      .catch((e) => uiLog(`copy failed: ${String(e)}`))
  }

  const retry = () => {
    if (st?.original) transStart(st.original).catch(() => {})
  }

  const hasSrc = !!st?.original
  const hasDst = !!st?.translated

  return (
    <main className="tw-root trans-root">
      <header className="tw-bar">
        <div className="tw-title">
          翻译
          {st?.targetLang && <span className="tw-count">→ {st.targetLang}</span>}
          {st?.modelName && <span className="tw-count">{st.modelName}</span>}
        </div>
        <div className="tw-actions">
          <button className="tw-btn" onClick={retry} disabled={!hasSrc || st?.busy}>
            {st?.busy ? '翻译中…' : '重新翻译'}
          </button>
          <button className="tw-btn" onClick={() => selectAllIn(srcRef.current)} disabled={!hasSrc}>
            全选原文
          </button>
          <button className="tw-btn" onClick={() => selectAllIn(dstRef.current)} disabled={!hasDst}>
            全选译文
          </button>
          <button className="tw-btn" onClick={() => copy('dst', st?.translated ?? '')} disabled={!hasDst}>
            {copied === 'dst' ? '已复制' : '复制译文'}
          </button>
          <button className="tw-btn ghost" onClick={() => transClose().catch(() => {})}>
            关闭
          </button>
        </div>
      </header>

      {st?.error && (
        <div className="tw-error">
          {st.error}
          <button className="tw-btn" onClick={retry} style={{ marginLeft: 10 }}>
            重试
          </button>
        </div>
      )}

      <div className="trans-panes">
        <section className="trans-pane">
          <div className="pane-head">
            原文
            <button className="pane-btn" onClick={() => copy('src', st?.original ?? '')} disabled={!hasSrc}>
              {copied === 'src' ? '已复制' : '复制'}
            </button>
          </div>
          {/* 可拖动选择：user-select:text，不拦截指针事件 */}
          <div className="pane-body" ref={srcRef}>
            {st?.original || <span className="tw-empty">（还没有内容）</span>}
          </div>
        </section>
        <section className="trans-pane">
          <div className="pane-head">
            译文
            {st?.busy && <span className="pane-spinner" />}
            <button className="pane-btn" onClick={() => copy('dst', st?.translated ?? '')} disabled={!hasDst}>
              {copied === 'dst' ? '已复制' : '复制'}
            </button>
          </div>
          <div className="pane-body" ref={dstRef}>
            {st?.translated || (
              <span className="tw-empty">{st?.busy ? '翻译中…' : '（还没有译文）'}</span>
            )}
          </div>
        </section>
      </div>

      <footer className="tw-foot">
        两边都可以直接用鼠标拖动选择，Ctrl+C 复制；也可以点「全选」一次选完整栏。
      </footer>
    </main>
  )
}
