# Windows / macOS 发布验证

当前范围排除上一张／下一张和收藏，其余已实现功能执行全量自动回归。此记录不把 CI 自动验证等同于所有真机交互或逐像素视觉验收。

| 功能范围 | 验证入口 |
| --- | --- |
| 截图选区、工具条各工具、复制／保存／贴图／OCR／翻译／长图调度 | `capture_toolbar_test.dart` |
| 输入法组合、行内文字、多行输入与快捷键 | `capture_text_editor_test.dart` |
| 标注、马赛克、橡皮擦、聚光灯、智能擦除与序号历史 | annotation / capture_image / mosaic_preview / tauri_effect_parity 测试 |
| 长图方向恢复、滚动条跳转、内存边界、原始像素及后台导出 | `long_stitch_test.dart` |
| 贴图适应／原始尺寸／编辑／贴纸／保存反馈／未保存提醒／阴影快捷键 | `pin_viewer_test.dart`、`annotated_image_test.dart` |
| OCR 图片生命周期、布局坐标、分列排版、置顶切换 | ocr_image_store / ocr_document / ocr_window 测试 |
| 设置、模型增删改、连接错误、快捷键录制 | settings / models_settings / model_connection / shortcuts 测试 |
| 菜单热键占用提示、退出确认、保存取消、翻译窗口 | app_menu_warning / capture_exit_dialog / image_save / translation_window 测试 |
| Windows 真实 Release OCR、线程响应、重复调用、无效输入、原生阴影、识别中关闭 | `tool/windows_runtime_smoke.ps1` |
| macOS 真实 Release Vision、布局、线程响应、重复调用、无效输入、原生阴影、关闭 | `tool/macos_runtime_smoke.sh` |
| macOS 隐藏子引擎、托盘左右回调、菜单实际动作、菜单退出到进程 exit 0 | `tool/macos_startup_smoke.sh` |
| Intel 与 Apple Silicon | 可执行文件和所有嵌入 Mach-O 的 x86_64 / arm64 检查 |
| DMG 安装结构 | CI 挂载检查 SnapPin.app 和 /Applications 链接，本地 ZIP CRC、UDIF 与 SHA-256 校验 |

全量 Flutter 测试目前 86 项。Windows 的原生自检使用独立 APPDATA，不影响用户正在运行的实例和既有日志。Mac CI 在打包前执行相同 Flutter 测试和额外原生自检；任何失败都阻止 DMG 上传。

实际打印、第三方模型服务凭据、屏幕录制授权、物理鼠标与多屏 DPI 的全部组合没有被自动化测试穷尽。源码中的共享 UI 和平台 API 已对齐，发布包仍需用户的 Intel macOS 13.6.6 真机验收。
