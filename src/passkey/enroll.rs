//! One-time enrollment: link a phone and register the credential that will
//! unlock this machine.
//!
//! This is the only ceremony that shows a QR code. libwebauthn's persistent QR
//! device hands the phone's linking information to our store as a side effect,
//! and that linking state is what lets every later unlock contact the phone
//! directly with nothing to scan.
//!
//! Runs blocking, on a worker thread. The caller supplies a sink for progress
//! so a GTK surface can draw the QR without this module knowing about GTK.
//!
//! # Security notes specific to enrollment
//!
//! Enrollment writes to a root-owned store, so it must run privileged (pkexec)
//! and only after the user has already authenticated by other means. An
//! unauthenticated enrollment path is a privilege escalation: it lets whoever
//! reaches it register their own phone.
//!
//! User verification is required here as well as at assertion time. Enrolling
//! a credential the phone will release without a biometric would build the
//! weakness in at the root, where no later check can recover it.

use std::sync::Arc;

use libwebauthn::fido::AuthenticatorDataFlags;
use libwebauthn::ops::webauthn::{
    MakeCredentialRequest, OriginValidation, RequestOrigin, RequestSettings,
};
use libwebauthn::transport::cable::qr_code_device::{
    CableQrCodeDevice, CableTransports, QrCodeOperationHint,
};
use libwebauthn::transport::cable::is_available;
use libwebauthn::transport::{ChannelSettings, Device as _};
use libwebauthn::webauthn::WebAuthn;

use super::store::{EnrolledCredential, FileDeviceStore, RP_ID};

/// Enrollment is a deliberate, attended act, so the window is generous: the
/// user has to find their phone, open the camera and approve.
const TIMEOUT_MS: u32 = 120_000;

/// What the caller needs in order to drive a UI.
#[derive(Debug, Clone)]
pub enum Progress {
    /// caBLE QR payload. Render it as a QR code; the phone's camera reads it.
    ShowQr(String),
    /// The phone answered and the encrypted channel is up. The QR is now
    /// useless and should come off screen.
    Connected,
}

/// Runs the ceremony to completion. Blocking: call it off the GTK thread.
///
/// On success the credential is already persisted, and the returned copy is
/// for display only.
pub fn run(
    user: &str,
    progress: impl Fn(Progress) + Send + 'static,
) -> Result<EnrolledCredential, String> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("cannot start async runtime: {e}"))?;
    runtime.block_on(ceremony(user, progress))
}

async fn ceremony(
    user: &str,
    progress: impl Fn(Progress),
) -> Result<EnrolledCredential, String> {
    if !is_available().await {
        return Err("No Bluetooth adapter. The phone is contacted over BLE, so \
                    enrollment cannot proceed without one."
            .to_owned());
    }

    let store = Arc::new(
        FileDeviceStore::open_default().map_err(|e| format!("cannot open passkey store: {e}"))?,
    );

    // Persistent, not transient: the transient constructor discards the
    // phone's linking information, which would leave every future unlock
    // needing a fresh QR.
    let mut device = CableQrCodeDevice::new_persistent(
        QrCodeOperationHint::MakeCredential,
        store.clone(),
        CableTransports::CloudAssistedOrLocal,
    )
    .map_err(|e| format!("cannot start cable ceremony: {e:?}"))?;

    progress(Progress::ShowQr(device.qr_code.to_string()));

    let mut channel = device
        .channel(ChannelSettings::default())
        .await
        .map_err(|e| format!("phone did not connect: {e:?}"))?;
    progress(Progress::Connected);

    let request = MakeCredentialRequest::prepare(&origin()?, &request_json(user)?, &settings())
        .await
        .map_err(|e| format!("malformed enrollment request: {e:?}"))?;

    let response = channel
        .webauthn_make_credential(&request)
        .await
        .map_err(|e| format!("enrollment declined or failed: {e:?}"))?;

    // Requesting user verification is not the same as getting it. A credential
    // enrolled without UV would be released later on possession alone.
    if !response
        .authenticator_data
        .flags
        .contains(AuthenticatorDataFlags::USER_VERIFIED)
    {
        return Err("The phone did not verify you (no biometric or PIN). \
                    Refusing to enroll a credential that unlocks on possession alone."
            .to_owned());
    }

    let attested = response
        .authenticator_data
        .attested_credential
        .ok_or_else(|| "phone returned no credential".to_owned())?;

    let credential = EnrolledCredential {
        user: user.to_owned(),
        rp_id: RP_ID.to_owned(),
        credential_id: attested.credential_id,
        public_key_cose: attested.credential_public_key,
    };
    store
        .store_credential(&credential)
        .map_err(|e| format!("cannot persist credential: {e}"))?;

    Ok(credential)
}

