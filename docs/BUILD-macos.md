# 在 macOS 上构建 SnapPin

> Windows 上**无法**交叉编译出 `.app`：macOS 二进制必须在 macOS 上链接
> （需要 Xcode 的 clang + macOS SDK，Apple 不授权在其他系统上分发 SDK）。
> 所以 Mac 端的包要在那台 Mac 上跑一条命令生成。

## 零、手边没有 Mac，但又想要现成的 dmg？

在 Windows 上**做不出** dmg（原因见文首），但有两条不用 Mac 的路：

### 路线 A：GitHub Actions 的 macOS 云主机（免费，推荐）

仓库里已备好 `.github/workflows/build-macos-dmg.yml`，手工触发即可出包：

```bash
# 1) 在 GitHub 建一个空仓库（public 即可，免费额度够用）
git remote add origin https://github.com/<你的账号>/<仓库名>.git
git add -A
git commit -m "ci: build macos dmg"
git push -u origin main
```

2) 打开仓库网页 → **Actions** → 左侧「出 macOS 安装包（dmg）」→ **Run workflow**
3) 等 10~15 分钟，在该次运行的页面底部 **Artifacts** 下载：
   - `snappin-macos-dmg.zip` → 解压得到 `SnapPin_0.1.0_universal.dmg`
   - `snappin-macos-app.zip` → 免安装的 `SnapPin.app`

Run workflow 时可只选一个架构（`aarch64` 快一倍），默认 `universal` 两种 Mac 都能跑。

### 路线 B：借一台 Mac 跑脚本

就是下面第一节，5~15 分钟一条命令。

> 注意：dmg 是 ad-hoc 签名（`signingIdentity: "-"`），从网上下载后被 Gatekeeper
> 拦下属正常，右键 → 打开，或 `xattr -dr com.apple.quarantine /Applications/SnapPin.app`。

## 一、一键构建（推荐）

把项目拷到 Mac（U 盘 / 局域网 / git 都行），然后在终端：

```bash
cd /path/to/SnapPin
bash scripts/build-macos.command
```

也可以在 Finder 里双击 `scripts/build-macos.command`（首次可能需右键 → 打开）。
若双击提示"没有执行权限"（从 zip 里解出来会丢权限位），执行一次：

```bash
chmod +x scripts/build-macos.command
```

脚本会依次做：检查 Node/Rust/Xcode CLT → `rustup target add` → `npm ci` →
`npm run build` → `npx tauri build`，最后自动打开产物目录。

默认打**通用包**（Apple Silicon + Intel 都能跑，最慢）。
只想出自己机器的架构，可指定目标，快一倍以上：

```bash
TARGET=aarch64-apple-darwin bash scripts/build-macos.command   # Apple Silicon
TARGET=x86_64-apple-darwin   bash scripts/build-macos.command   # Intel
```

## 二、产物位置

> ⚠️ **`.dmg` 只在 Mac 上构建后才会出现**，Windows 上不存在（macOS 的包必须用
> macOS 的 clang + SDK 链接，Apple 也不允许在其他系统分发 SDK）。
> 你拿到的是源码 zip，需要在 Mac 上跑一次构建脚本才会产出 dmg。

产物路径**相对于项目根目录**（即你解压出来的 `SnapPin/` 目录）：

```
target/universal-apple-darwin/release/bundle/dmg/SnapPin_0.1.0_universal.dmg
target/universal-apple-darwin/release/bundle/macos/SnapPin.app
```

> 📌 这里没有 `src-tauri/` 前缀 —— 本项目是 **cargo workspace**（根 `Cargo.toml`
> 里有 `[workspace]`），cargo 把 `target/` 放在**仓库根**，不是 `src-tauri/target/`。
> 按 `src-tauri/target/...` 找会扑空。

换成**完整绝对路径**（把 `<用户名>` 换成你的 macOS 用户名，`~/SnapPin` 换成你实际解压的位置）：

```
/Users/<用户名>/SnapPin/target/universal-apple-darwin/release/bundle/dmg/SnapPin_0.1.0_universal.dmg
/Users/<用户名>/SnapPin/target/universal-apple-darwin/release/bundle/macos/SnapPin.app
```

构建脚本跑完会直接把 dmg 的**完整路径**打出来，并自动在 Finder 里打开该目录。
也可以自己找：

```bash
# 在项目根目录执行，直接给出绝对路径
ls -1 "$PWD"/target/*/release/bundle/dmg/*.dmg

# 或者全盘找（慢一点，但不会错）
find ~ -name "SnapPin_*.dmg" 2>/dev/null
```

注意：`universal-apple-darwin` 这段目录名随 `TARGET` 变化——
只出 Apple Silicon 时是 `aarch64-apple-darwin`，只出 Intel 时是 `x86_64-apple-darwin`。

