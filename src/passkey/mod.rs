//! Phone-as-passkey authentication (caBLE / CTAP hybrid).
//!
//! Unlock this machine by approving a prompt on a phone, with the biometric
//! happening there. One QR scan at enrollment links the phone; every unlock
//! afterwards contacts it directly through the tunnel, so there is nothing to
//! scan day to day.
//!
//! # Shape
//!
//! - [`store`] holds the linking state and the enrolled credential, root-owned.
//! - Enrollment runs one QR ceremony and persists what it learns.
//! - The agent (later) performs an assertion on request and answers yes or no.
//!
//! # Non-negotiables
//!
//! Whatever else changes, these five are the security of the scheme:
//!
//! 1. Request `userVerification: required` AND verify the UV bit that comes
//!    back. Asking without checking turns this into proximity auth, where an
//!    unlocked phone in the room is enough.
//! 2. A fresh 32-byte CSPRNG challenge per attempt. Phone passkeys routinely
//!    report a signature counter of zero forever, so the counter is not a
//!    replay defence and the challenge is carrying it alone.
//! 3. Pin `allowCredentials` to the enrolled credential id and re-check the id
//!    in the response, so no other credential on the phone can stand in.
//! 4. The store stays root-owned and 0600, and enrollment stays behind pkexec.
//!    Write access there is a complete bypass.
//! 5. Fail closed. No adapter, no network, timeout, malformed response: deny.
//!    PAM entries stay `sufficient`, never `required`, so password and
//!    fingerprint remain underneath.
//!
//! Unlock needs Bluetooth and the internet. That is a denial-of-service
//! surface, not an escalation one, and the fallbacks are why.

pub mod store;
