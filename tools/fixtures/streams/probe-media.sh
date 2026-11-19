#!/usr/bin/env bash
# Print codec/field-order/colour info for every generated clip.
HERE="$(cd "$(dirname "$0")" && pwd)"
for f in "$HERE"/../media/*.ts; do
  echo "== $(basename "$f")"
  ffprobe -v error -show_entries stream=codec_name,profile,width,height,field_order,pix_fmt,color_transfer,color_primaries,channels -of compact "$f"
done
