import { useCallback, useEffect, useState } from 'react'
import {
  imgSrc,
  pinClose,
  pinFocus,
  pinList,
  pinSetGroup,
  uiLog,
  type PinInfo,
} from '../lib/api'
import '../styles/pinmanager.css'

/**
 * 贴图管理面板（跨窗口）。
 *
 * 为什么需要它：贴图是**各自独立的置顶窗口**，贴多了之后想找某一张、想批量关掉、
 * 想看清哪些属于同一组，光靠桌面上去点很容易被别的窗口盖住看不见。
 * 这里把所有贴图做成缩略图列表，点一下就置前。
 *
 * 分组本身（指派 / 只看某组 / 隐藏某组 / 显示全部）在**贴图右键菜单**里，
 * 本面板只做"看 + 置前 + 关闭 + 改组名"，避免两处 UI 语义打架。
 */
export function PinManagerWindow() {
  const [pins, setPins] = useState<PinInfo[] | null>(null)
  const [busy, setBusy] = useState<string>('')

  const reload = useCallback(() => {
    pinList()
      .then(setPins)
      .catch((e) => uiLog(`pin list failed: ${String(e)}`))
  }, [])

  useEffect(() => {
    reload()
    // 贴图窗口增减没有事件推送，面板显式刷新即可（打开面板/操作后各刷一次）
  }, [reload])

  const focus = (label: string) => {
    setBusy(label)
    pinFocus(label)
      .catch((e) => uiLog(`pin focus failed: ${String(e)}`))
      .finally(() => setBusy(''))
  }

  const close = (label: string) => {
    setBusy(label)
    pinClose(label)
      .then(() => reload())
      .catch((e) => uiLog(`pin close failed: ${String(e)}`))
      .finally(() => setBusy(''))
  }

  const setGroup = (label: string, group: string) => {
    pinSetGroup(label, group)
      .then(() => reload())
      .catch((e) => uiLog(`pin set group failed: ${String(e)}`))
  }

  const list = pins ?? []

  return (
    <main className="pm-root">
      <header className="pm-bar">
        <span className="pm-title">贴图管理</span>
        <span className="pm-count">{pins === null ? '读取中…' : `${list.length} 张`}</span>
        <button className="pm-btn" onClick={reload} title="刷新列表">
          刷新
        </button>
      </header>

      {pins !== null && list.length === 0 && (
        <p className="pm-empty">
          还没有贴图。
          <br />
          截图后点工具条的「贴图」，或按 Ctrl+2 把剪贴板图片贴到屏幕上。
        </p>
      )}

      <div className="pm-list">
        {list.map((p) => (
          <div className="pm-item" key={p.label}>
            <button
              className="pm-thumb"
              title={`置前（${p.label}）`}
              disabled={busy === p.label}
              onClick={() => focus(p.label)}
            >
              {p.src ? <img src={imgSrc(p.src)} alt="" draggable={false} /> : null}
            </button>
            <div className="pm-meta">
              <span className="pm-size">
                {Math.round(p.width)} × {Math.round(p.height)}
              </span>
              <input
                className="pm-group"
                value={p.group || '默认'}
                title="分组名（改完立刻生效，右键菜单里可按组显示/隐藏）"
                onChange={(e) => setGroup(p.label, e.target.value)}
                onBlur={reload}
              />
            </div>
            <div className="pm-actions">
              <button className="pm-btn" onClick={() => focus(p.label)}>
                置前
              </button>
              <button className="pm-btn danger" onClick={() => close(p.label)}>
                关闭
              </button>
            </div>
          </div>
        ))}
      </div>
    </main>
  )
}
