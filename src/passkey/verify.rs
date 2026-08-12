//! Assertion against an already-linked phone: the unlock path, minus PAM.
//!
//! This is the half that has to work for the whole idea to be worth building.
//! Enrollment only proved the phone will link. This proves the linking state
//! can be used to reach it again, with nothing to scan, which is the entire
//! difference between "seamless" and "scan a QR every time you sit down".
//!
//! # This is not yet an authentication path
//!
//! It checks user verification and that the phone returned the credential we
//! pinned. It does **not** yet verify the assertion signature against the
//! enrolled public key, which needs COSE key parsing and P-256 ECDSA.
//!
//! Until that lands, a successful run here means "the phone answered", not
//! "the phone proved it holds the private key". Nothing in PAM may depend on
//! this module before [`verify_signature`] exists, and the command says so on
//! every run rather than trusting a future reader to know.

use std::sync::Arc;

use libwebauthn::fido::AuthenticatorDataFlags;
use libwebauthn::ops::webauthn::GetAssertionRequest;
use libwebauthn::transport::cable::known_devices::{CableKnownDevice, ClientPayloadHint};
use libwebauthn::transport::{Channel as _, ChannelSettings, Device as _};
use libwebauthn::webauthn::WebAuthn;

use super::enroll::{base64url, forward_updates, origin, random_bytes, settings, Progress};
use super::store::{rp_id, EnrolledCredential, FileDeviceStore};

/// Unlocking is not an attended ceremony: the user is waiting at a lock
/// screen. Long enough to pick the phone up, short enough that a phone that
/// is off or out of coverage falls back to a password quickly.
const TIMEOUT_MS: u32 = 30_000;

/// Contacts the linked phone and asks it to assert the enrolled credential.
///
/// Blocking; call it off the GTK thread.
pub fn run(
    user: &str,
    progress: impl Fn(Progress) + Clone + Send + Sync + 'static,
) -> Result<(), String> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("cannot start async runtime: {e}"))?;
    runtime.block_on(assertion(user, progress))
}

async fn assertion(
    user: &str,
    progress: impl Fn(Progress) + Clone + Send + Sync + 'static,
) -> Result<(), String> {
    let store = Arc::new(
        FileDeviceStore::open_default().map_err(|e| format!("cannot open passkey store: {e}"))?,
    );

    let credential = store
        .load_credential()
        .ok_or_else(|| "no credential enrolled; run passkey-enroll first".to_owned())?;
    check_credential_applies(&credential, user)?;

    // The store can hold several phones; any of them may be the one in the
    // room, so try each rather than guessing. First success wins.
    let devices = store.devices().await;
    if devices.is_empty() {
        return Err("no linked phone; run passkey-enroll first".to_owned());
    }

    let mut last_error = String::new();
    for (id, info) in devices {
        match try_device(&info, store.clone(), &credential, &progress).await {
            Ok(()) => return Ok(()),
            Err(e) => {
                log::debug!("passkey: device {id} did not assert: {e}");
                last_error = e;
            }
        }
    }
    Err(last_error)
}

