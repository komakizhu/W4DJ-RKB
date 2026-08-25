#!/usr/bin/env bash
set -euo pipefail

ROOT="${1:-/private/tmp/W4DJ-RKB-v3.2.0-test-fixtures}"
FFMPEG="${FFMPEG:-$(command -v ffmpeg || true)}"
FFPROBE="${FFPROBE:-$(command -v ffprobe || true)}"

if [[ -z "$FFMPEG" || -z "$FFPROBE" ]]; then
  echo "需要 ffmpeg 和 ffprobe。" >&2
  exit 1
fi

rm -rf "$ROOT"
mkdir -p \
  "$ROOT/01-basic-formats" \
  "$ROOT/02-filename-cases" \
  "$ROOT/03-netease-like" \
  "$ROOT/04-analysis" \
  "$ROOT/05-failure-cases" \
  "$ROOT/06-conflict-output/existing" \
  "$ROOT/07-special-path/中文 路径 & space"

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

make_cover() {
  "$FFMPEG" -hide_banner -loglevel error -y \
    -f lavfi -i "color=c=0xE67E4A:s=256x256:d=1" \
    -frames:v 1 "$1"
}

make_tone() {
  local output="$1"
  local duration="$2"
  local frequency="$3"
  "$FFMPEG" -hide_banner -loglevel error -y \
    -f lavfi -i "sine=frequency=${frequency}:sample_rate=44100:duration=${duration}" \
    -c:a pcm_s16le "$output"
}

make_click_track() {
  local output="$1"
  local duration="$2"
  "$FFMPEG" -hide_banner -loglevel error -y \
    -f lavfi -i "aevalsrc=0.16*sin(2*PI*110*t)+0.08*sin(2*PI*220*t):s=44100:d=${duration}" \
    -af "volume=0.7" -c:a pcm_s16le "$output"
}

make_cover "$WORK/cover.jpg"
make_tone "$WORK/tagged-source.wav" 6 440
make_tone "$WORK/untagged-source.wav" 6 523.25
make_tone "$WORK/long-analysis.wav" 60 330
make_click_track "$WORK/drop-lufs-dance.wav" 48
make_tone "$WORK/short-under-32-beats.wav" 3 220

"$FFMPEG" -hide_banner -loglevel error -y -i "$WORK/tagged-source.wav" \
  -i "$WORK/cover.jpg" -map 0:a -map 1:v -c:a libmp3lame -b:a 192k -c:v mjpeg \
  -metadata title="Tagged Song" -metadata artist="Tagged Artist" \
  -metadata album="Tagged Album" -metadata genre="House" \
  -metadata comment="fixture: tagged mp3" -id3v2_version 3 \
  "$ROOT/01-basic-formats/tagged-mp3.mp3"

"$FFMPEG" -hide_banner -loglevel error -y -i "$WORK/tagged-source.wav" \
  -i "$WORK/cover.jpg" -map 0:a -map 1:v -c:a flac \
  -metadata title="Tagged FLAC" -metadata artist="Tagged Artist" \
  -metadata album="Tagged Album" -metadata genre="Jazz" \
  -metadata comment="fixture: tagged flac" \
  "$ROOT/01-basic-formats/tagged-flac.flac"

"$FFMPEG" -hide_banner -loglevel error -y -i "$WORK/untagged-source.wav" \
  -map_metadata -1 -c:a pcm_s16le \
  "$ROOT/01-basic-formats/untagged-source.wav"

"$FFMPEG" -hide_banner -loglevel error -y -i "$WORK/untagged-source.wav" \
  -map_metadata -1 -c:a pcm_s16be \
  "$ROOT/01-basic-formats/untagged-source.aiff"

"$FFMPEG" -hide_banner -loglevel error -y -i "$WORK/untagged-source.wav" \
  -map_metadata -1 -c:a libmp3lame -b:a 192k \
  "$ROOT/02-filename-cases/Artist First - Title Second.mp3"

"$FFMPEG" -hide_banner -loglevel error -y -i "$WORK/untagged-source.wav" \
  -map_metadata -1 -c:a flac \
  "$ROOT/02-filename-cases/Title First - Artist Second.flac"

"$FFMPEG" -hide_banner -loglevel error -y -i "$WORK/untagged-source.wav" \
  -map_metadata -1 -c:a libmp3lame -b:a 192k \
  "$ROOT/02-filename-cases/Artist One x Artist Two - Title (Official Mix).mp3"

cp "$ROOT/02-filename-cases/Artist First - Title Second.mp3" \
  "$ROOT/03-netease-like/网易云歌手 - 网易云歌曲.mp3"
cp "$WORK/cover.jpg" "$ROOT/03-netease-like/cover.jpg"
cat > "$ROOT/03-netease-like/README.txt" <<'EOF'
这是“普通 MP3 + 邻近封面”的网易云本地数据夹具。
它不是 .ncm，不包含网易云加密信息，只用于检查 W4DJ 是否结合文件名和同目录封面恢复元数据。
EOF

cp "$WORK/drop-lufs-dance.wav" "$ROOT/04-analysis/drop-lufs-dance.wav"
cp "$WORK/short-under-32-beats.wav" "$ROOT/04-analysis/short-under-32-beats.wav"
cp "$WORK/long-analysis.wav" "$ROOT/04-analysis/long-analysis.wav"

printf 'not an audio file\n' > "$ROOT/05-failure-cases/corrupted.mp3"
printf 'partial output\n' > "$ROOT/05-failure-cases/.w4dj-partial-output.mp3"
cp "$ROOT/01-basic-formats/tagged-mp3.mp3" \
  "$ROOT/06-conflict-output/existing/Tagged Song - Tagged Artist.mp3"
cp "$ROOT/02-filename-cases/Artist First - Title Second.mp3" \
  "$ROOT/07-special-path/中文 路径 & space/Artist First - Title Second.mp3"

cat > "$ROOT/MANIFEST.md" <<'EOF'
# W4DJ RKB 3.2.0 测试夹具

这些文件由 `scripts/generate-v3.2.0-test-fixtures.sh` 生成，只用于测试，不是音乐素材。

| 目录 | 用途 |
|---|---|
| `01-basic-formats` | 已有标签 MP3/FLAC，以及无标签 WAV/AIFF |
| `02-filename-cases` | 歌手-歌曲名、歌曲名-歌手、合作标记 |
| `03-netease-like` | 普通 MP3、邻近封面、中文文件名 |
| `04-analysis` | Drop LUFS、少于 32 Beat、较长音频 |
| `05-failure-cases` | 损坏文件和临时半成品 |
| `06-conflict-output` | 已存在目标文件 |
| `07-special-path` | 中文、空格、`&` 路径 |

夹具中的音频是合成音，只能验证流程、容器、标签、封面和错误处理，不能代表真实音乐的 Essentia 风格准确度。
EOF

"$FFPROBE" -hide_banner -loglevel error -show_entries format=duration \
  -of default=noprint_wrappers=1:nokey=1 \
  "$ROOT/01-basic-formats/tagged-mp3.mp3" > "$ROOT/tagged-mp3-duration.txt"

if command -v zip >/dev/null 2>&1; then
  (cd "$(dirname "$ROOT")" && zip -qr "$(basename "$ROOT").zip" "$(basename "$ROOT")")
fi

echo "已生成：$ROOT"
if [[ -f "${ROOT}.zip" ]]; then
  echo "压缩包：${ROOT}.zip"
fi
