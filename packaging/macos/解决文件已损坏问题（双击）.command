#!/bin/bash

set -u

APP_PATH="/Applications/W4DJ RKB.app"

if [[ ! -d "$APP_PATH" ]]; then
  osascript -e 'display dialog "请先把 W4DJ RKB.app 拖到“应用程序”文件夹，再双击这个修复工具。" buttons {"知道了"} default button "知道了" with title "W4DJ RKB"'
  exit 1
fi

if /usr/bin/xattr -cr "$APP_PATH"; then
  osascript -e 'display dialog "已完成修复，现在可以打开 W4DJ RKB。" buttons {"好的"} default button "好的" with title "W4DJ RKB"'
  if ! /usr/bin/open "$APP_PATH"; then
    osascript -e 'display dialog "修复已完成，但自动打开 W4DJ RKB 失败，请手动打开应用程序文件夹中的 W4DJ RKB.app。" buttons {"好的"} default button "好的" with title "W4DJ RKB"'
    exit 1
  fi
else
  osascript -e 'display dialog "修复失败，请确认 W4DJ RKB.app 位于“应用程序”文件夹，并重试。" buttons {"好的"} default button "好的" with title "W4DJ RKB"'
  exit 1
fi
