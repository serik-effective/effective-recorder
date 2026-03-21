#!/bin/bash
# Build a minimal static FFmpeg with only the codecs we need (h264, aac)
# This eliminates ALL dynamic FFmpeg dependencies from the final binary
set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
BUILD_DIR="$SCRIPT_DIR/ffmpeg-build"
PREFIX="$BUILD_DIR/install"
NASM_VER="2.16.03"

echo "=== Building minimal static FFmpeg ==="
echo "    Only: libx264 + AAC (native) + swscale + swresample"
echo "    Output: $PREFIX"
echo ""

mkdir -p "$BUILD_DIR"
cd "$BUILD_DIR"

# ── 1. Build nasm (needed by x264) ────────────────────────────
if [ ! -f "$PREFIX/bin/nasm" ]; then
  echo ">>> Building nasm..."
  curl -sL "https://www.nasm.us/pub/nasm/releasebuilds/$NASM_VER/nasm-$NASM_VER.tar.xz" -o nasm.tar.xz
  tar xf nasm.tar.xz
  cd "nasm-$NASM_VER"
  ./configure --prefix="$PREFIX"
  make -j$(sysctl -n hw.ncpu)
  make install
  cd "$BUILD_DIR"
fi
export PATH="$PREFIX/bin:$PATH"

# ── 2. Build x264 (static) ───────────────────────────────────
if [ ! -f "$PREFIX/lib/libx264.a" ]; then
  echo ">>> Building x264..."
  if [ ! -d "x264" ]; then
    git clone --depth 1 https://code.videolan.org/videolan/x264.git
  fi
  cd x264
  ./configure \
    --prefix="$PREFIX" \
    --enable-static \
    --disable-shared \
    --disable-cli \
    --disable-opencl
  make -j$(sysctl -n hw.ncpu)
  make install
  cd "$BUILD_DIR"
fi

# ── 3. Build FFmpeg (static, minimal) ─────────────────────────
if [ ! -f "$PREFIX/lib/libavcodec.a" ]; then
  echo ">>> Building FFmpeg (minimal static)..."
  if [ ! -d "ffmpeg-src" ]; then
    git clone --depth 1 --branch n7.1 https://github.com/FFmpeg/FFmpeg.git ffmpeg-src
  fi
  cd ffmpeg-src
  PKG_CONFIG_PATH="$PREFIX/lib/pkgconfig" ./configure \
    --prefix="$PREFIX" \
    --enable-static \
    --disable-shared \
    --enable-gpl \
    --enable-libx264 \
    --disable-programs \
    --disable-doc \
    --disable-htmlpages \
    --disable-manpages \
    --disable-podpages \
    --disable-txtpages \
    --disable-network \
    --disable-everything \
    --enable-encoder=libx264 \
    --enable-encoder=aac \
    --enable-decoder=rawvideo \
    --enable-muxer=mp4 \
    --enable-muxer=mov \
    --enable-protocol=file \
    --enable-filter=aresample \
    --enable-filter=scale \
    --enable-filter=null \
    --enable-filter=anull \
    --enable-swscale \
    --enable-swresample \
    --enable-demuxer=rawvideo \
    --enable-parser=h264 \
    --extra-cflags="-I$PREFIX/include" \
    --extra-ldflags="-L$PREFIX/lib"
  make -j$(sysctl -n hw.ncpu)
  make install
  cd "$BUILD_DIR"
fi

echo ""
echo "=== Done! ==="
echo ""
echo "Static libs in: $PREFIX/lib/"
ls -la "$PREFIX/lib/"*.a
echo ""
echo "To build the app with static FFmpeg, run:"
echo ""
echo "  export FFMPEG_DIR=$PREFIX"
echo "  export PKG_CONFIG_PATH=$PREFIX/lib/pkgconfig"
echo "  bash rebuild.sh"
