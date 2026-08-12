# Phone-as-passkey unlock (caBLE / CTAP hybrid) — why it isn't here

Investigated 2026-08-12. The goal was unlocking this machine by approving a
prompt on a phone, so that authentication is reachable when working from the
external keyboard with the laptop out of arm's reach.

**Outcome: abandoned, on evidence.** The transport works. The property that
would make it usable does not exist, by a deliberate decision at Google.

The implementation was removed in the commit that added this file. It is
recoverable from history:

| commit    | what it added                                          |
|-----------|--------------------------------------------------------|
| `d266460` | root-owned credential + linking store, permissions, tests |
| `359fadc` | enrollment ceremony (QR) and CLI                        |
| `b26d9ce` | hostname-based RP identity, real progress reporting     |
| `8e32fd7` | assertion against a linked phone                        |

## What works

Using `libwebauthn`'s cable transport, all of this was demonstrated end to end
against a Pixel 10 Pro:

- QR-initiated registration over the cloud-assisted tunnel.
- Assertion over the same, with the phone verifying the user biometrically.
- The full CTAP 2.3 hybrid **BLE-only** path: proximity check, LE pairing,
  encryption, L2CAP CoC on a dynamic PSM, Noise handshake, and a successful
  `MakeCredential`.

So phone-as-authenticator is real and functional on Linux. What it needs is a
QR scan for **every single ceremony**, which is not a lock screen.

## Why it stops there

Every unlock would be QR-free only if the phone sent *linking information*
("state-assisted", caBLE v2.1), which is what populates a `CableKnownDevice`.
It never did — not on registration, not on assertion, not over BLE, despite the
QR advertising key 4 (`state_assisted`) every time and the client lingering for
it after the ceremony.

The reason is in Chromium's own source, `device/fido/cable/v2_handshake.cc`:

```cpp
bool ShouldOfferLinking(RequestType request_type) {
  return std::visit(
      absl::Overload{[](const FidoRequestType&) {
                       // Hybrid linking is not supported for WebAuthn.
                       return false;
                     },
                     [](const CredentialRequestType&) {
                       return base::FeatureList::IsEnabled(
                           device::kDigitalCredentialsHybridLinking);
                     }},
      request_type);
}
```

Linking is hardcoded off for WebAuthn. It survives only for Digital
Credentials, the identity-wallet flow, and there behind a feature flag. The
phone parses `supports_linking` from the QR correctly and then declines to act
on it because the request type is WebAuthn. No client-side change reaches this.

Corroboration: Windows users report "linked devices" disappearing from the
registry and never coming back, which is the same decision seen from the other
side.

## Two Linux-side bugs found on the way

Both are real, both cost hours, and neither is documented anywhere we could
find. Worth reporting upstream to `libwebauthn`.

**1. `EACCES` on the L2CAP read is a missing BlueZ pairing agent.**

The BLE path fails with `Permission denied (os error 13)` on the first read
after a successful connect and write. That is not a Unix permission problem:
`net/bluetooth/lib.c` maps HCI status `0x05` (Authentication Failure) and
`0x18` (Pairing Not Allowed) onto `EACCES`. The phone connects from a fresh
random address every ceremony, so it is an unknown device requesting
encryption, and BlueZ refuses when no agent is registered to accept it.

Registering an agent that accepts the pairing fixes it. Note that bonding the
phone's *identity* address does not help and is not the fix: caBLE generates a
new non-resolvable address per ceremony (observed: four different addresses
across four runs), so the bond is never the peer being talked to.

A production implementation must register an agent only for the duration of a
ceremony the user started, and accept only the address the caBLE advertisement
already identified. A blanket-accept agent takes any pairing from anyone in
range.

**2. Bluetooth audio starves the handshake.**

With A2DP streaming to headphones on the same adapter, the handshake stalls
after the client's first message. A capture showed 6806 packets on the audio
handle against 268 on the LE link — the radio was 96 % audio. Disconnecting
the headphones was the difference between a hung ceremony and a successful one.

**Also worth upstreaming:** `connect_data_channel` falls back to the tunnel
only when the L2CAP *connect* fails. A connect that succeeds and then dies on
read takes the whole ceremony down instead of degrading, which is what made
this look like a protocol incompatibility for hours.

## What would have to change

Google would have to offer linking for WebAuthn requests again, at which point
the four commits above are most of a working implementation. Nothing else in
the stack is blocking: the transport, the store, the ceremony and the security
model were all built and tested.

## What was chosen instead

A biometric FIDO2 key (YubiKey Bio class) in a desk dock, driven by
`security.pam.u2f`. Offline, instant, one touch, and because verification
happens on the key it can stay plugged in without becoming "whoever reaches
the desk can authenticate".