装到「应用程序」：打开 dmg，把 `SnapPin.app` 拖进去。

## 三、首次运行必做：屏幕录制权限

macOS 10.15 起，任何程序读屏幕像素都要授权。

**未授权时系统不会报错**，它照常给你一张图，只是把**其他应用的窗口全部过滤掉** ——
所以现象不是"截图黑屏"，而是 **"进入截图后屏幕上所有窗口都不见了，只剩桌面壁纸"**，
截出来的图自然也只有壁纸。（更早的 macOS 版本会返回全黑图，现象不同但根因相同。）

1. 首次启动会弹系统授权框 → 允许；
2. 若没弹或误点了拒绝：`系统设置 → 隐私与安全性 → 屏幕录制` → 勾选 **SnapPin**；
   列表里没有 SnapPin 时点 `+` 手动添加 `/Applications/SnapPin.app`；
3. **列表里已勾选但仍截不到窗口**：先**取消勾选、再重新勾选**。原因是 dmg 是
   ad-hoc 签名，每次重新构建/cdhash 变化，TCC 会把它当成"另一个程序"，旧授权失效；
4. **改完权限必须退出并重新打开 SnapPin**（macOS 的硬性要求，权限只在进程启动时读取）。

### 为什么"昨晚那版能截到窗口，换了新包就只剩壁纸"

**实测结论（2026-10-21）**：同一台机器、同一份采集代码（xcap 0.9.8 未变），
昨晚 22:31 那个 dmg 能正常截到所有窗口，今天新打的 dmg 只截到壁纸 —— 差别不在代码，
在**代码签名**：dmg 是 ad-hoc 签名（`signingIdentity = "-"`），每次重新构建 cdhash
都会变，TCC 数据库里那条"已允许"的记录就与新二进制对不上了。此时会出现最迷惑人的
状态：**系统设置里 SnapPin 明明勾着，但系统就是不认**，而且
`CGPreflightScreenCaptureAccess()` 也可能仍返回 `true`（它只看 TCC 记录在不在，
不看记录与当前二进制是否匹配）。

所以**每次装新 dmg 后都要重做一遍授权**（换开发者签名才会彻底消失）：

```bash
# 方案 A（推荐，纯 GUI）：系统设置 → 隐私与安全性 → 屏幕录制
#   选中 SnapPin → 点「−」删掉旧记录 → 重新启动 SnapPin → 从弹出的授权框里添加并勾选
#   → 再退出、重新打开（权限只在进程启动时读取）

# 方案 B（终端一把清干净，之后重新启动 SnapPin 走一次授权框）：
tccutil reset ScreenCapture com.snappin.dev
```

判定到底是"没授权"还是"授权失效"：看日志里这一行（托盘菜单 → 打开运行日志）

```
[snappin] 屏幕录制权限 preflight = true / false
```

- `false` → 就是没授权，按上面 1~4 步勾上；
- `true` 但截出来只有壁纸 → 授权记录失效（签名变了），只能按上面删掉重加。

另外**别从 dmg 挂载盘或「下载」目录里直接双击运行**：未移入 `/Applications` 时
macOS 会启用路径随机化（App Translocation），每次启动路径都不同，授权永远存不住。
先拖进 `/Applications` 再打开。

代码里的处理：

- 启动时 `CGPreflightScreenCaptureAccess` 探测，未授权才调
  `CGRequestScreenCaptureAccess` 触发系统授权框（后者每个进程只能弹一次）；
- 探测结果无论成败都写进日志：`[snappin] 屏幕录制权限 preflight = true/false`；
- **每次截图前**再次 preflight（`commands::mac_capture_gate`）。未授权则**中止截图**并弹
  原生提示框（含「打开系统设置」按钮，直达屏幕录制面板），而不是让用户截一张空桌面；
- 未授权状态同时显示在托盘 tooltip 上。

## 四、Gatekeeper

本地构建是 **ad-hoc 签名**（`bundle.macOS.signingIdentity = "-"`），
不是开发者签名，也没有公证。所以：

- 从别的机器拷过来（AirDrop / 下载）会被打上隔离属性 → 提示"无法打开/已损坏"，
  解决：右键 App → 打开，或执行
  `xattr -dr com.apple.quarantine /Applications/SnapPin.app`
- 自己机器上直接构建的不会带隔离属性，双击即可。

要正式分发再考虑 Developer ID 签名 + notarytool 公证（需要 Apple 开发者账号）。

## 四·五、退出与卸载（0.1.1 及更早版本的"卡死"现场）

**症状**：菜单栏图标点不动 → 界面上没有任何地方能退出；Finder 里把 App 拖进废纸篓
提示"正在使用"；装新 dmg 时提示"无法替换，因为正在使用"。

