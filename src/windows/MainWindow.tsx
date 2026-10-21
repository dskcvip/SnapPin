import { useEffect, useMemo, useState } from 'react'
import {
  appInfo,
  ballCommand,
  getConfig,
  openUrl,
  pinFromClipboard,
  startCapture,
  transTest,
  updateConfig,
  type AppConfig,
  type AppInfo,
  type TranslationConfig,
} from '../lib/api'
import '../styles/main.css'

/** 左侧导航的一项。 */
interface NavItem {
  id: string
  label: string
  /** 导航图标（内联 SVG，16px，currentColor） */
  icon: React.JSX.Element
}

const ICONS = {
  general: (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8">
      <circle cx="12" cy="12" r="3" />
      <path d="M19.4 15a1.7 1.7 0 0 0 .34 1.87l.06.06a2 2 0 1 1-2.83 2.83l-.06-.06a1.7 1.7 0 0 0-1.87-.34 1.7 1.7 0 0 0-1.03 1.56V21a2 2 0 1 1-4 0v-.09a1.7 1.7 0 0 0-1.11-1.56 1.7 1.7 0 0 0-1.87.34l-.06.06a2 2 0 1 1-2.83-2.83l.06-.06A1.7 1.7 0 0 0 4.6 15a1.7 1.7 0 0 0-1.56-1.03H3a2 2 0 1 1 0-4h.09A1.7 1.7 0 0 0 4.65 8.86a1.7 1.7 0 0 0-.34-1.87l-.06-.06a2 2 0 1 1 2.83-2.83l.06.06A1.7 1.7 0 0 0 9 4.6a1.7 1.7 0 0 0 1.03-1.56V3a2 2 0 1 1 4 0v.09a1.7 1.7 0 0 0 1.03 1.56 1.7 1.7 0 0 0 1.87-.34l.06-.06a2 2 0 1 1 2.83 2.83l-.06.06a1.7 1.7 0 0 0-.34 1.87v0a1.7 1.7 0 0 0 1.56 1.03H21a2 2 0 1 1 0 4h-.09a1.7 1.7 0 0 0-1.51 1.03z" />
    </svg>
  ),
  keys: (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8">
      <rect x="2" y="6" width="20" height="12" rx="2" />
      <path d="M6 10h.01M10 10h.01M14 10h.01M18 10h.01M8 14h8" />
    </svg>
  ),
  model: (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8">
      <path d="M12 2 3 7v10l9 5 9-5V7z" />
      <path d="M3 7l9 5 9-5M12 22V12" />
    </svg>
  ),
  data: (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8">
      <ellipse cx="12" cy="5" rx="8" ry="3" />
      <path d="M4 5v7c0 1.7 3.6 3 8 3s8-1.3 8-3V5M4 12v7c0 1.7 3.6 3 8 3s8-1.3 8-3v-7" />
    </svg>
  ),
  pencil: (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8">
      <path d="M17 3a2.8 2.8 0 0 1 4 4L7.5 20.5 2 22l1.5-5.5z" />
    </svg>
  ),
  link: (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8">
      <path d="M10 13a5 5 0 0 0 7.5.5l3-3a5 5 0 0 0-7-7l-1.7 1.7" />
      <path d="M14 11a5 5 0 0 0-7.5-.5l-3 3a5 5 0 0 0 7 7l1.7-1.7" />
    </svg>
  ),
  trash: (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8">
      <path d="M3 6h18M8 6V4h8v2M19 6l-1 14H6L5 6M10 11v6M14 11v6" />
    </svg>
  ),
}

const NAV_GROUPS: { title: string; items: NavItem[] }[] = [
  {
    title: '设置',
    items: [
      { id: 'general', label: '通用', icon: ICONS.general },
      { id: 'keys', label: '快捷键', icon: ICONS.keys },
      { id: 'models', label: '模型', icon: ICONS.model },
    ],
  },
  {
    title: '数据与安全',
    items: [{ id: 'data', label: '数据管理', icon: ICONS.data }],
  },
]

