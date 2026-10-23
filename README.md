# SailBubbles

iMessage for Sailfish OS, as a native Silica app. Chat with iPhone, iPad and Mac
users: text, photos and videos, replies, reactions, edits, unsending, read
receipts, typing, group chats (name, photo, people), search, drafts, archiving
and notifications.

It is built on [rustpush](https://github.com/OpenBubbles/rustpush), the iMessage
library of [OpenBubbles](https://openbubbles.app), and follows how the
OpenBubbles Android app behaves. It is not affiliated with OpenBubbles or Apple.

## How it works

Apple only lets its own devices register for iMessage, so SailBubbles registers
through **your own Mac**:

- On the Mac run two small programs: `registration-relay`, which the phone talks
  to, and `mac-registration-provider`, which answers Apple's checks for the phone
  (with [a patch](mac/README.md) for current macOS and Apple's sign-in).
- On the phone run `sailbubbles-daemon`, a background service that keeps the
  connection to Apple open and stores your chats, and `harbour-sailbubbles`, the
  app.

The phone reaches the Mac over your home network, or a VPN, when you sign in
and every few weeks when the registration renews. In between, messages work
anywhere and the Mac can be off.

## Before you start

- **A Mac** on Apple Silicon with macOS 26.5.1 (build 25F80), or a macOS
  version mac-registration-provider supports. Step 3 below tells you.
- **A phone** with Sailfish OS 5.1 or later on a 64-bit ARM chip (recent Jolla
  phones, Xperia 10 and later).
- **Your Apple ID**, and a device or phone number that gets its 2-factor codes.
- Apple does not support this. It may block an Apple ID or a Mac it finds using
  iMessage this way. Use it with that in mind.
- Everyone uses their own Mac. There is no shared relay, and there will be none.

## Installing from the ready-made builds

The [releases](../../releases) have the phone packages and a zip with both Mac
programs (for Apple Silicon and Intel).

### On the Mac

1. Download `sailbubbles-mac.zip` and unzip it, for example into your home
   folder. In Terminal:

       cd ~/sailbubbles-mac
       xattr -d com.apple.quarantine mac-registration-provider registration-relay
       chmod +x mac-registration-provider registration-relay

   The programs are not signed by Apple; `xattr` tells macOS you downloaded them
   on purpose.

2. Check your macOS is supported:

       ./mac-registration-provider -check-compatibility

   If it says unsupported, stop here: your macOS build needs its own addresses
   added to the patch first, see [mac/README.md](mac/README.md).

3. Make a secret for the relay, once, and keep it:

       openssl rand -base64 32

4. Start the relay. Port 5000 is taken by macOS AirPlay, hence the second
   address:

       ./registration-relay -secret 'THE SECRET FROM STEP 3' -metricsListen 127.0.0.1:5001

   If macOS asks whether to accept incoming network connections, allow them.

5. In a second Terminal window, start the provider:

       cd ~/sailbubbles-mac
       ./mac-registration-provider -relay-server http://localhost:8000

   It prints an iMessage registration code. You need it on the phone.

6. Find the Mac's address on your network: System Settings, Wi-Fi, Details,
   "IP address", for example 192.168.1.23. The relay is then at
   `http://192.168.1.23:8000`.

Keep both programs running while you sign in on the phone, and later whenever
the registration renews (the app's Settings shows when). The Mac must not sleep
then.

### On the phone

1. Settings, Untrusted software: allow installing it.
2. Download `sailbubbles-daemon-*.rpm` and `harbour-sailbubbles-*.rpm` and
   install the daemon first, then the app (tap them in Downloads or the file
   manager).
3. Restart the phone, so the message service starts.
4. Open SailBubbles. It asks, step by step:
   - the relay address from step 6, like `http://192.168.1.23:8000`
   - the code from step 5
   - your Apple ID and password
   - the code Apple sends to your devices, or by SMS

   Then it registers for iMessage and your chats come in. Sailfish asks once to
   allow the app's permissions; allow them.

The relay must be on your own network (a home network, a VPN such as Tailscale,
or the phone itself): the code travels unencrypted, so SailBubbles accepts no
other addresses.

## Building from source

### The phone packages

The Sailfish SDK's own Rust is too old for the daemon's dependencies, so the
build uses Rust from rustup and the official Sailfish SDK target and tooling
archives, without root and without sfdk. On an x86_64 Linux machine you need:

- rustup, with `rustup target add aarch64-unknown-linux-gnu`
- `protoc`, `cmake`, `git`, `curl`, `7z`, `make`, and SQLite's development
  files for the host tests (`libsqlite3-dev`)
- user namespaces (`unshare -r`), for rpmbuild in the SDK tooling; on Ubuntu
  24.04 and later: `sudo sysctl kernel.apparmor_restrict_unprivileged_userns=0`

Then:

    git clone https://github.com/sudoyen/harbour-sailbubbles
    cd harbour-sailbubbles
    export SDK=$HOME/sailfish-sdk      # any folder, about 4 GB
    scripts/sdk.sh                     # Sailfish OS 5.1.0.11 target and tooling
    scripts/build-heif.sh              # libheif and libde265, for HEIC photos
    scripts/build.sh                   # both RPMs, in out/

`build.sh` first runs `scripts/prepare.sh`, which fetches rustpush and the parts
of it the daemon uses, and puts rustpush's public legacy FairPlay certificate
under the names rustpush expects, as rustpush's own CI does. The daemon's tests
run on the build machine after that: `scripts/prepare.sh && cd daemon && cargo test`.

### The Mac programs

On the Mac, with the Xcode command line tools (`xcode-select --install`) and Go:

    git clone https://github.com/beeper/mac-registration-provider
    cd mac-registration-provider
    git checkout c9e8497d0d383c0a466371502af390f7a185e36b
    git apply /path/to/sailbubbles/mac/mac-registration-provider.patch
    go build -o mac-registration-provider

    git clone https://github.com/beeper/registration-relay
    cd registration-relay
    go build -o registration-relay ./cmd/registration_relay

Then continue with step 2 of "On the Mac" above.

### Releases

Pushing a tag such as `v0.2.0` runs `.github/workflows/release.yml`, which
builds all of the above and publishes it as a GitHub release.

## If something goes wrong

- **"no Mac is connected to this relay with that code"**: the provider is not
  running, or the code is mistyped.
- **"the relay must be on your own network"**: use the Mac's local address,
  not a public one.
- **The relay address does not answer**: phone and Mac must be on the same
  network, and the relay must be running.
- **Apple asks to sign in again later**: in the app, pull down, Settings, "Sign
  in again", with both Mac programs running.
- The daemon logs to the journal (`journalctl --user -u sailbubbles-daemon`),
  never with message text, names or numbers.

## Licenses

- SailBubbles is under the Server Side Public License (SSPL) 1.0, see
  [LICENSE](LICENSE), as rustpush is.
- `mac/mac-registration-provider.patch` is under the AGPL-3.0, like
  mac-registration-provider, see [mac/LICENSE](mac/LICENSE). The release's Mac
  programs are built from mac-registration-provider and registration-relay
  (both AGPL-3.0) at the commits named above.
- The daemon package ships unmodified builds of
  [libheif](https://github.com/strukturag/libheif) 1.23.6 and
  [libde265](https://github.com/strukturag/libde265) 1.1.3 as shared libraries,
  under the LGPL-3.0; their license texts are installed with them and
  `scripts/build-heif.sh` rebuilds them from those tags.
