/** 窗口类型：由各 HTML 入口的内联全局 __SNAPPIN_WINDOW__ 声明。 */
export type WindowKind =
  | 'ball'
  | 'overlay'
  | 'pin'
  | 'menu'
  | 'main'
  | 'settings'
  | 'ocr'
  | 'trans'
  | 'pinmanager'
  | 'longresult'

declare global {
  interface Window {
    __SNAPPIN_WINDOW__?: WindowKind
  }
}

export function getWindowKind(): WindowKind {
  const kind = window.__SNAPPIN_WINDOW__
  if (
    kind === 'ball' ||
    kind === 'overlay' ||
    kind === 'pin' ||
    kind === 'menu' ||
    kind === 'ocr' ||
    kind === 'trans' ||
    kind === 'pinmanager' ||
    kind === 'longresult' ||
    kind === 'settings'
  ) {
    return kind
  }
  return 'main'
}
