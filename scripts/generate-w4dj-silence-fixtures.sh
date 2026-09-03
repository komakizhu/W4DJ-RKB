#!/usr/bin/env bash

set -euo pipefail

playlist_path="${1:?usage: $0 PLAYLIST.w4dj OUTPUT_DIRECTORY}"
output_directory="${2:?usage: $0 PLAYLIST.w4dj OUTPUT_DIRECTORY}"
source_labels_path="${3:-}"

if [[ ! -f "$playlist_path" ]]; then
  echo "playlist does not exist: $playlist_path" >&2
  exit 1
fi

if ! command -v jq >/dev/null 2>&1 || ! command -v ffmpeg >/dev/null 2>&1 || ! command -v ffprobe >/dev/null 2>&1; then
  echo "jq, ffmpeg, and ffprobe are required" >&2
  exit 1
fi

mkdir -p "$output_directory"

jq -e '
  .format == "w4dj" and
  .format_version == 2 and
  (.tracks | length > 0) and
  all(.tracks[]; (.position | type) == "number" and (.title | type) == "string" and (.artist_display | type) == "string")
' "$playlist_path" >/dev/null

if [[ -n "$source_labels_path" ]]; then
  if [[ ! -f "$source_labels_path" ]]; then
    echo "source label map does not exist: $source_labels_path" >&2
    exit 1
  fi

  expected_count="$(jq '.tracks | length' "$playlist_path")"
  jq -e --argjson expected_count "$expected_count" '
    type == "array" and
    length == $expected_count and
    all(.[]; (.position | type) == "number" and (.title | type) == "string" and (.artist_display | type) == "string")
  ' "$source_labels_path" >/dev/null
fi

safe_component() {
  local value="$1"
  value="$(printf '%s' "$value" | tr -cs '[:alnum:]_.-' '_' | cut -c1-80)"
  value="${value##_}"
  value="${value%_}"
  printf '%s' "${value:-track}"
}

while IFS=$'\t' read -r position title artist; do
  source_title="$title"
  source_artist="$artist"
  if [[ -n "$source_labels_path" ]]; then
    source_row="$(jq -r --argjson position "$position" '
      first(.[] | select(.position == $position) | [.title, .artist_display] | @tsv) // empty
    ' "$source_labels_path")"
    if [[ -z "$source_row" ]]; then
      echo "source label map is missing position $position" >&2
      exit 1
    fi
    IFS=$'\t' read -r source_title source_artist <<<"$source_row"
  fi

  padded_position="$(printf '%03d' "$position")"
  safe_title="$(safe_component "$source_title")"
  fixture_path="$output_directory/${padded_position}-${safe_title}.wav"

  ffmpeg -hide_banner -loglevel error -nostdin -y \
    -f lavfi -i 'anullsrc=r=44100:cl=stereo' -t 2 \
    -metadata "title=$source_title" \
    -metadata "artist=$source_artist" \
    -c:a pcm_s16le "$fixture_path"

  probe="$(ffprobe -v error -show_entries format=duration:format_tags=title,artist -of json "$fixture_path")"
  jq -e --arg title "$source_title" --arg artist "$source_artist" '
    ((.format.duration // "0") | tonumber) > 0 and
    (.format.tags.title // "") == $title and
    (.format.tags.artist // "") == $artist
  ' <<<"$probe" >/dev/null
done < <(jq -r '.tracks[] | [.position, .title, .artist_display] | @tsv' "$playlist_path")

echo "generated $(find "$output_directory" -maxdepth 1 -type f -name '*.wav' | wc -l | tr -d ' ') WAV fixture(s) in $output_directory"
