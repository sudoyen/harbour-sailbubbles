#!/bin/sh
# Builds the daemon and the app for the phone and packages both as RPMs in out/.
set -eu
. "$(dirname "$0")/env.sh"
ROOT=$(realpath "$(dirname "$0")/..")
OUT=$ROOT/out
STRIP=$WRAP/aarch64-meego-linux-gnu-strip
WORK=$TOOLING/tmp/sailbubbles-rpm

[ -d "$TARGET/usr" ] && [ -x "$WRAP/sfos-aarch64-cc" ] || { echo "run scripts/sdk.sh first" >&2; exit 1; }
[ -f "$HEIF_PREFIX/lib/libheif.so" ] || { echo "run scripts/build-heif.sh first" >&2; exit 1; }
"$ROOT/scripts/prepare.sh"

(cd "$ROOT/daemon" && SAILFISH_SYSROOT=$TARGET SQLITE3_LIB_DIR=$SDK/linkshim \
    cargo build --release --target aarch64-unknown-linux-gnu)
make -C "$ROOT/app" SDK="$SDK"

rm -rf "$WORK"
mkdir -p "$WORK/SOURCES" "$WORK/daemon-libs/usr/lib64/sailbubbles-daemon" \
         "$WORK/daemon-libs/usr/share/licenses/sailbubbles-daemon"
libs=$WORK/daemon-libs/usr/lib64/sailbubbles-daemon
cp -a "$HEIF_PREFIX"/lib/libheif.so.* "$HEIF_PREFIX"/lib/libde265.so.* "$libs/"
for lib in "$libs"/*; do [ -L "$lib" ] || "$STRIP" --strip-unneeded "$lib"; done
cp "$SDK/heif-src/libheif/COPYING" "$WORK/daemon-libs/usr/share/licenses/sailbubbles-daemon/libheif-COPYING"
cp "$SDK/heif-src/libde265/COPYING" "$WORK/daemon-libs/usr/share/licenses/sailbubbles-daemon/libde265-COPYING"
tar -cf "$WORK/SOURCES/heic-libs.tar" --owner=0 --group=0 -C "$WORK/daemon-libs" .
"$STRIP" -o "$WORK/SOURCES/sailbubbles-daemon" \
    "${CARGO_TARGET_DIR:-$ROOT/daemon/target}/aarch64-unknown-linux-gnu/release/sailbubbles-daemon"
cp "$ROOT"/daemon/rpm/sailbubbles-daemon.service "$ROOT"/daemon/rpm/SailBubbles.permission \
   "$ROOT"/daemon/rpm/x-sailbubbles.message.conf "$WORK/SOURCES/"

app=$WORK/app
mkdir -p "$app/usr/bin" "$app/usr/share/harbour-sailbubbles" "$app/usr/share/applications" \
         "$app/usr/share/dbus-1/services"
"$STRIP" -o "$app/usr/bin/harbour-sailbubbles" "$ROOT/app/build/harbour-sailbubbles"
cp -r "$ROOT/app/qml" "$app/usr/share/harbour-sailbubbles/"
cp "$ROOT/app/harbour-sailbubbles.desktop" "$app/usr/share/applications/"
cp "$ROOT/app/org.sailbubbles.harbour-sailbubbles.service" "$app/usr/share/dbus-1/services/"
for s in 86 108 128 172; do
    install -D -m 644 "$ROOT/app/icons/${s}x$s.png" "$app/usr/share/icons/hicolor/${s}x$s/apps/harbour-sailbubbles.png"
done
tar -cf "$WORK/SOURCES/harbour-sailbubbles.tar" --owner=0 --group=0 -C "$app" .

cp "$ROOT/daemon/rpm/sailbubbles-daemon.spec" "$ROOT/app/rpm/harbour-sailbubbles.spec" "$WORK/"
# rpmbuild from the tooling, in a user namespace so no root is needed
for spec in sailbubbles-daemon harbour-sailbubbles; do
    unshare -rm /usr/sbin/chroot "$TOOLING" /usr/bin/rpmbuild -bb --target aarch64 \
        --define "_topdir /tmp/sailbubbles-rpm" --define "_sourcedir /tmp/sailbubbles-rpm/SOURCES" \
        "/tmp/sailbubbles-rpm/$spec.spec"
done
mkdir -p "$OUT"
cp "$WORK"/RPMS/aarch64/*.rpm "$OUT/"
rm -rf "$WORK"
ls "$OUT"
