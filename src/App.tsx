import { BallWindow } from './windows/BallWindow'
import { OverlayWindow } from './windows/OverlayWindow'
import { PinWindow } from './windows/PinWindow'
import { MainWindow } from './windows/MainWindow'
import { MenuWindow } from './windows/MenuWindow'
import { OcrWindow } from './windows/OcrWindow'
import { TranslationWindow } from './windows/TranslationWindow'
import { PinManagerWindow } from './windows/PinManagerWindow'
import { LongResultWindow } from './windows/LongResultWindow'
import { LongPreviewWindow } from './windows/LongPreviewWindow'
import { getWindowKind } from './lib/windowKind'

/**
 * 单一 SPA 承载全部窗口，由各 HTML 入口的内联全局 __SNAPPIN_WINDOW__ 区分：
 * - ball     悬浮球
 * - overlay  截图选区浮层（每显示器一个）
 * - pin      贴图窗口
 * - menu     自绘菜单（托盘点击 / 悬浮球右键）
 * - main     主窗口（设置/动作）
 */
export function App() {
  switch (getWindowKind()) {
    case 'ball':
      return <BallWindow />
    case 'overlay':
      return <OverlayWindow />
    case 'pin':
      return <PinWindow />
    case 'menu':
      return <MenuWindow />
    case 'settings':
      // 设置窗（label=settings，HTML 入口 settings.html）
      return <MainWindow />
    case 'ocr':
      return <OcrWindow />
    case 'trans':
      return <TranslationWindow />
    case 'pinmanager':
      return <PinManagerWindow />
    case 'longresult':
      return <LongResultWindow />
    case 'longpreview':
      return <LongPreviewWindow />
    default:
      return <MainWindow />
  }
}
