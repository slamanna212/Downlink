#!/usr/bin/env bash
# Generate synthetic MPEG-TS clips (PLAN 13.3) into tools/fixtures/media/ (gitignored).
# Recipes are committed, media is not. Clips are short; serve.py loops them continuously.
# Usage: make-media.sh [--force]   (DUR=20 to change clip length in seconds)
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
OUT="$HERE/../media"
DUR="${DUR:-20}"
mkdir -p "$OUT"
FORCE="${1:-}"

V1080="testsrc2=size=1920x1080:rate=30:duration=$DUR"
A440="sine=frequency=440:sample_rate=48000:duration=$DUR"
FF=(ffmpeg -hide_banner -loglevel error -y)

gen() { # name, then ffmpeg args
  local name="$1"; shift
  if [ -f "$OUT/$name.ts" ] && [ "$FORCE" != "--force" ]; then echo "have $name.ts"; return; fi
  echo "making $name.ts"
  "${FF[@]}" "$@" -f mpegts "$OUT/$name.ts"
}

# 1. H.264 / AAC 1080p30 (the baseline channel)
gen h264_aac_1080p30 -f lavfi -i "$V1080" -f lavfi -i "$A440" \
  -c:v libx264 -preset veryfast -g 60 -pix_fmt yuv420p -b:v 6M -c:a aac -b:a 128k -shortest

# 2. HEVC + AC-3 (1080p30, 5.1-less stereo AC-3)
gen hevc_ac3_1080p30 -f lavfi -i "$V1080" -f lavfi -i "sine=frequency=523:sample_rate=48000:duration=$DUR" \
  -c:v libx265 -preset ultrafast -x265-params "keyint=60:log-level=error" -pix_fmt yuv420p -b:v 5M \
  -tag:v hvc1 -c:a ac3 -b:a 192k -shortest

# 3. 1080i (interlaced, top field first), 25 fps / 50 fields
gen h264_1080i_tff -f lavfi -i "testsrc2=size=1920x1080:rate=50:duration=$DUR" -f lavfi -i "sine=frequency=330:sample_rate=48000:duration=$DUR" \
  -vf "interlace=scan=tff,format=yuv420p" -c:v libx264 -preset veryfast -g 50 -flags +ilme+ildct -top 1 \
  -b:v 6M -c:a aac -b:a 128k -shortest

# 4. HEVC HDR10 (10-bit, BT.2020 / PQ, static metadata)
gen hevc_hdr10 -f lavfi -i "testsrc2=size=1920x1080:rate=30:duration=$DUR" -f lavfi -i "sine=frequency=659:sample_rate=48000:duration=$DUR" \
  -vf "format=yuv420p10le" -c:v libx265 -preset ultrafast -b:v 8M -tag:v hvc1 \
  -x265-params "keyint=60:log-level=error:colorprim=bt2020:transfer=smpte2084:colormatrix=bt2020nc:range=limited:hdr10=1:repeat-headers=1:master-display=G(13250,34500)B(7500,3000)R(34000,16000)WP(15635,16450)L(10000000,1):max-cll=1000,400" \
  -c:a aac -b:a 128k -shortest

# 5. Broken timestamps: remux the baseline with PTS/DTS jitter, a backwards jump and a forward gap.
BASE="$OUT/h264_aac_1080p30.ts"
if [ ! -f "$OUT/broken_timestamps.ts" ] || [ "$FORCE" = "--force" ]; then
  echo "making broken_timestamps.ts"
  "${FF[@]}" -i "$BASE" -c copy \
    -bsf:v "setts=pts='PTS+if(between(N,150,300),-90000*2,if(gt(N,450),90000*5,0))':dts='DTS+if(between(N,150,300),-90000*2,if(gt(N,450),90000*5,0))'" \
    -muxdelay 0 -f mpegts "$OUT/broken_timestamps.ts"
fi

ls -lh "$OUT"
