import { useEffect, useState } from 'react'
import { listen } from '@tauri-apps/api/event'
import { closeSelf, imgSrc, type LongProgress } from '../lib/api'
import '../styles/longpreview.css'

/**
 * 长截图实时缩略预览窗（独立小窗）。
 *
 * 为什么独立成窗而不是塞在状态条里：长截图时页面在自己滚动，用户的视线跟着
 * 内容走，状态条贴在选区边上很容易被滚出视野/被别的窗口盖住。这个小窗
 * 常驻置顶，能一直看到"已经拼到多长"。
 *
 * 数据来自 Rust 的 `long-capture-progress` 事件（缩略图是临时 PNG 路径），
 * 本窗只读不驱动，关掉不影响拼接。
 */
export function LongPreviewWindow() {
  const [p, setP] = useState<LongProgress | null>(null)

  useEffect(() => {
    let unlisten: (() => void) | undefined
    let alive = true
    listen<LongProgress>('long-capture-progress', (ev) => {
      if (alive) setP(ev.payload)
    })
      .then((f) => {
        unlisten = f
      })
      .catch(() => {})
    return () => {
      alive = false
      unlisten?.()
    }
  }, [])

  const finished = p?.phase === 'done' || p?.phase === 'error'

  return (
    <main className="lp-root" data-tauri-drag-region>
      <header className="lp-bar" data-tauri-drag-region>
        <span className="lp-title">长截图预览</span>
        <span className={`lp-badge ${finished ? 'done' : ''}`}>
          {finished ? (p?.phase === 'done' ? '已完成' : '已中止') : '拼接中'}
        </span>
        <button
          className="lp-close"
          title="关闭预览（不影响拼接）"
          onClick={() => void closeSelf()}
        >
          ✕
        </button>
      </header>

      <div className="lp-shot">
        {p?.thumb ? (
          <img src={imgSrc(p.thumb)} alt="拼接预览" draggable={false} />
        ) : (
          <span className="lp-empty">等待第一帧…</span>
        )}
      </div>

      <footer className="lp-foot">
        <span className="lp-h">{p ? `${p.height} px` : '—'}</span>
        <span className="lp-steps">{p ? `${p.steps} 步` : ''}</span>
      </footer>
      {p?.message && <p className="lp-msg">{p.message}</p>}
    </main>
  )
}
