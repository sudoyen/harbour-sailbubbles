#!/bin/sh
# Fetches rustpush with the parts of it the daemon builds with, and puts its
# public legacy FairPlay certificate where rustpush looks for its named ones,
# as rustpush's own CI does.
set -eu
cd "$(dirname "$0")/.."
git submodule update --init rustpush
# two of rustpush's submodules are listed with SSH addresses; open-absinthe is not used
git -C rustpush -c url."https://github.com/".insteadOf=git@github.com: submodule update --init \
    third_party/apple-private-apis third_party/quinn third_party/rtc third_party/rustls \
    third_party/rustls-platform-verifier
mkdir -p rustpush/certs/fairplay
for name in 4056631661436364584235346952193 4056631661436364584235346952194 4056631661436364584235346952195 \
            4056631661436364584235346952196 4056631661436364584235346952197 4056631661436364584235346952198 \
            4056631661436364584235346952199 4056631661436364584235346952200 4056631661436364584235346952201 \
            4056631661436364584235346952208; do
    cp rustpush/certs/legacy-fairplay/fairplay.pem "rustpush/certs/fairplay/$name.pem"
    cp rustpush/certs/legacy-fairplay/fairplay.crt "rustpush/certs/fairplay/$name.crt"
done
