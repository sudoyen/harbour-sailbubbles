# Settings shared by the build scripts; SDK is where sdk.sh put the Sailfish SDK.
: "${SDK:?set SDK to a folder for the Sailfish SDK, see README.md}"
SDK=$(realpath -m "$SDK")
TARGET=$SDK/tgt-aarch64
TOOLING=$SDK/tooling
WRAP=$SDK/wrap
HEIF_PREFIX=${HEIF_PREFIX:-$SDK/heif}
export SDK HEIF_PREFIX
export CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER=$WRAP/sfos-aarch64-cc
export CC_aarch64_unknown_linux_gnu=$WRAP/sfos-aarch64-cc
export CXX_aarch64_unknown_linux_gnu=$WRAP/sfos-aarch64-cxx
export AR_aarch64_unknown_linux_gnu=$WRAP/aarch64-meego-linux-gnu-ar
