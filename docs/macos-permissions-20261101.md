# macOS 权限兼容与恢复（2026-11-01）

## 日志与参考

用户提供的 `snappin-2026-11-01.log` 中，11:21:31、11:22:01、11:22:48、11:23:00 均在原生 `captureDisplays` 的权限闸门返回 `screen_recording_permission_required`。重启没有解除。该日志没有运行路径和签名信息，不能单凭它证明是旧授权记录、路径不一致还是系统 preflight 误报。

核对 Tauri `src-tauri/src/commands.rs` 的 `mac_capture_gate`、`check_and_fix_permissions`、`reset_all_permissions`，以及 `src/windows/MenuWindow.tsx`、`src-tauri/src/lib.rs` 的托盘菜单：

- 两个菜单共用一个 macOS「权限检查…」入口，检查屏幕录制和辅助功能，并提供设置及重置动作。
- Tauri 对 macOS 13.6.6 的 preflight=false 做了兼容：系统请求只发一次，检测结果不阻断实际采集。真正没有权限时，系统仍会隐藏其他窗口，只能得到壁纸。
- Tauri 也使用 ad-hoc 签名；升级后旧授权可能不匹配，不能承诺跨版本免授权。

## Flutter 修复

- 托盘和悬浮球共用菜单加入同样的「权限检查…」，并增加一行窗口高度，避免退出项被截断。
- 同时报告 CGPreflightScreenCaptureAccess 和 AXIsProcessTrusted；去设置时优先打开当前缺失的权限页面。
- 只有用户明确点击「重置授权」才后台执行 `/usr/bin/tccutil reset ScreenCapture <当前 bundle-id>` 和 `Accessibility`，记录两个命令结果。不会清除其他应用记录，也不直接修改 TCC 数据库。
- 屏幕录制系统请求每个进程只触发一次。preflight 为 false 时提供「继续截图／权限检查…／取消」。用户选择继续后本次进程不再重复警告，继续使用受 macOS 权限约束的原生采集 API。
- 不把“返回了一张 PNG”当成权限已生效，不持久化这样的成功标记；避免壁纸图像被误当作已授权。
- 启动状态和菜单结果进入 Dart 日志；原生日志补应用路径、bundle ID、系统版本、签名 requirement、前后 preflight 状态、用户动作、重置结果和图片生成结果。
- 权限检查或取消后以 capture_cancelled 正常结束本次截图，不再叠加旧的权限提示框或错误 SnackBar。

## 验证边界

- 菜单测试覆盖 Windows 不显示该项、macOS 显示该项、有无快捷键告警时布局不溢出，以及点击后向主引擎派发 perm-check。
- macOS Release 自检读取真实权限诊断，确认 bundle、签名、路径与两项布尔状态可通过 IPC 返回；此自检不弹系统授权框、不执行重置。
- macOS CI 继续执行全量 Flutter 测试、通用架构编译、原生 OCR／退出／托盘自检及 DMG 挂载检查。
- 当前 Windows 环境不能操作用户 Intel macOS 13.6.6 的 TCC。真正的旧记录清理、授权生效和截图内容仍须在用户真机复测。

## 真机操作

退出旧版，从新 DMG 将 SnapPin 放入“应用程序”，推出 DMG，再打开应用。若截图警告出现，先选择“继续截图”观察窗口内容；能正常看到其他窗口即无需反复授权。若只有壁纸，从托盘或悬浮球 → 权限检查… → 重置授权，然后按对话框说明完全退出、重开并重新授权。关闭窗口不等于退出应用。
