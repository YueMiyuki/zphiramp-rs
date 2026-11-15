#!/bin/sh
# Real-encoder audio for tests/real_audio.rs: LAME 4.0 (MP3) and ffmpeg 9.0 (cover art, Ogg
# Vorbis, WAV) encode a synthetic 24 s signal (8 s pink noise, 8 s 1 kHz sine, 8 s silence, so
# the VBR bitrate varies), and the first bytes of each file are kept as tests/fixtures/real/*.head
# together with the size of the whole file and ffprobe's duration (the ground truth) in
# tests/fixtures/real/manifest.tsv.
#
# Recorded once and committed: the encoders are not run by the tests. Rerun only to add cases,
# then regenerate duration fixtures (README.md) so the Java column is recorded for the new rows.
#
#   sh tests/java/make_real_audio.sh     # from crates/phira; needs lame, ffmpeg, ffprobe
set -eu
OUT=tests/fixtures/real
WORK=$(mktemp -d)
mkdir -p "$OUT"
rm -f "$OUT"/*.head "$OUT/manifest.tsv"

ffmpeg -hide_banner -loglevel error -y \
  -f lavfi -i "anoisesrc=d=8:c=pink:a=0.6:r=44100:seed=7" -f lavfi -i "sine=f=1000:d=8:r=44100" \
  -f lavfi -i "anullsrc=r=44100:cl=stereo" \
  -filter_complex "[0:a]aformat=channel_layouts=stereo[a0];[1:a]aformat=channel_layouts=stereo[a1];[2:a]atrim=duration=8,aformat=channel_layouts=stereo[a2];[a0][a1][a2]concat=n=3:v=0:a=1[out]" \
  -map "[out]" "$WORK/src44.wav"
ffmpeg -hide_banner -loglevel error -y -t 6 -i "$WORK/src44.wav" "$WORK/src6.wav"

# record NAME KIND HEAD_BYTES FILE TOOL
record() {
  name=$1; kind=$2; cap=$3; file=$4; tool=$5
  head -c "$cap" "$file" > "$OUT/$name.head"
  size=$(wc -c < "$file" | tr -d ' ')
  ms=$(ffprobe -v error -show_entries format=duration -of csv=p=0 "$file" | awk '{printf "%d", $1 * 1000 + 0.5}')
  printf '%s\t%s\t%s\t%s\t%s\n' "$name" "$kind" "$size" "$ms" "$tool" >> "$OUT/manifest.tsv"
}

lame_run() { out=$1; shift; lame --quiet "$@" "$out"; }
SRC="$WORK/src44.wav"
lame --quiet -V0 "$SRC" "$WORK/a.mp3";                                   record lame_vbr_v0_44k mp3 32768 "$WORK/a.mp3" "lame -V0 (MPEG-1 stereo, Xing)"
lame --quiet --cbr -b 128 "$SRC" "$WORK/b.mp3";                          record lame_cbr128_info_44k mp3 32768 "$WORK/b.mp3" "lame --cbr -b 128 (Info tag)"
lame --quiet --cbr -b 128 -t "$SRC" "$WORK/c.mp3";                       record lame_cbr128_notag_44k mp3 32768 "$WORK/c.mp3" "lame --cbr -b 128 -t (no tag)"
lame --quiet -V4 -m m --resample 22.05 "$SRC" "$WORK/d.mp3";             record lame_vbr_v4_mono_22k mp3 32768 "$WORK/d.mp3" "lame -V4 -m m --resample 22.05 (MPEG-2 mono, Xing)"
lame --quiet --cbr -b 64 -t --resample 22.05 "$SRC" "$WORK/e.mp3";       record lame_cbr64_notag_22k mp3 32768 "$WORK/e.mp3" "lame --cbr -b 64 -t --resample 22.05 (MPEG-2)"
lame --quiet --cbr -b 32 -t -m m --resample 11.025 "$SRC" "$WORK/f.mp3"; record lame_cbr32_notag_mono_11k mp3 32768 "$WORK/f.mp3" "lame --cbr -b 32 -t -m m --resample 11.025 (MPEG-2.5)"
lame --quiet -V5 --resample 8 "$SRC" "$WORK/g.mp3";                      record lame_vbr_v5_8k mp3 32768 "$WORK/g.mp3" "lame -V5 --resample 8 (MPEG-2.5, Xing)"

# Error protection (`-p`): the header's protection bit is clear and a 16-bit CRC follows it. LAME
# still puts the Xing/Info tag at the unprotected offset (4 + side info), measured here.
lame --quiet -p -V2 "$SRC" "$WORK/k.mp3";                                record lame_vbr_crc_44k mp3 32768 "$WORK/k.mp3" "lame -p -V2 (CRC protected, Xing)"
lame --quiet -p --cbr -b 128 "$SRC" "$WORK/l.mp3";                       record lame_cbr128_crc_44k mp3 32768 "$WORK/l.mp3" "lame -p --cbr -b 128 (CRC protected, Info)"

# A tag larger than the first window: 6 s of 128 kbps behind a 50 KB JPEG cover (ID3v2.3 APIC)
lame --quiet --cbr -b 128 -t "$WORK/src6.wav" "$WORK/cover_base.mp3"
ffmpeg -hide_banner -loglevel error -y -f lavfi -i "nullsrc=s=280x280,geq=random(1)*255:128:128" -frames:v 1 -q:v 3 "$WORK/cover.jpg"
ffmpeg -hide_banner -loglevel error -y -i "$WORK/cover_base.mp3" -i "$WORK/cover.jpg" -map 0:a -map 1:v -c copy \
  -id3v2_version 3 -metadata:s:v title="Album cover" -metadata:s:v comment="Cover (front)" "$WORK/h.mp3"
record lame_cbr128_cover_44k mp3 1000000 "$WORK/h.mp3" "lame --cbr -b 128 -t + ffmpeg APIC cover (ID3v2.3)"

ffmpeg -hide_banner -loglevel error -y -i "$SRC" -c:a vorbis -strict -2 -b:a 96k "$WORK/i.ogg"
record ffmpeg_vorbis_96k ogg 4096 "$WORK/i.ogg" "ffmpeg native vorbis encoder (writes nominal bitrate 0)"
ffmpeg -hide_banner -loglevel error -y -i "$SRC" -c:a pcm_s16le "$WORK/j.wav"
record ffmpeg_pcm_44k wav 4096 "$WORK/j.wav" "ffmpeg pcm_s16le (LIST/INFO chunk before data)"

rm -rf "$WORK"
cat "$OUT/manifest.tsv"
