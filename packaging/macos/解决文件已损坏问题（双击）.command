#!/bin/bash

set -u

APP_NAME="__W4DJ_APP_NAME__"
APP_PLACEHOLDER="__W4DJ_APP_NAME_""__"
if [[ "$APP_NAME" == "$APP_PLACEHOLDER" ]]; then
  APP_NAME="W4DJ RKB"
fi
APP_PATH="/Applications/${APP_NAME}.app"

if [[ ! -d "$APP_PATH" ]]; then
  osascript -e "display dialog \"请先把 ${APP_NAME}.app 拖到“应用程序”文件夹，再双击这个修复工具。\" buttons {\"知道了\"} default button \"知道了\" with title \"${APP_NAME}\""
  exit 1
fi

if /usr/bin/xattr -cr "$APP_PATH"; then
  osascript -e "display dialog \"已完成修复，现在可以打开 ${APP_NAME}。\" buttons {\"好的\"} default button \"好的\" with title \"${APP_NAME}\""
  if ! /usr/bin/open "$APP_PATH"; then
    osascript -e "display dialog \"修复已完成，但自动打开 ${APP_NAME} 失败，请手动打开应用程序文件夹中的 ${APP_NAME}.app。\" buttons {\"好的\"} default button \"好的\" with title \"${APP_NAME}\""
    exit 1
  fi
else
  osascript -e "display dialog \"修复失败，请确认 ${APP_NAME}.app 位于“应用程序”文件夹，并重试。\" buttons {\"好的\"} default button \"好的\" with title \"${APP_NAME}\""
  exit 1
fi
