/**
 * 长截图结果窗（独立窗口，`longresult.html`）。
 *
 * 为什么从浮层里搬出来：原来结果面板挂在**全屏浮层**上，只要面板还在，
 * 就一直是"截图模式"——桌面点不动、必须按 Esc 才退出。用户明确要求
 * "结果展示出来 + 退出截图模式"，所以拆成普通窗口：
 * 浮层关掉（回到正常桌面），结果留在这个窗里，保存/复制/贴图/关闭都在这里。
 *
 * 图片默认「适应宽度」显示，可切到「1:1」看原始像素（超长图靠滚动查看）。
 */
import { useEffect, useState } from 'react'
import {
  imgSrc,
  longCaptureCommit,
  longResultClose,
  longResultInfo,
  type LongResultInfo,
  type ShotAction,
} from '../lib/api'
import '../styles/longresult.css'

export function LongResultWindow() {
  const [info, setInfo] = useState<LongResultInfo | null>(null)
  const [err, setErr] = useState<string | null>(null)
  const [busy, setBusy] = useState<ShotAction | null>(null)
  /** true = 适应窗口宽度；false = 原始像素 1:1（默认） */
  const [fit, setFit] = useState(false)

  useEffect(() => {
    let alive = true
    longResultInfo()
      .then((v) => {
        if (alive) setInfo(v)
      })
      .catch((e) => {
        if (alive) setErr(String(e))
      })
    return () => {
      alive = false
    }
  }, [])

  const act = async (a: ShotAction) => {
    if (busy) return
    setBusy(a)
    setErr(null)
    try {
      await longCaptureCommit(a)
      // commit 已经消费了会话（图已产出），窗口没有继续存在的意义
      await longResultClose()
    } catch (e) {
      setErr(String(e))
      setBusy(null)
    }
  }

  const close = () => {
    longResultClose().catch(() => {})
  }

  return (
    <main className="lr-root">
      <header className="lr-bar">
        <div className="lr-title">
          长截图
          {info && (
            <span className="lr-chip">
              {info.width} × {info.height} px
            </span>
          )}
          {info && <span className="lr-chip">{info.steps} 步拼接</span>}
        </div>
        <div className="lr-actions">
          <button className="lr-btn ghost" onClick={() => setFit((v) => !v)} disabled={!info}>
            {fit ? '原始大小' : '适应窗口'}
          </button>
          <button className="lr-btn ghost" onClick={close} title="关闭（丢弃这张长图）">
            关闭
          </button>
        </div>
      </header>

      {err && (
        <div className="lr-error">
          <span>{err}</span>
          <button className="lr-btn" onClick={() => setErr(null)}>
            知道了
          </button>
        </div>
      )}

      <div className={`lr-stage${fit ? ' is-fit' : ' is-actual'}`}>
        {info ? (
          <img className="lr-img" src={imgSrc(info.path)} alt="" draggable={false} />
        ) : (
          <div className="lr-empty">{err ? '结果不可用' : '正在准备图片…'}</div>
        )}
      </div>

      <footer className="lr-foot">
        <span className="lr-hint">
          可以直接拖动图片/滚动查看；「保存」会弹出系统另存为对话框。
        </span>
        <div className="lr-actions">
          <button className="lr-btn" onClick={() => act('save')} disabled={!info || !!busy}>
            {busy === 'save' ? '保存中…' : '保存'}
          </button>
          <button className="lr-btn" onClick={() => act('copy')} disabled={!info || !!busy}>
            {busy === 'copy' ? '复制中…' : '复制'}
          </button>
          <button className="lr-btn primary" onClick={() => act('pin')} disabled={!info || !!busy}>
            {busy === 'pin' ? '贴图中…' : '贴图'}
          </button>
        </div>
      </footer>
    </main>
  )
}
