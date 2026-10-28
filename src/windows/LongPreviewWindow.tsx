import { useEffect, useState } from 'react'
import { listen } from '@tauri-apps/api/event'
import { Check, X } from 'lucide-react'
import {
  cancelCapture,
  imgSrc,
  longCaptureStop,
  longPreviewInfo,
  type LongProgress,
} from '../lib/api'
import '../styles/longpreview.css'

export function LongPreviewWindow() {
  const [progress, setProgress] = useState<LongProgress | null>(null)
  const [busy, setBusy] = useState(false)

  useEffect(() => {
    let alive = true
    let unlisten: (() => void) | undefined
    void listen<LongProgress>('long-capture-progress', (event) => {
      if (alive) setProgress(event.payload)
    }).then((fn) => {
      if (alive) unlisten = fn
      else fn()
    })
    void longPreviewInfo().then((value) => {
      if (alive) setProgress((current) => current ?? value)
    })
    return () => {
      alive = false
      unlisten?.()
    }
  }, [])

  const finish = async () => {
    if (busy) return
    setBusy(true)
    try {
      await longCaptureStop()
    } catch {
      setBusy(false)
    }
  }

  const cancel = async () => {
    if (busy) return
    setBusy(true)
    try {
      await cancelCapture()
    } catch {
      setBusy(false)
    }
  }

  return (
    <main className="lp-root">
      <section className="lp-panel">
        <header className="lp-header">
          <div className="lp-status">
            <span className="lp-dot" />
            <span>{progress ? `${progress.height} px` : '准备中'}</span>
          </div>
          <div className="lp-actions">
            <button type="button" title="取消长截图" aria-label="取消长截图" disabled={busy} onClick={cancel}>
              <X size={16} strokeWidth={2} />
            </button>
            <button type="button" className="lp-finish" title="完成长截图" aria-label="完成长截图" disabled={busy} onClick={finish}>
              <Check size={17} strokeWidth={2.4} />
            </button>
          </div>
        </header>
        {progress?.thumb && (
          <img
            className="lp-image"
            src={`${imgSrc(progress.thumb)}?step=${progress.steps}`}
            alt="长截图实时预览"
            draggable={false}
          />
        )}
        {progress?.message.includes('找不到重叠') && (
          <div className="lp-warning">滚动过快，请向上滚回已截区域</div>
        )}
      </section>
    </main>
  )
}
