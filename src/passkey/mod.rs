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

pub mod enroll;
pub mod store;
pub mod verify;

/// `swaypplet passkey-enroll [--user NAME]` — the one attended ceremony.
///
/// Refuses to run unprivileged: the store is root-owned precisely so that a
/// compromised user session cannot enroll an authenticator, and a friendly
/// "run me with pkexec" beats a permission-denied traceback.
pub fn enroll_cli(mut args: impl Iterator<Item = String>) {
    let mut user: Option<String> = None;
    let mut allow_local = false;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--user" => user = args.next(),
            // Offer the direct BLE data channel as well as the tunnel. Older
            // Play services reject the QR key it adds, so this is opt-in.
            "--allow-local" => allow_local = true,
            other => {
                eprintln!("passkey-enroll: unexpected argument {other}");
                std::process::exit(2);
            }
        }
    }

    // SAFETY: geteuid cannot fail and touches no memory we own.
    if unsafe { libc::geteuid() } != 0 {
        eprintln!(
            "passkey-enroll must run as root; the credential store is root-owned.\n\
             Try: pkexec swaypplet passkey-enroll"
        );
        std::process::exit(1);
    }

    // Under pkexec the invoking account is the one being enrolled, not root.
    let user = user
        .or_else(|| std::env::var("PKEXEC_UID").ok().and_then(uid_to_name))
        .or_else(|| std::env::var("SUDO_USER").ok())
        .unwrap_or_else(|| {
            eprintln!("passkey-enroll: cannot tell who to enroll; pass --user NAME");
            std::process::exit(2);
        });

    println!("Enrolling a phone for {user}.");
    let outcome = enroll::run(&user, allow_local, |progress| match progress {
        enroll::Progress::ShowQr(payload) => match qrcode::QrCode::new(&payload) {
            Ok(code) => {
                use qrcode::render::unicode;
                println!(
                    "\nScan with the phone's camera:\n\n{}",
                    code.render::<unicode::Dense1x2>()
                        .dark_color(unicode::Dense1x2::Light)
                        .light_color(unicode::Dense1x2::Dark)
                        .build()
                );
            }
            // The payload is the ceremony; if only the rendering failed, the
            // user can still scan it from another QR tool rather than restart.
            Err(e) => println!("QR render failed ({e}); raw payload:\n{payload}"),
        },
        enroll::Progress::ProximityCheck => println!("Phone is nearby; checking proximity."),
        enroll::Progress::Connecting => println!("Reaching the phone."),
        enroll::Progress::Connected => println!("Connected. The QR is no longer needed."),
        enroll::Progress::AwaitingApproval => println!("Approve on the phone."),
        enroll::Progress::Note(note) => println!("({note})"),
    });

    match outcome {
        Ok(cred) => println!(
            "Enrolled {} on {}. The phone lists it as \"{}\"; unlocks from here \n\
             need no QR.",
            cred.user,
            cred.rp_id,
            store::rp_name(),
        ),
        Err(e) => {
            eprintln!("Enrollment failed: {e}");
            std::process::exit(1);
        }
    }
}

fn uid_to_name(uid: String) -> Option<String> {
    let uid: u32 = uid.parse().ok()?;
    std::fs::read_to_string("/etc/passwd").ok().and_then(|text| {
        text.lines().find_map(|line| {
            let mut f = line.split(':');
            let name = f.next()?;
            let parsed: u32 = f.nth(1)?.parse().ok()?;
            (parsed == uid).then(|| name.to_owned())
        })
    })
}

/// `swaypplet passkey-verify [--user NAME]` — contact the linked phone with
/// no QR.
///
/// The transport test for the unlock path. Prints a loud warning because it
/// does not yet verify the assertion signature, so a success here means the
/// phone answered, not that it proved possession of the key.
pub fn verify_cli(mut args: impl Iterator<Item = String>) {
    let mut user: Option<String> = None;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--user" => user = args.next(),
            other => {
                eprintln!("passkey-verify: unexpected argument {other}");
                std::process::exit(2);
            }
        }
    }

    // SAFETY: geteuid cannot fail and touches no memory we own.
    if unsafe { libc::geteuid() } != 0 {
        eprintln!(
            "passkey-verify must run as root; the credential store is root-owned.\n\
             Try: pkexec swaypplet passkey-verify"
        );
        std::process::exit(1);
    }

    let user = user
        .or_else(|| std::env::var("PKEXEC_UID").ok().and_then(uid_to_name))
        .or_else(|| std::env::var("SUDO_USER").ok())
        .unwrap_or_else(|| {
            eprintln!("passkey-verify: cannot tell whom to verify; pass --user NAME");
            std::process::exit(2);
        });

    println!("Contacting your phone for {user}. No QR: this uses the stored link.");
    let outcome = verify::run(&user, |progress| match progress {
        enroll::Progress::ShowQr(_) => {
            // The known-device path has no QR stage; if one ever appears here
            // the linking state was lost and the run is not testing what it
            // claims to.
            println!("WARNING: a QR was requested — the stored link is not being used.");
        }
        enroll::Progress::ProximityCheck => println!("Proximity check."),
        enroll::Progress::Connecting => println!("Reaching the phone."),
        enroll::Progress::Connected => println!("Connected."),
        enroll::Progress::AwaitingApproval => println!("Approve on the phone."),
        enroll::Progress::Note(note) => println!("({note})"),
    });

    match outcome {
        Ok(()) => println!(
            "\nPhone answered and verified you.\n\
             NOTE: the signature is not checked yet, so this proves the transport \
             works, not possession of the key. Not an auth path until that lands."
        ),
        Err(e) => {
            eprintln!("Verification failed: {e}");
            std::process::exit(1);
        }
    }
}