async fn try_device(
    info: &libwebauthn::transport::cable::known_devices::CableKnownDeviceInfo,
    store: Arc<FileDeviceStore>,
    credential: &EnrolledCredential,
    progress: &(impl Fn(Progress) + Clone + Send + Sync + 'static),
) -> Result<(), String> {
    let mut device = CableKnownDevice::new(ClientPayloadHint::GetAssertion, info, store)
        .await
        .map_err(|e| format!("cannot address linked phone: {e:?}"))?;

    let mut channel = device
        .channel(ChannelSettings::default())
        .await
        .map_err(|e| format!("cannot open channel to phone: {e:?}"))?;
    forward_updates(channel.get_ux_update_receiver(), progress);

    let request = GetAssertionRequest::prepare(&origin()?, &request_json(credential)?, &settings())
        .await
        .map_err(|e| format!("malformed assertion request: {e:?}"))?;

    let response = channel
        .webauthn_get_assertion(&request)
        .await
        .map_err(|e| format!("phone declined or was unreachable: {e:?}"))?;

    let assertion = response
        .assertions
        .first()
        .ok_or_else(|| "phone returned no assertion".to_owned())?;

    // Requesting user verification is not the same as receiving it. Without
    // this check, an unlocked phone in the room is enough to unlock the
    // machine, which is a different and much weaker product.
    if !assertion
        .authenticator_data
        .flags
        .contains(AuthenticatorDataFlags::USER_VERIFIED)
    {
        return Err("phone did not verify the user (no biometric or PIN)".to_owned());
    }

    // Pinning allowCredentials asks for one credential; this checks we got
    // that one, so no other key the phone holds for this rp can stand in.
    match assertion.credential_id.as_ref() {
        Some(returned) if returned.id.as_slice() == credential.credential_id.as_slice() => {}
        Some(_) => return Err("phone asserted a different credential".to_owned()),
        // A single-entry allow list permits the id to be omitted; there was
        // only one thing it could have been.
        None => {}
    }

    verify_signature(assertion, credential)?;
    Ok(())
}

/// Placeholder for the check that makes this an authentication path.
///
/// Verifying the assertion means parsing the stored COSE key (EC2 P-256,
/// alg -7) and checking an ECDSA signature over `authenticatorData ||
/// SHA-256(clientDataJSON)`. That needs a CBOR reader and a P-256
/// implementation, neither of which is a direct dependency yet.
///
/// It returns `Ok` today so the transport can be exercised, and that is
/// exactly why nothing in PAM may call this module yet.
fn verify_signature(
    _assertion: &libwebauthn::ops::webauthn::Assertion,
    _credential: &EnrolledCredential,
) -> Result<(), String> {
    log::warn!(
        "passkey: assertion signature NOT verified — transport test only, not an auth decision"
    );
    Ok(())
}

/// A credential is only usable for the account and machine it was made for.
/// The rp id carries the hostname, so a renamed machine fails here with a
/// readable message instead of an opaque assertion error.
fn check_credential_applies(credential: &EnrolledCredential, user: &str) -> Result<(), String> {
    if credential.user != user {
        return Err(format!(
            "credential is enrolled for {}, not {user}",
            credential.user
        ));
    }
    let current = rp_id();
    if credential.rp_id != current {
        return Err(format!(
            "credential was enrolled for {} but this machine is {current}; re-enroll",
            credential.rp_id
        ));
    }
    Ok(())
}

fn request_json(credential: &EnrolledCredential) -> Result<String, String> {
    // Fresh every attempt. Phone passkeys commonly report a zero signature
    // counter forever, so replay defence rests on this alone.
    let challenge = base64url(&random_bytes::<32>()?);
    let cred_id = base64url(&credential.credential_id);
    let id = rp_id();

    Ok(format!(
        r#"{{
    "rpId": "{id}",
    "challenge": "{challenge}",
    "timeout": {TIMEOUT_MS},
    "allowCredentials": [
        {{ "type": "public-key", "id": "{cred_id}" }}
    ],
    "userVerification": "required"
}}"#
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cred() -> EnrolledCredential {
        EnrolledCredential {
            user: "meros".to_owned(),
            rp_id: rp_id(),
            credential_id: vec![1, 2, 3, 4],
            public_key_cose: vec![9; 77],
        }
    }

    #[test]
    fn request_pins_the_credential_and_demands_uv() {
        let json = request_json(&cred()).unwrap();
        assert!(json.contains(r#""userVerification": "required""#));
        assert!(json.contains(&base64url(&[1, 2, 3, 4])));
        assert!(json.contains(&format!(r#""rpId": "{}""#, rp_id())));
    }

    #[test]
    fn every_attempt_gets_a_new_challenge() {
        assert_ne!(request_json(&cred()).unwrap(), request_json(&cred()).unwrap());
    }

    #[test]
    fn credential_is_rejected_for_another_account() {
        let err = check_credential_applies(&cred(), "melvin").unwrap_err();
        assert!(err.contains("enrolled for meros"));
    }

    #[test]
    fn credential_is_rejected_after_a_rename() {
        let mut c = cred();
        c.rp_id = "old-name.local".to_owned();
        let err = check_credential_applies(&c, "meros").unwrap_err();
        assert!(err.contains("re-enroll"));
    }
}
