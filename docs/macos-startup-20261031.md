# macOS 启动与隐藏子窗口排查（2026-10-31）

用户环境：Intel Mac，macOS 13.6.6。

## 日志证据

- 主窗口在 11:29:13 进入 `ball-ready`；托盘和两个热键注册完成。
- 预创建一个截图窗口后，整份日志没有任何子窗口的 `startup-binding` 或 `window-start`。
- 托盘和悬浮球打开菜单反复返回 `CHANNEL_UNREGISTERED`。原日志还错误地在失败后写入 `tray-menu-opened`。
- 三次截图尝试均返回 `Screen recording permission is required`，并未进入屏幕采集。

## 修复

- Flutter 3.47.6 的 macOS FlutterViewController 从 `viewWillAppear` 启动 engine；隐藏预创建窗口不能只依赖该生命周期回调。在子窗口原生插件与通道准备后显式调用 `engine.run(withEntrypoint: nil)`。该 API 的重复调用按 Flutter 文档直接返回。
- 原生层增加引擎创建、运行结果、权限和应用重新打开日志，文件为 `~/Library/Application Support/com.snappin.flutter/logs/snappin-native-日期.log`。
- 截图窗口预热必须收到 Dart `prewarmReady` 确认才记录“已就绪”。菜单打开期间合并重复点击，只有成功才记录托盘菜单已打开。
- 屏幕录制权限不足使用独立错误码，并显示原生说明与“打开系统设置”按钮。授权后提示退出重开。
- 点击已运行应用的图标时显示主窗口。
- CI 显式构建 Intel / Apple Silicon 通用包，并核对主程序及嵌入框架架构。打包前启动真正的 Release 应用，要求主窗口、隐藏截图窗口和菜单 IPC 就绪；失败时上传日志并阻止生成可交付 DMG。

CI 真实启动检查不等同于 macOS 13.6.6 真机的屏幕录制授权及截图功能验收。
