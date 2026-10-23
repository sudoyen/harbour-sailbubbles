#!/bin/sh
# Builds libheif and libde265 for the phone into $HEIF_PREFIX; Sailfish has no
# HEIC decoder, and iPhones send their photos as HEIC.
set -eu
. "$(dirname "$0")/env.sh"
SRC=$SDK/heif-src
BUILD=$SDK/heif-build
mkdir -p "$SRC" "$BUILD"
[ -d "$SRC/libde265" ] || git clone --depth 1 -b v1.1.3 https://github.com/strukturag/libde265.git "$SRC/libde265"
[ -d "$SRC/libheif" ] || git clone --depth 1 -b v1.23.6 https://github.com/strukturag/libheif.git "$SRC/libheif"

cat > "$BUILD/toolchain.cmake" <<TOOLCHAIN
set(CMAKE_SYSTEM_NAME Linux)
set(CMAKE_SYSTEM_PROCESSOR aarch64)
set(CMAKE_SYSROOT $TARGET)
set(CMAKE_C_COMPILER $WRAP/sfos-aarch64-cc)
set(CMAKE_CXX_COMPILER $WRAP/sfos-aarch64-cxx)
set(CMAKE_FIND_ROOT_PATH $TARGET $HEIF_PREFIX)
set(CMAKE_FIND_ROOT_PATH_MODE_PROGRAM NEVER)
set(CMAKE_FIND_ROOT_PATH_MODE_LIBRARY ONLY)
set(CMAKE_FIND_ROOT_PATH_MODE_INCLUDE ONLY)
set(CMAKE_FIND_ROOT_PATH_MODE_PACKAGE ONLY)
TOOLCHAIN

common="-DCMAKE_TOOLCHAIN_FILE=$BUILD/toolchain.cmake -DCMAKE_BUILD_TYPE=Release -DCMAKE_INSTALL_PREFIX=$HEIF_PREFIX"
cmake -S "$SRC/libde265" -B "$BUILD/de265" $common -DENABLE_DECODER=OFF -DENABLE_ENCODER=OFF -DENABLE_SDL=OFF
cmake --build "$BUILD/de265" --target install
# only HEVC decoding through libde265, nothing else to ship
cmake -S "$SRC/libheif" -B "$BUILD/heif" $common -DWITH_LIBDE265=ON -DWITH_X265=OFF -DWITH_AOM_DECODER=OFF \
    -DWITH_AOM_ENCODER=OFF -DWITH_DAV1D=OFF -DWITH_RAV1E=OFF -DWITH_SvtEnc=OFF -DWITH_KVAZAAR=OFF \
    -DWITH_OpenJPEG_DECODER=OFF -DWITH_OpenJPEG_ENCODER=OFF -DWITH_JPEG_DECODER=OFF -DWITH_JPEG_ENCODER=OFF \
    -DWITH_FFMPEG_DECODER=OFF -DWITH_VVDEC=OFF -DWITH_VVENC=OFF -DWITH_UVG266=OFF -DWITH_UNCOMPRESSED_CODEC=OFF \
    -DWITH_LIBSHARPYUV=OFF -DWITH_HEADER_COMPRESSION=OFF -DWITH_GDK_PIXBUF=OFF -DWITH_EXAMPLES=OFF \
    -DENABLE_PLUGIN_LOADING=OFF -DBUILD_TESTING=OFF
cmake --build "$BUILD/heif" --target install
echo "libheif and libde265 installed in $HEIF_PREFIX"
