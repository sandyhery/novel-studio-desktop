#!/usr/bin/env bash
# 一键发布：打包 release → 安装到 /Applications → 启动新版本。
# 以后代码更新后，跑 `pnpm release` 即可把最新版装进「应用程序」。
set -euo pipefail

cd "$(dirname "$0")/.."

APP_NAME="小说工作台"
APP_SRC="src-tauri/target/release/bundle/macos/${APP_NAME}.app"
APP_DST="/Applications/${APP_NAME}.app"

echo "==> 1/3 打包 release（pnpm tauri build）..."
pnpm tauri build

if [ ! -d "$APP_SRC" ]; then
  echo "✗ 未找到打包产物：$APP_SRC" >&2
  exit 1
fi

echo "==> 2/3 安装到 /Applications ..."
# 尽力退出正在运行的旧实例，避免覆盖冲突（未运行则忽略错误）
osascript -e "tell application \"${APP_NAME}\" to quit" 2>/dev/null || true
sleep 1

if [ -d "$APP_DST" ]; then
  rm -rf "$APP_DST"
fi
ditto "$APP_SRC" "$APP_DST"

echo "==> 3/3 启动新版本 ..."
open "$APP_DST"

echo "✓ 完成：已安装并启动 $APP_DST"