**根因**：本应用是 `Accessory`（不占 Dock），所以既没有 Dock 图标右键菜单，
也没有 ⌘Q（没有 app menu）。托盘菜单是唯一出口。0.1.0/0.1.1 换成自绘菜单后，
macOS 上建 webview 窗口必须在主线程而当时没投递 → 菜单建不出来 → 出口全灭。
（0.1.2 起：修了主线程建窗，并且**右键改成系统原生菜单**做兜底，见第五节。）

**旧版本卡住时怎么退**（任选其一）：

```bash
pkill -x SnapPin            # 0.1.1 起可执行文件叫 SnapPin
pkill -x snappin            # 0.1.0 及更早叫 snappin
pkill -f 'SnapPin|snappin'  # 不确定哪个名字时用这条（宽松匹配）
```

图形界面替代：`⌥⌘⎋`（Option+Command+Esc）强制退出窗口里选中 SnapPin → 强制退出；
或「活动监视器」搜 SnapPin → ⑧ 退出。

**卸载**（先退出进程，再删）：

```bash
pkill -f 'SnapPin|snappin'
rm -rf /Applications/SnapPin.app
rm -rf ~/Library/Application\ Support/com.snappin.dev   # 配置 + 日志 + 模型缓存
rm -rf ~/Library/Caches/com.snappin.dev
rm -rf ~/Library/WebKit/com.snappin.dev
tccutil reset ScreenCapture com.snappin.dev              # 清掉屏幕录制授权记录
```

**为什么删不掉**：`/Applications` 的 App 被运行中的进程占用时 Finder 会拒绝移动；
另外如果当初是**直接从挂载的 dmg 里双击运行**，那个磁盘映像因为"正在使用"也无法推出
（`hdiutil detach /Volumes/SnapPin` 或磁盘工具推出）。先 kill 进程再操作即可。

## 五、macOS 端已做的适配

| 项 | 处理 |
|---|---|
| 透明窗口 | `tauri.conf.json` 开 `macOSPrivateApi: true` + Cargo `macos-private-api` feature（缺一会启动即报配置不一致） |
| 不占 Dock | `set_activation_policy(Accessory)`，只在菜单栏留图标 |
| 退出兜底 | 菜单栏图标**右键 = 系统原生 NSMenu**（左键仍是自绘菜单，`show_menu_on_left_click(false)`）。原生菜单不依赖 webview / 前端 / 主线程投递，任何情况下都能「退出 SnapPin」，避免再次出现"没有出口只能 kill" |
| 屏幕录制权限 | 启动时 preflight + request；**每次截图前再 preflight**，未授权中止截图并弹原生提示（含直达系统设置按钮）+ 托盘 tooltip 告警；成功与否都记日志 |
| 建窗线程 | macOS 要求 NSWindow 在主线程建：浮层预创建、贴图窗口都改成投递主线程并同步等待；Windows 保持原有的工作线程建窗（主线程会死锁） |
| 显示器原点单位 | xcap 在 macOS 返回的是**点**、Windows 是**物理像素**，新增 `MonitorShot::logical_origin()` 统一成逻辑像素，避免 Retina 副屏位置算成 1/2 |
| 打开"关于"链接 | 已有 `open` 分支 |

## 六、已知差异 / 待实测（诚实清单）

1. **快捷键是 Control+1 / Control+2**，不是 Command+1/2（沿用 Windows 习惯；
   Mac 上想要 ⌘ 组合需要另加注册）。
2. **剪贴板贴图统一落在主屏中央**（Ctrl+V 跟随光标的贴图入口已下线）。
3. **多显示器**：坐标换算已按平台修正，但只在 Windows 双屏验证过；
   Mac 上的副屏（尤其"显示器在左侧/上方"）位置请重点测。
4. 贴图窗口的橙色阴影、工具条 hover 等纯视觉项按 CSS 渲染，理论上一致，需目测。
5. 未做代码签名/公证，未做 App Store 沙盒。

## 七、Mac 上建议的验收顺序

1. 启动 → 菜单栏出现 SnapPin 图标、屏幕上出现悬浮球；
2. 点球 → 整屏压暗 → 拖出选区 → 工具条 16 个按钮、二级菜单在按钮下方且不超出工具条；
3. Enter 复制 → 剪贴板内容正确；Ctrl+2 贴剪贴板图；Ctrl+1 重新截图；
4. 保存 PNG → 用「预览」打开确认是 Retina 满分辨率（逻辑尺寸 ×2 的像素）；
5. Esc / 右键取消；退出用菜单栏「退出」。

任何一步不符合，把 Mac 上终端里 `SnapPin.app/Contents/MacOS/snappin` 的
stdout/stderr 贴回来（我们用 `eprintln!` 打了完整链路日志）。
