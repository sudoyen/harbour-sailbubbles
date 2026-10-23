#!/bin/sh
# Fetches the official Sailfish SDK target (aarch64) and tooling into $SDK and
# makes their i486 cross compilers usable on an x86_64 Linux host, without
# root and without sfdk.
set -eu
. "$(dirname "$0")/env.sh"
VERSION=${SFOS_VERSION:-5.1.0.11}
BASE=https://releases.sailfishos.org/sdk/targets

fetch() { # part folder
    file=Sailfish_OS-$VERSION-Sailfish_SDK_$1.tar.7z
    mkdir -p "$SDK/dl" "$SDK/$2"
    [ -f "$SDK/dl/$file" ] || curl -fL -o "$SDK/dl/$file" "$BASE/$file"
    (cd "$SDK/dl" && curl -fsSL "$BASE/$file.md5sum" | md5sum -c -)
    # device nodes cannot be made without root and are not needed
    7z x -so "$SDK/dl/$file" | tar -x --no-same-owner --exclude='./dev/*' -C "$SDK/$2"
}

[ -d "$TARGET/usr" ] || fetch Target-aarch64 tgt-aarch64
[ -d "$TOOLING/usr" ] || fetch Tooling-i486 tooling

# the tools are i486 programs; run them through the tooling's own loader
mkdir -p "$WRAP" "$TOOLING/opt/cross/aarch64-meego-linux-gnu/bin"
for tool in "$TOOLING"/opt/cross/bin/aarch64-meego-linux-gnu-*; do
    cat > "$WRAP/${tool##*/}" <<WRAPPER
#!/bin/sh
exec "$TOOLING/lib/ld-linux.so.2" --library-path "$TOOLING/lib:$TOOLING/usr/lib" "$tool" "\$@"
WRAPPER
    chmod +x "$WRAP/${tool##*/}"
done
# gcc looks for as and ld here
for tool in ar as ld ld.bfd ld.gold nm objcopy; do
    ln -sfn "$WRAP/aarch64-meego-linux-gnu-$tool" "$TOOLING/opt/cross/aarch64-meego-linux-gnu/bin/$tool"
done
for lang in cc:gcc cxx:g++; do
    printf '#!/bin/sh\nexec "%s" --sysroot="%s" "$@"\n' "$WRAP/aarch64-meego-linux-gnu-${lang#*:}" "$TARGET" \
        > "$WRAP/sfos-aarch64-${lang%%:*}"
    chmod +x "$WRAP/sfos-aarch64-${lang%%:*}"
done
# rusqlite links -lsqlite3; the target only has the versioned library
mkdir -p "$SDK/linkshim"
ln -sfn "$TARGET/usr/lib64/libsqlite3.so.0" "$SDK/linkshim/libsqlite3.so"
echo "Sailfish SDK ready in $SDK"
