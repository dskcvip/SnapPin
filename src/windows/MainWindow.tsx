import { useEffect, useState } from 'react'
import {
  appInfo,
  ballCommand,
  getConfig,
  pinFromClipboard,
  startCapture,
  updateConfig,
  type AppConfig,
  type AppInfo,
} from '../lib/api'
import '../styles/main.css'

const DEFAULT_CONFIG: AppConfig = {
  themeColor: '#12A5D8',
  ballSize: 56,
  ballPosition: null,
  shortcutCapture: 'Ctrl+1',
  shortcutPinClipboard: 'Ctrl+2',
  saveDir: null,
}

/** 主窗口：动作入口 + 基础配置。 */
export function MainWindow() {
  const [info, setInfo] = useState<AppInfo | null>(null)
  const [cfg, setCfg] = useState<AppConfig>(DEFAULT_CONFIG)
  const [dirty, setDirty] = useState(false)

  useEffect(() => {
    appInfo().then(setInfo).catch(() => setInfo(null))
    getConfig().then(setCfg).catch(() => {})
  }, [])

  const patch = (p: Partial<AppConfig>) => {
    setCfg((c) => ({ ...c, ...p }))
    setDirty(true)
  }

  const save = () => {
    updateConfig(cfg)
      .then((c) => {
        setCfg(c)
        setDirty(false)
      })
      .catch(() => {})
  }

  return (
    <main className="main-root">
      <header className="main-head">
        <h1>SnapPin</h1>
        <span className="main-version">v{info?.version ?? '-'} · {info?.platform ?? '-'}</span>
      </header>

      <section className="main-actions">
        <button className="mbtn primary" onClick={() => startCapture().catch(() => {})}>
          开始截图
        </button>
        <button className="mbtn" onClick={() => pinFromClipboard().catch(() => {})}>
          剪贴板贴图
        </button>
        <button className="mbtn ghost" onClick={() => ballCommand('quit').catch(() => {})}>
          退出
        </button>
      </section>

      <section className="main-keys">
        <h2>快捷键</h2>
        <ul>
          <li>
            <kbd>Ctrl</kbd>+<kbd>1</kbd> 截图
          </li>
          <li>
            <kbd>Ctrl</kbd>+<kbd>2</kbd> 剪贴板贴图
          </li>
          <li>
            <kbd>Enter</kbd> 复制选区 · <kbd>Esc</kbd> 取消
          </li>
          <li>
            <kbd>Ctrl</kbd>+滚轮 缩放贴图
          </li>
        </ul>
      </section>

      <section className="main-config">
        <h2>外观</h2>
        <div className="field">
          <label htmlFor="theme">主题色</label>
          <input
            id="theme"
            type="color"
            value={cfg.themeColor}
            onChange={(e) => patch({ themeColor: e.target.value })}
          />
        </div>
        <div className="field">
          <label htmlFor="ball-size">悬浮球尺寸 {cfg.ballSize}px</label>
          <input
            id="ball-size"
            type="range"
            min={40}
            max={80}
            step={2}
            value={cfg.ballSize}
            onChange={(e) => patch({ ballSize: Number(e.target.value) })}
          />
        </div>
        <button className="mbtn" disabled={!dirty} onClick={save}>
          {dirty ? '保存设置' : '已保存'}
        </button>
      </section>
    </main>
  )
}
