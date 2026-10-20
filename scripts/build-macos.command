#!/usr/bin/env bash
#
# SnapPin · macOS 一键构建
#
# 用法（在 Mac 上）：
#   1) 双击本文件（Finder 里首次可能要右键 → 打开）
#   2) 或在终端执行： bash scripts/build-macos.command
#
# 产物：
#   src-tauri/target/<target>/release/bundle/dmg/SnapPin_0.1.0_<arch>.dmg
#   src-tauri/target/<target>/release/bundle/macos/SnapPin.app
#
# 可选环境变量：
#   TARGET=aarch64-apple-darwin   只出 Apple Silicon（默认打通用包，最慢但哪儿都能跑）
#   TARGET=x86_64-apple-darwin    只出 Intel
#   TARGET=universal-apple-darwin 通用包（默认）
#   SKIP_INSTALL=1                跳过 npm ci / rustup target add
#
set -euo pipefail

cd "$(dirname "$0")/.."
ROOT="$(pwd)"
TARGET="${TARGET:-universal-apple-darwin}"

echo "=============================================="
echo " SnapPin macOS 构建"
echo " 目录   : $ROOT"
echo " 目标   : $TARGET"
echo "=============================================="
echo

die() { echo "✗ $1"; exit 1; }

# ---------- 1. 环境检查 ----------
command -v node  >/dev/null 2>&1 || die "缺少 Node.js。请先安装：brew install node  或 https://nodejs.org （需 20+）"
command -v npm   >/dev/null 2>&1 || die "缺少 npm（随 Node.js 安装）"
command -v cargo >/dev/null 2>&1 || die "缺少 Rust。请先安装：curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh"
command -v xcode-select >/dev/null 2>&1 || die "缺少 Xcode 命令行工具。请执行：xcode-select --install"
xcode-select -p >/dev/null 2>&1 || die "Xcode 命令行工具未就绪。请执行：xcode-select --install"

echo "node : $(node -v)"
echo "cargo: $(cargo --version)"
echo

# ---------- 2. 安装 Rust 目标 ----------
if [ "${SKIP_INSTALL:-0}" != "1" ]; then
  case "$TARGET" in
    universal-apple-darwin)
      echo "→ 安装 Rust 目标：aarch64-apple-darwin + x86_64-apple-darwin"
      rustup target add aarch64-apple-darwin x86_64-apple-darwin
      ;;
    *)
      echo "→ 安装 Rust 目标：$TARGET"
      rustup target add "$TARGET"
      ;;
  esac
  echo

  # ---------- 3. 前端依赖 ----------
  echo "→ 安装前端依赖"
  if [ -f package-lock.json ]; then
    npm ci
  else
    npm install
  fi
  echo
fi

# ---------- 4. 前端产物（tsc 类型检查 + vite） ----------
echo "→ 构建前端"
npm run build
echo

# ---------- 5. 打包 ----------
# --features custom-protocol: 必须，否则 WebviewUrl::App 会去连 devUrl 导致白屏
echo "→ 打包 .app / .dmg（首次编译约 5~15 分钟）"
npx tauri build --target "$TARGET" --features custom-protocol

DMG_DIR="$ROOT/src-tauri/target/$TARGET/release/bundle/dmg"
APP_DIR="$ROOT/src-tauri/target/$TARGET/release/bundle/macos"

echo
echo "=============================================="
echo " 完成，产物（完整路径，可直接复制）："
ls -1  "$DMG_DIR"/*.dmg 2>/dev/null || echo "  (未找到 .dmg，检查上面的打包日志)"
ls -1d "$APP_DIR"/*.app 2>/dev/null || true
echo "----------------------------------------------"
echo " 安装包（.dmg）目录：$DMG_DIR"
echo " 可直接运行（.app）：$APP_DIR"
echo "=============================================="
echo
echo "首次运行请注意："
echo " 1. 系统设置 → 隐私与安全性 → 屏幕录制 → 勾选 SnapPin，然后**重启应用**"
echo "    （没授权时截图会是全黑/空白，这是 macOS 系统限制）"
echo " 2. 若提示「已损坏/无法验证开发者」，执行："
echo "      xattr -dr com.apple.quarantine /Applications/SnapPin.app"
echo "    或右键 App → 打开"
echo " 3. 快捷键是 Control+1 / Control+2（不是 Command）"
echo

open "src-tauri/target/$TARGET/release/bundle/dmg" 2>/dev/null || true
