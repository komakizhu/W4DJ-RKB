#!/usr/bin/env bash
set -euo pipefail

ROOT="${1:-/private/tmp/W4DJ-RKB-v3.2.0-test-fixtures}"
FFPROBE="${FFPROBE:-$(command -v ffprobe || true)}"

if [[ -z "$FFPROBE" ]]; then
  echo "需要 ffprobe。" >&2
  exit 1
fi
if [[ ! -d "$ROOT" ]]; then
  echo "夹具目录不存在：$ROOT" >&2
  exit 1
fi

failures=0
check_file() {
  if [[ ! -f "$1" ]]; then
    echo "FAIL missing: $1"
    failures=$((failures + 1))
  else
    echo "PASS exists: $1"
  fi
}

check_audio() {
  local path="$1"
  check_file "$path"
  if [[ -f "$path" ]] && ! "$FFPROBE" -hide_banner -loglevel error -select_streams a:0 \
    -show_entries stream=codec_name,duration -of csv=p=0 "$path" >/dev/null; then
    echo "FAIL unreadable audio: $path"
    failures=$((failures + 1))
  else
    echo "PASS readable audio: $path"
  fi
}

for path in \
  "$ROOT/01-basic-formats/tagged-mp3.mp3" \
  "$ROOT/01-basic-formats/tagged-flac.flac" \
  "$ROOT/01-basic-formats/untagged-source.wav" \
  "$ROOT/01-basic-formats/untagged-source.aiff" \
  "$ROOT/02-filename-cases/Artist First - Title Second.mp3" \
  "$ROOT/02-filename-cases/Title First - Artist Second.flac" \
  "$ROOT/03-netease-like/网易云歌手 - 网易云歌曲.mp3" \
  "$ROOT/04-analysis/drop-lufs-dance.wav" \
  "$ROOT/04-analysis/short-under-32-beats.wav" \
  "$ROOT/04-analysis/long-analysis.wav" \
  "$ROOT/07-special-path/中文 路径 & space/Artist First - Title Second.mp3"; do
  check_audio "$path"
done

check_file "$ROOT/03-netease-like/cover.jpg"
check_file "$ROOT/05-failure-cases/corrupted.mp3"
check_file "$ROOT/05-failure-cases/.w4dj-partial-output.mp3"
check_file "$ROOT/06-conflict-output/existing/Tagged Song - Tagged Artist.mp3"

if "$FFPROBE" -hide_banner -loglevel error -show_entries format=duration \
  -of default=noprint_wrappers=1:nokey=1 \
  "$ROOT/05-failure-cases/corrupted.mp3" >/dev/null 2>&1; then
  echo "FAIL corrupted fixture unexpectedly parses as audio"
  failures=$((failures + 1))
else
  echo "PASS corrupted fixture is rejected"
fi

if [[ "$failures" -ne 0 ]]; then
  echo "夹具校验失败：$failures 项"
  exit 1
fi
echo "夹具校验通过。"