/** 快捷键行：可改的两条走录制，其余展示固定值。 */
interface KeyRow {
  cmd: string
  /** 配置字段名（可改项）；undefined = 固定不可改 */
  field?: 'shortcutCapture' | 'shortcutPinClipboard'
  /** 固定说明文案（不可改项） */
  fixed?: string
}

const KEY_ROWS: KeyRow[] = [
  { cmd: '开始截图', field: 'shortcutCapture' },
  { cmd: '剪贴板贴图', field: 'shortcutPinClipboard' },
  { cmd: '复制选区 / 确认', fixed: 'Enter' },
  { cmd: '取消截图 / 关闭贴图', fixed: 'Esc' },
  { cmd: '缩放贴图', fixed: 'Ctrl+滚轮' },
  { cmd: '调整贴图不透明度', fixed: 'Alt+滚轮' },
  { cmd: '选区取色', fixed: 'C' },
]

/** 「添加模型」的快捷预设（点一下填好表单，只剩 Key 要手填）。 */
const MODEL_PRESETS: {
  name: string
  note: string
  baseUrl: string
  model: string
  keyUrl: string
}[] = [
  {
    name: 'DeepSeek',
    note: '深度求索',
    baseUrl: 'https://api.deepseek.com',
    model: 'deepseek-flash',
    keyUrl: 'https://platform.deepseek.com/api_keys',
  },
  {
    name: 'Kimi',
    note: 'Moonshot 月之暗面',
    baseUrl: 'https://api.moonshot.cn',
    model: 'kimi-k2-0905-preview',
    keyUrl: 'https://platform.moonshot.cn/console/api-keys',
  },
  {
    name: '智谱 GLM',
    note: 'BigModel 开放平台',
    baseUrl: 'https://open.bigmodel.cn/api/paas/v4',
    model: 'glm-4-flash',
    keyUrl: 'https://open.bigmodel.cn/usercenter/apikeys',
  },
  {
    name: '通义千问',
    note: '阿里云百炼',
    baseUrl: 'https://dashscope.aliyuncs.com/compatible-mode/v1',
    model: 'qwen-flash',
    keyUrl: 'https://bailian.console.aliyun.com/#/api-key',
  },
  {
    name: '自定义',
    note: '任意 OpenAI 兼容端点',
    baseUrl: '',
    model: '',
    keyUrl: '',
  },
]

const DEFAULT_CONFIG: AppConfig = {
  themeColor: '#12A5D8',
  ballSize: 56,
  ballPosition: null,
  shortcutCapture: 'Ctrl+1',
  shortcutPinClipboard: 'Ctrl+2',
  saveDir: null,
  translation: {
    models: [],
    targetLang: '英文',
  },
}

/** 生成模型 id（时间戳 + 随机，够用且不需要引 uuid 库）。 */
function newModelId(): string {
  return `m${Date.now().toString(36)}${Math.random().toString(36).slice(2, 6)}`
}

/** 把键盘事件格式化成 "Ctrl+Shift+A"（与 Rust parse_shortcut 的双向协议）。 */
function formatHotkey(e: KeyboardEvent): string | null {
  const parts: string[] = []
  if (e.ctrlKey) parts.push('Ctrl')
  if (e.shiftKey) parts.push('Shift')
  if (e.altKey) parts.push('Alt')
  if (e.metaKey) parts.push('Super')
  let key: string | null = null
  if (/^[a-z]$/i.test(e.key)) key = e.key.toUpperCase()
  else if (/^[0-9]$/.test(e.key)) key = e.key
  else if (/^F([1-9]|1[0-2])$/.test(e.key)) key = e.key.toUpperCase()
  if (!key) return null
  parts.push(key)
  // 纯主键不算组合键（用户单独按一个字母时不该触发）
  return parts.length > 1 ? parts.join('+') : null
}

