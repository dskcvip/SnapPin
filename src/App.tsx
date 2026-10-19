import { BallWindow } from './windows/BallWindow'
import { OverlayWindow } from './windows/OverlayWindow'
import { PinWindow } from './windows/PinWindow'
import { MainWindow } from './windows/MainWindow'
import { getWindowKind } from './lib/windowKind'

/**
 * 单一 SPA 承载全部窗口，通过 URL 查询参数 ?window=<kind> 区分：
 * - ball     悬浮球
 * - overlay  截图选区浮层（每显示器一个）
 * - pin      贴图窗口
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
    default:
      return <MainWindow />
  }
}
