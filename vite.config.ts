import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'

// SnapPin 前端配置：多 HTML 入口对应四个窗口（悬浮球/截图浮层/贴图/主窗）。
// 每页通过内联全局 __SNAPPIN_WINDOW__ 声明窗口类型（避免 URL 查询串被
// Tauri 的 App(PathBuf) URL 编码破坏）。
export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: {
    port: 5173,
    strictPort: true,
  },
  envPrefix: ['VITE_', 'TAURI_'],
  build: {
    outDir: 'dist',
    emptyOutDir: true,
    target: 'chrome110',
    sourcemap: false,
    rollupOptions: {
      input: {
        ball: 'ball.html',
        overlay: 'overlay.html',
        pin: 'pin.html',
        menu: 'menu.html',
        main: 'main.html',
        settings: 'settings.html',
        ocr: 'ocr.html',
        trans: 'trans.html',
        longpreview: 'longpreview.html',
      },
    },
  },
})