export function MainWindow() {
  const [info, setInfo] = useState<AppInfo | null>(null)
  const [cfg, setCfg] = useState<AppConfig>(DEFAULT_CONFIG)
  const [dirty, setDirty] = useState(false)
  const [page, setPage] = useState('general')
  const [navQuery, setNavQuery] = useState('')
  const [toast, setToast] = useState('')

  // 快捷键录制中（字段名 → 等待用户按键）
  const [recording, setRecording] = useState<string | null>(null)

  // 模型编辑弹层：null = 关闭；'new' = 新建；否则是模型 id
  const [editing, setEditing] = useState<string | null>(null)
  const [draft, setDraft] = useState<TranslationConfig['models'][number] | null>(null)
  const [testState, setTestState] = useState<{ id: string; ok: boolean; msg: string } | null>(null)
  const [testing, setTesting] = useState(false)

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
        setToast('已保存')
        setTimeout(() => setToast(''), 1600)
      })
      .catch((e) => setToast(`保存失败：${String(e)}`))
  }

  // 快捷键录制：录制期间拦截键盘，拿到组合键即写入配置草稿
  useEffect(() => {
    if (!recording) return
    const onKey = (e: KeyboardEvent) => {
      e.preventDefault()
      if (e.key === 'Escape') {
        setRecording(null)
        return
      }
      const combo = formatHotkey(e)
      if (!combo) return
      patch(
        recording === 'shortcutCapture'
          ? { shortcutCapture: combo }
          : { shortcutPinClipboard: combo },
      )
      setRecording(null)
    }
    window.addEventListener('keydown', onKey, true)
    return () => window.removeEventListener('keydown', onKey, true)
    // patch 每次渲染都是新函数，这里只依赖 recording
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [recording])

  const groups = useMemo(() => {
    const q = navQuery.trim()
    if (!q) return NAV_GROUPS
    return NAV_GROUPS.map((g) => ({
      ...g,
      items: g.items.filter((it) => it.label.includes(q)),
    })).filter((g) => g.items.length > 0)
  }, [navQuery])

  const openEditor = (id: string | 'new') => {
    if (id === 'new') {
      setDraft({
        id: newModelId(),
        name: '',
        note: '',
        baseUrl: 'https://api.deepseek.com',
        apiKey: '',
        model: 'deepseek-flash',
        keyUrl: 'https://platform.deepseek.com/api_keys',
      })
    } else {
      const found = cfg.translation.models.find((m) => m.id === id)
      if (found) setDraft({ ...found })
    }
    setEditing(id)
    setTestState(null)
  }

  const commitDraft = () => {
    if (!draft) return
    const models = [...cfg.translation.models]
    const idx = models.findIndex((m) => m.id === draft.id)
    if (idx >= 0) models[idx] = draft
    else models.push(draft)
    patch({ translation: { ...cfg.translation, models } })
    setEditing(null)
    setDraft(null)
  }

  const removeModel = (id: string) => {
    patch({
      translation: {
        ...cfg.translation,
        models: cfg.translation.models.filter((m) => m.id !== id),
      },
    })
  }

  const runTest = async (id: string) => {
    setTesting(true)
    setTestState(null)
    try {
      await transTest(id)
      setTestState({ id, ok: true, msg: '连接成功：端点与 API Key 均可用' })
    } catch (e) {
      setTestState({ id, ok: false, msg: String(e) })
    } finally {
      setTesting(false)
    }
  }

  const activeId = cfg.translation.models[0]?.id

  return (
    <main className="set-root">
      {/* ---------------- 左侧导航 ---------------- */}
      <nav className="set-nav">
        <div className="set-search">
          <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
            <circle cx="11" cy="11" r="7" />
            <path d="m20 20-3.5-3.5" />
          </svg>
          <input
            value={navQuery}
            onChange={(e) => setNavQuery(e.target.value)}
            placeholder="搜索设置项"
          />
        </div>
        {groups.map((g) => (
          <div key={g.title}>
            <div className="set-group">{g.title}</div>
            {g.items.map((it) => (
              <button
                key={it.id}
                className={`set-item ${page === it.id ? 'active' : ''}`}
                onClick={() => setPage(it.id)}
              >
                <span className="ic">{it.icon}</span>
                {it.label}
              </button>
            ))}
          </div>
        ))}
      </nav>

      {/* ---------------- 右侧内容 ---------------- */}
      <div className="set-main">
        {page === 'general' && (
          <>
            <div className="set-page-head">
              <div>
                <h2>通用</h2>
                <p>外观、悬浮球与翻译目标语言等常规项。</p>
              </div>
            </div>
            <div className="set-card">
              <div className="field">
                <label htmlFor="theme">主题色</label>
                <input
                  id="theme"
                  type="color"
                  value={cfg.themeColor}
                  onChange={(e) => patch({ themeColor: e.target.value })}
                />
                <span className="hint">悬浮球 / 菜单 / 选中色</span>
              </div>
              <div className="field">
                <label htmlFor="ball-size">悬浮球尺寸</label>
                <input
                  id="ball-size"
                  type="range"
                  min={40}
                  max={80}
                  step={2}
                  value={cfg.ballSize}
                  onChange={(e) => patch({ ballSize: Number(e.target.value) })}
                />
                <span className="hint">{cfg.ballSize}px</span>
              </div>
              <div className="field">
                <label htmlFor="target-lang">翻译目标语言</label>
                <input
                  id="target-lang"
                  type="text"
                  value={cfg.translation.targetLang}
                  onChange={(e) =>
                    patch({
                      translation: { ...cfg.translation, targetLang: e.target.value },
                    })
                  }
                  placeholder="英文"
                />
                <span className="hint">写进提示词，如 英文 / 日文 / 繁体中文</span>
              </div>
            </div>

            <div className="set-card" style={{ marginTop: 16 }}>
              <div className="set-page-head" style={{ marginBottom: 10 }}>
                <div>
                  <h2>动作</h2>
                  <p>常用入口，和托盘菜单一致。</p>
                </div>
              </div>
              <div className="btn-row" style={{ marginTop: 0 }}>
                <button className="btn primary" onClick={() => startCapture().catch(() => {})}>
                  开始截图
                </button>
                <button className="btn" onClick={() => pinFromClipboard().catch(() => {})}>
                  剪贴板贴图
                </button>
                <button className="btn" onClick={() => ballCommand('quit').catch(() => {})}>
                  退出
                </button>
              </div>
            </div>
          </>
        )}

        {page === 'keys' && (
          <>
            <div className="set-page-head">
              <div>
                <h2>快捷键</h2>
                <p>点击可修改的按键绑定，直接按下新的组合键即可；Esc 放弃录制。</p>
              </div>
              <button
                className="btn"
                onClick={() => {
                  patch({
                    shortcutCapture: 'Ctrl+1',
                    shortcutPinClipboard: 'Ctrl+2',
                  })
                  setToast('已恢复默认快捷键，记得保存')
                }}
              >
                全部恢复默认
              </button>
            </div>
            <div className="set-card">
              <table className="set-table">
                <thead>
                  <tr>
                    <th>命令</th>
                    <th>按键绑定</th>
                    <th style={{ textAlign: 'right' }}>操作</th>
                  </tr>
                </thead>
                <tbody>
                  {KEY_ROWS.map((row) => {
                    const value = row.field ? cfg[row.field] : undefined
                    const isRec = recording === row.field
                    return (
                      <tr key={row.cmd}>
                        <td className="cmd">{row.cmd}</td>
                        <td>
                          {row.field ? (
                            <button
                              className={`key-pill ${isRec ? 'editing' : ''}`}
                              onClick={() => setRecording(isRec ? null : row.field!)}
                            >
                              {isRec ? '按下新组合键…' : value}
                            </button>
                          ) : (
                            <span className="key-fixed">{row.fixed}</span>
                          )}
                        </td>
                        <td className="ops">
                          {row.field && (
                            <button
                              className="icon-btn danger"
                              title="恢复默认"
                              onClick={() =>
                                patch(
                                  row.field === 'shortcutCapture'
                                    ? { shortcutCapture: 'Ctrl+1' }
                                    : { shortcutPinClipboard: 'Ctrl+2' },
                                )
                              }
                            >
                              <span className="ic" style={{ width: 15, height: 15 }}>
                                {ICONS.trash}
                              </span>
                            </button>
                          )}
                        </td>
                      </tr>
                    )
                  })}
                </tbody>
              </table>
            </div>
          </>
        )}

        {page === 'models' && (
          <>
            <div className="set-page-head">
              <div>
                <h2>自定义模型</h2>
                <p>
                  模型添加后会写入本地配置文件（%APPDATA%\com.snappin.dev\config.json），
                  API Key 只存在本机。列表第一个为当前启用的模型。
                </p>
              </div>
              <button className="btn" onClick={() => openEditor('new')}>
                添加模型
              </button>
            </div>
            <div className="set-card">
              {cfg.translation.models.length === 0 && (
                <p style={{ color: '#9aa0ab', fontSize: 12, margin: '4px 0' }}>
                  还没有模型。点右上角「添加模型」，或从预设里选一个，只剩 API Key 需要你填。
                </p>
              )}
              {cfg.translation.models.map((m) => (
                <div className="model-row" key={m.id}>
                  <div className="model-avatar">{(m.name || '?').slice(0, 1)}</div>
                  <div className="model-info">
                    <div className="model-name">
                      {m.name || '（未命名）'}
                      {m.id === activeId && <span className="model-badge">使用中</span>}
                    </div>
                    <div className="model-note">
                      {[m.note, m.model].filter(Boolean).join(' · ') || m.baseUrl}
                    </div>
                  </div>
                  <div className="model-ops">
                    <button
                      className="test-btn test-pill"
                      disabled={testing}
                      onClick={() => runTest(m.id)}
                    >
                      {testing ? '测试中…' : '测试连接'}
                    </button>
                    <button
                      className="icon-btn"
                      title="编辑"
                      onClick={() => openEditor(m.id)}
                    >
                      <span className="ic" style={{ width: 15, height: 15 }}>
                        {ICONS.pencil}
                      </span>
                    </button>
                    <button
                      className="icon-btn"
                      title="获取 API Key 的网页"
                      disabled={!m.keyUrl}
                      onClick={() => m.keyUrl && openUrl(m.keyUrl).catch(() => {})}
                    >
                      <span className="ic" style={{ width: 15, height: 15 }}>
                        {ICONS.link}
                      </span>
                    </button>
                    <button
                      className="icon-btn danger"
                      title="删除"
                      onClick={() => removeModel(m.id)}
                    >
                      <span className="ic" style={{ width: 15, height: 15 }}>
                        {ICONS.trash}
                      </span>
                    </button>
                  </div>
                </div>
              ))}
              {testState && !testing && (
                <div className={`test-result ${testState.ok ? 'ok' : 'bad'}`}>
                  {testState.msg}
                </div>
              )}
              {cfg.translation.models.length > 1 && (
                <p style={{ color: '#9aa0ab', fontSize: 12, margin: '12px 0 0' }}>
                  想切换启用模型：把更常用的那个删掉再重新添加，它就会排到最前面（列表第一个 =
                  当前启用）。
                </p>
              )}
            </div>
          </>
        )}

        {page === 'data' && (
          <>
            <div className="set-page-head">
              <div>
                <h2>数据管理</h2>
                <p>配置、日志与 OCR 模型的落点。删了配置会回默认值，不影响使用。</p>
              </div>
            </div>
            <div className="set-card">
              <div className="data-row">
                <div>
                  <div className="t">配置文件</div>
                  <div className="d">
                    Windows %APPDATA%\com.snappin.dev\config.json · macOS
                    ~/Library/Application Support/com.snappin.dev/config.json
                  </div>
                </div>
                <span className="path-box">config.json</span>
              </div>
              <div className="data-row">
                <div>
                  <div className="t">运行日志</div>
                  <div className="d">snappin.log，排障唯一现场；托盘菜单 → 打开运行日志</div>
                </div>
                <span className="path-box">snappin.log</span>
              </div>
              <div className="data-row">
                <div>
                  <div className="t">OCR 模型</div>
                  <div className="d">
                    det.onnx / rec.onnx / ppocr_keys_v1.txt（约 15.6MB，缺失时识别功能不可用）
                  </div>
                </div>
                <span className="path-box">models/ocr</span>
              </div>
              <div className="data-row">
                <div>
                  <div className="t">版本</div>
                  <div className="d">
                    {info?.platform ?? '-'} · {info?.arch ?? '-'}
                  </div>
                </div>
                <span className="path-box">v{info?.version ?? '-'}</span>
              </div>
            </div>
          </>
        )}

        {/* 底部保存条：所有页共用 */}
        <div className="btn-row">
          <button className="btn primary" disabled={!dirty} onClick={save}>
            保存设置
          </button>
          <span className={`save-state ${dirty ? 'dirty' : ''}`}>
            {toast || (dirty ? '有未保存的修改' : '已保存')}
          </span>
        </div>
      </div>

      {/* ---------------- 模型编辑弹层 ---------------- */}
      {editing && draft && (
        <div
          className="modal-mask"
          onMouseDown={(e) => {
            if (e.target === e.currentTarget) setEditing(null)
          }}
        >
          <div className="modal">
            <h3>{editing === 'new' ? '添加模型' : '编辑模型'}</h3>
            {editing === 'new' && (
              <div className="field">
                <label>快速预设</label>
                <select
                  value=""
                  onChange={(e) => {
                    const p = MODEL_PRESETS.find((x) => x.name === e.target.value)
                    if (p) {
                      setDraft({
                        ...draft,
                        name: p.name === '自定义' ? '' : p.name,
                        note: p.note,
                        baseUrl: p.baseUrl || draft.baseUrl,
                        model: p.model || draft.model,
                        keyUrl: p.keyUrl || draft.keyUrl,
                      })
                    }
                  }}
                >
                  <option value="">选择预设（可改）…</option>
                  {MODEL_PRESETS.map((p) => (
                    <option key={p.name} value={p.name}>
                      {p.name}
                      {p.note ? ` · ${p.note}` : ''}
                    </option>
                  ))}
                </select>
              </div>
            )}
            <div className="field">
              <label htmlFor="m-name">名称</label>
              <input
                id="m-name"
                type="text"
                value={draft.name}
                onChange={(e) => setDraft({ ...draft, name: e.target.value })}
                placeholder="DeepSeek-V4 Pro"
              />
            </div>
            <div className="field">
              <label htmlFor="m-note">备注</label>
              <input
                id="m-note"
                type="text"
                value={draft.note}
                onChange={(e) => setDraft({ ...draft, note: e.target.value })}
                placeholder="深度搜索"
              />
            </div>
            <div className="field">
              <label htmlFor="m-url">Base URL</label>
              <input
                id="m-url"
                type="text"
                value={draft.baseUrl}
                onChange={(e) => setDraft({ ...draft, baseUrl: e.target.value })}
                placeholder="https://api.deepseek.com"
              />
            </div>
            <div className="field">
              <label htmlFor="m-key">API Key</label>
              <input
                id="m-key"
                type="password"
                value={draft.apiKey}
                onChange={(e) => setDraft({ ...draft, apiKey: e.target.value })}
                placeholder="sk-…"
              />
            </div>
            <div className="field">
              <label htmlFor="m-model">模型 ID</label>
              <input
                id="m-model"
                type="text"
                value={draft.model}
                onChange={(e) => setDraft({ ...draft, model: e.target.value })}
                placeholder="deepseek-flash"
              />
            </div>
            <div className="field">
              <label htmlFor="m-keyurl">获取 Key 网址</label>
              <input
                id="m-keyurl"
                type="text"
                value={draft.keyUrl}
                onChange={(e) => setDraft({ ...draft, keyUrl: e.target.value })}
                placeholder="https://platform.deepseek.com/api_keys"
              />
            </div>
            <div className="btn-row">
              <button className="btn" onClick={() => setEditing(null)}>
                取消
              </button>
              <button
                className="btn primary"
                disabled={!draft.name.trim() || !draft.baseUrl.trim() || !draft.model.trim()}
                onClick={commitDraft}
              >
                确定
              </button>
            </div>
          </div>
        </div>
      )}
    </main>
  )
}
