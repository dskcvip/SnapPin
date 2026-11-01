# macOS 启动空白窗口

用户真机日志记录 macOS 12.7.4：原生窗口和插件注册完成，但没有 Dart 启动及托盘就绪记录。多次点击应用后触发 `native-app-reopen`。现有 reopen 实现无条件显示主窗口，可能直接暴露尚未配置的 800×600 默认窗口。

主窗口现在从 nib 加载时保持隐藏，关闭窗口状态恢复，所有通道安装后显式调用主 Flutter 引擎的 `run`，不再依赖窗口显示触发 `viewWillAppear`。记录引擎启动前、返回结果、Dart 入口握手和悬浮球就绪阶段；15 秒后仍未就绪时记录各阶段状态。悬浮球完成定位、热键及托盘注册后，才允许显示。

应用重复打开改为经 Dart 请求恢复悬浮球，启动未完成时排队；截图过程中不会强制显示悬浮球。正常启动回归验证主引擎、Dart 握手、窗口大小及隐藏后重新打开的完整路径，成功标记是 `macos-main-startup-smoke-ready`，缺失时禁止打包。

这修复了已确认的生命周期依赖和空白窗口显示路径；用户真机的引擎实际返回状态仍需新版日志及运行结果确认，CI 的新系统运行不能替代 Intel macOS 12.7.4 真机验收。

## 第二次真机日志与兼容性处理

用户新版日志明确显示 `native-main-engine-run success=false`、`dartStarted=false`，排除了仅靠隐藏窗口和等待生命周期即可解决的假设。Flutter `run` 还可能在重复启动时返回 false，因此最终以 Dart 握手作为实际启动证据，不仅凭返回值判断。

Flutter 3.47 默认启用 Impeller。此轮将整个应用（包括子引擎）统一设为 Skia/Metal，使用官方 `FLTEnableImpeller=false` 配置；这是针对图形初始化兼容性的修正方向，尚不能仅由上述日志断定 Impeller 就是唯一根因。启动时记录 CPU 架构、Metal 设备和安装路径。原生窗口创建前开始将 stderr 同时写入原控制台及独立的 `snappin-engine-日期-PID.log`，保留 Flutter 的底层初始化错误。如果 Dart 仍未启动，显示原生失败提示，提供打开日志并退出，避免静默驻留。

CI 增加 Intel (`macos-15-intel`) 和 Apple 芯片两套实际构建运行验证；逐个检查主程序、AOT App.framework、Flutter 引擎及插件的两种架构和 Mach-O 最低系统版本，并检查运行资源完整性。两套均运行现有 91 项 Flutter 测试、主引擎与悬浮球恢复、托盘菜单/退出、原生 OCR、真实滚轮长图拼接回归，并验证使用 Skia。CI 使用的系统仍高于用户的 12.7.4，不能声称已完成同版本真机验证。

## 已确认的 Metal 设备缺失

第三份引擎日志明确记录 `metalDevices=[]`、`Could not acquire Metal device`、`Unable to create FlutterView; no MTLDevice or MTLCommandQueue available`，最后 `FlutterEngineInitialize` 报渲染器配置无效。实际运行路径为 `/Volumes/SnapPin/SnapPin.app`。这次失败发生在 Flutter 界面和 Dart 入口之前，Skia/Metal 也无法绕过缺少 Metal 设备的问题。

现在在创建 FlutterViewController 前检查默认 Metal 设备和命令队列。缺失时保持主窗口隐藏，不注册 Flutter 插件、不启动 Flutter 引擎，立即显示原生说明，提供打开日志并退出及退出两个入口。重复打开应用不会把该失败路径重新排入悬浮球启动队列。

CI 两种架构都注入缺设备、缺命令队列两种情形，实际执行前置检查并构建原生错误提示；验证未进入 Flutter 初始化、未显示默认主窗口，并以约定退出码退出，不留下挂起进程。正常启动及现有全部回归仍保留。这只改善不支持环境的失败处理，不能使没有 Metal 的虚拟机运行 Flutter。