/// There is no web origin here. The assertion is checked against a key in our
/// own store rather than by a remote server, so origin-to-RP binding is ours
/// to assert and `Trust` is the honest expression of that. Validation against
/// the public suffix list would reject a local RP id anyway.
fn settings<'a>() -> RequestSettings<'a> {
    RequestSettings {
        origin: OriginValidation::Trust,
    }
}

fn origin() -> Result<RequestOrigin, String> {
    format!("https://{RP_ID}")
        .as_str()
        .try_into()
        .map_err(|_| format!("invalid origin for rp id {RP_ID}"))
}

fn request_json(user: &str) -> Result<String, String> {
    // Not security-critical at enrollment (nothing verifies this attestation),
    // but a predictable challenge in a stored ceremony is a bad habit to build.
    let challenge = base64url(&random_bytes::<32>()?);
    // Stable per account, so re-enrolling the same user replaces rather than
    // accumulates on the phone.
    let user_handle = base64url(user.as_bytes());
    let display = user;

    Ok(format!(
        r#"{{
    "rp": {{ "id": "{RP_ID}", "name": "swaypplet" }},
    "user": {{ "id": "{user_handle}", "name": "{user}", "displayName": "{display}" }},
    "challenge": "{challenge}",
    "pubKeyCredParams": [
        {{ "type": "public-key", "alg": -7 }},
        {{ "type": "public-key", "alg": -257 }}
    ],
    "timeout": {TIMEOUT_MS},
    "excludeCredentials": [],
    "authenticatorSelection": {{
        "residentKey": "discouraged",
        "userVerification": "required"
    }},
    "attestation": "none"
}}"#
    ))
}

/// Straight from the kernel CSPRNG. No crate needed for this, and a read that
/// returns short is treated as failure rather than quietly producing a weak
/// challenge.
fn random_bytes<const N: usize>() -> Result<[u8; N], String> {
    use std::io::Read as _;
    let mut buf = [0u8; N];
    std::fs::File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut buf))
        .map_err(|e| format!("cannot read /dev/urandom: {e}"))?;
    Ok(buf)
}

/// base64url without padding, per WebAuthn's JSON encoding.
fn base64url(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = u32::from(b[0]) << 16 | u32::from(b[1]) << 8 | u32::from(b[2]);
        let digits = [
            TABLE[(n >> 18 & 63) as usize],
            TABLE[(n >> 12 & 63) as usize],
            TABLE[(n >> 6 & 63) as usize],
            TABLE[(n & 63) as usize],
        ];
        // 1 leftover byte yields 2 digits, 2 yield 3; padding is omitted.
        let keep = chunk.len() + 1;
        out.extend(digits[..keep].iter().map(|c| *c as char));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64url_matches_rfc4648_vectors() {
        // RFC 4648 §10, with URL alphabet and no padding.
        assert_eq!(base64url(b""), "");
        assert_eq!(base64url(b"f"), "Zg");
        assert_eq!(base64url(b"fo"), "Zm8");
        assert_eq!(base64url(b"foo"), "Zm9v");
        assert_eq!(base64url(b"foob"), "Zm9vYg");
        assert_eq!(base64url(b"fooba"), "Zm9vYmE");
        assert_eq!(base64url(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn base64url_uses_the_url_alphabet() {
        // 0xfb 0xff would be "+/" in standard base64.
        assert_eq!(base64url(&[0xfb, 0xff]), "-_8");
    }

    #[test]
    fn challenges_are_random_and_full_width() {
        let a = random_bytes::<32>().unwrap();
        let b = random_bytes::<32>().unwrap();
        assert_ne!(a, b, "two reads must not match");
        assert_eq!(base64url(&a).len(), 43, "32 bytes unpadded");
    }

    #[test]
    fn request_demands_user_verification() {
        let json = request_json("meros").unwrap();
        assert!(json.contains(r#""userVerification": "required""#));
        assert!(json.contains(&format!(r#""id": "{RP_ID}""#)));
        // Two ceremonies must not share a challenge.
        assert_ne!(json, request_json("meros").unwrap());
    }
}
