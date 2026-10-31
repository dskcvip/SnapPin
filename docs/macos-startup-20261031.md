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

## 托盘菜单专项检查

- 核对 tray_manager 的 macOS 原生左右键事件均转发到对应 Dart 回调。
- CI 核对原生托盘图标有有效 bounds，调用实际左右键 Dart 回调，要求菜单完成一帧绘制且可见、获得焦点。
- 两次分别通过菜单实际 action/IPC 执行“显示 / 隐藏悬浮球”，检查主窗口可见性确实变化，第二次恢复悬浮球；只有两次均成功才允许打包。
- 本检查覆盖真实窗口与插件通信，但回调由检查代码调用，不代表已在 Intel macOS 13.6.6 真机物理点击托盘验收。
- 最后重新打开托盘菜单，调用菜单真实“退出”action，要求日志记录退出分发且主进程以 0 退出；仅启动成功但退出卡住也会阻止打包。退出请求写入持久日志，可与菜单 action 时间对照。
- 首次专项运行确认菜单可见、左右键回调和 action 分发正常；发现 window_manager 的 macOS hide 在返回后才异步 orderOut。检查改为等待真实可见性变化（最多 2 秒），避免把请求返回误当成窗口状态已经变化。
