# Mac side

SailBubbles signs in to iMessage through your own Mac. Two programs run there:

- [registration-relay](https://github.com/beeper/registration-relay), unchanged,
  which the phone talks to;
- [mac-registration-provider](https://github.com/beeper/mac-registration-provider),
  with `mac-registration-provider.patch` from this folder, which answers Apple's
  checks for the phone.

The patch adds two things to mac-registration-provider (at commit `c9e8497`):

- **macOS 26.5.1 (25F80) on Apple Silicon.** The tool calls functions inside
  Apple's `identityservicesd` by their address, which moves with every macOS
  build. The patch adds the addresses for this build. Other builds need their
  own, found the same way.
- **Anisette from the Mac.** Apple sign-ins carry "anisette" headers. Since
  October 2026 Apple refuses those made by the public anisette servers (which
  emulate Android) next to a Mac identity, error -80045. The patch lets the
  relay ask the Mac for anisette made by Apple's own AOSKit, the
  `get-anisette-data` command, which SailBubbles uses for signing in.

The patch is under the AGPL-3.0, like mac-registration-provider (see LICENSE).

## Other macOS builds

`./mac-registration-provider -check-compatibility` says whether your build is
known. If it is not, the addresses for your `identityservicesd` have to be found
first, which takes some reverse engineering. Open an issue with the output of
`sw_vers` and of

    shasum -a 256 /System/Library/PrivateFrameworks/IDS.framework/identityservicesd.app/Contents/MacOS/identityservicesd

Building and running are described in the [main README](../README.md).
