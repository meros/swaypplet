//! On-disk state for phone-as-passkey authentication.
//!
//! Two records live under [`STATE_DIR`]: the caBLE linking state (which phone,
//! and the secret used to contact it again without a QR) and the enrolled
//! credential (which key is allowed to unlock this machine).
//!
//! # Why the permissions matter more than the crypto
//!
//! Write access to this directory is a complete authentication bypass. An
//! attacker who can add a credential here has enrolled their own phone and
//! owns every surface that trusts this store. So: root-owned, 0700 on the
//! directory, 0600 on the files, and enrollment itself gated behind pkexec so
//! a compromised user session cannot quietly add an authenticator.
//!
//! Nothing secret is logged. The link secret and contact id never leave this
//! module except into libwebauthn.

use std::collections::HashMap;
use std::fs;
use std::io;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

use async_trait::async_trait;
use libwebauthn::transport::cable::known_devices::{
    CableKnownDeviceId, CableKnownDeviceInfo, CableKnownDeviceInfoStore,
};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

/// Root-owned state directory. Not under $HOME on purpose: the greeter
/// authenticates before any home directory is mounted, so anything it needs
/// must live somewhere readable at that point.
pub const STATE_DIR: &str = "/var/lib/swaypplet/passkey";

const DEVICES_FILE: &str = "devices.json";
const CREDENTIAL_FILE: &str = "credential.json";

/// Relying-party id for local authentication. Fixed, local, and never a real
/// domain: there is no web origin here, and the assertion is verified against
/// a key in this store rather than by a remote server. Changing this value
/// invalidates every enrolled credential.
pub const RP_ID: &str = "swaypplet.local";

/// Serde mirror of [`CableKnownDeviceInfo`], which carries no derives of its
/// own. Byte arrays become `Vec<u8>` because serde's array impls stop at 32
/// and `public_key` is 65 bytes.
#[derive(Serialize, Deserialize, Clone, Debug)]
struct StoredDevice {
    contact_id: Vec<u8>,
    link_id: Vec<u8>,
    link_secret: Vec<u8>,
    public_key: Vec<u8>,
    name: String,
    tunnel_domain: String,
}

impl From<&CableKnownDeviceInfo> for StoredDevice {
    fn from(d: &CableKnownDeviceInfo) -> Self {
        Self {
            contact_id: d.contact_id.clone(),
            link_id: d.link_id.to_vec(),
            link_secret: d.link_secret.to_vec(),
            public_key: d.public_key.to_vec(),
            name: d.name.clone(),
            tunnel_domain: d.tunnel_domain.clone(),
        }
    }
}

impl StoredDevice {
    /// Rejects records whose fixed-width fields are the wrong length rather
    /// than padding them: a truncated link secret is corruption, and guessing
    /// at it would produce a device that fails to contact with no explanation.
    fn to_info(&self) -> Option<CableKnownDeviceInfo> {
        Some(CableKnownDeviceInfo {
            contact_id: self.contact_id.clone(),
            link_id: self.link_id.as_slice().try_into().ok()?,
            link_secret: self.link_secret.as_slice().try_into().ok()?,
            public_key: self.public_key.as_slice().try_into().ok()?,
            name: self.name.clone(),
            tunnel_domain: self.tunnel_domain.clone(),
        })
    }
}

/// The credential this machine will accept. One per enrollment; re-enrolling
/// replaces it.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct EnrolledCredential {
    /// Local account this credential authenticates.
    pub user: String,
    pub rp_id: String,
    /// Pinned in `allowCredentials` at assertion time and re-checked in the
    /// response, so no other credential the phone holds can be substituted.
    pub credential_id: Vec<u8>,
    /// COSE-encoded public key the assertion signature is verified against.
    pub public_key_cose: Vec<u8>,
}

/// File-backed [`CableKnownDeviceInfoStore`].
///
/// libwebauthn calls into this during a QR ceremony when the phone offers
/// linking information, which is what makes every later unlock QR-free.
#[derive(Debug)]
pub struct FileDeviceStore {
    dir: PathBuf,
    cache: Mutex<HashMap<CableKnownDeviceId, StoredDevice>>,
}

impl FileDeviceStore {
    /// Opens the store, creating the directory 0700 if absent. A missing or
    /// unparseable devices file yields an empty store rather than an error:
    /// the caller's next step is enrollment either way.
    pub fn open(dir: impl AsRef<Path>) -> io::Result<Self> {
        let dir = dir.as_ref().to_path_buf();
        fs::create_dir_all(&dir)?;
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o700))?;
        let cache = fs::read(dir.join(DEVICES_FILE))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        Ok(Self {
            dir,
            cache: Mutex::new(cache),
        })
    }

    pub fn open_default() -> io::Result<Self> {
        Self::open(STATE_DIR)
    }

    /// Every linked phone, in a form libwebauthn can contact. Records that
    /// fail [`StoredDevice::to_info`] are dropped.
    pub async fn devices(&self) -> Vec<(CableKnownDeviceId, CableKnownDeviceInfo)> {
        self.cache
            .lock()
            .await
            .iter()
            .filter_map(|(id, d)| d.to_info().map(|info| (id.clone(), info)))
            .collect()
    }

    pub async fn is_linked(&self) -> bool {
        !self.cache.lock().await.is_empty()
    }

    async fn flush(&self) {
        let snapshot = self.cache.lock().await.clone();
        match serde_json::to_vec_pretty(&snapshot) {
            Ok(bytes) => {
                if let Err(e) = write_private(&self.dir.join(DEVICES_FILE), &bytes) {
                    log::error!("passkey: cannot persist linked devices: {e}");
                }
            }
            Err(e) => log::error!("passkey: cannot serialize linked devices: {e}"),
        }
    }

    pub fn load_credential(&self) -> Option<EnrolledCredential> {
        let bytes = fs::read(self.dir.join(CREDENTIAL_FILE)).ok()?;
        serde_json::from_slice(&bytes).ok()
    }

    pub fn store_credential(&self, cred: &EnrolledCredential) -> io::Result<()> {
        let bytes = serde_json::to_vec_pretty(cred)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        write_private(&self.dir.join(CREDENTIAL_FILE), &bytes)
    }
}

#[async_trait]
impl CableKnownDeviceInfoStore for FileDeviceStore {
    async fn put_known_device(&self, device_id: &CableKnownDeviceId, device: &CableKnownDeviceInfo) {
        // No trace of the device contents here: the record contains the link
        // secret.
        log::debug!("passkey: linking device {device_id}");
        self.cache
            .lock()
            .await
            .insert(device_id.clone(), StoredDevice::from(device));
        self.flush().await;
    }

    async fn delete_known_device(&self, device_id: &CableKnownDeviceId) {
        log::debug!("passkey: unlinking device {device_id}");
        self.cache.lock().await.remove(device_id);
        self.flush().await;
    }
}

/// Write 0600, atomically. The temp file is created inside the same directory
/// so the rename cannot cross a filesystem, and its permissions are set before
/// any content lands in it — otherwise the secret exists world-readable for
/// the width of the write.
fn write_private(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let tmp = path.with_extension("tmp");
    {
        use std::io::Write as _;
        let mut f = fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
    }
    fs::set_permissions(&tmp, fs::Permissions::from_mode(0o600))?;
    fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info() -> CableKnownDeviceInfo {
        CableKnownDeviceInfo {
            contact_id: vec![1, 2, 3],
            link_id: [4u8; 8],
            link_secret: [5u8; 32],
            public_key: [6u8; 65],
            name: "Phone".to_owned(),
            tunnel_domain: "cable.example".to_owned(),
        }
    }

    #[test]
    fn stored_device_round_trips_through_serde() {
        let stored = StoredDevice::from(&info());
        let json = serde_json::to_vec(&stored).unwrap();
        let back: StoredDevice = serde_json::from_slice(&json).unwrap();
        let recovered = back.to_info().expect("fixed-width fields intact");
        assert_eq!(recovered.link_secret, info().link_secret);
        assert_eq!(recovered.public_key, info().public_key);
        assert_eq!(recovered.tunnel_domain, info().tunnel_domain);
    }

    #[test]
    fn truncated_records_are_rejected_not_padded() {
        let mut stored = StoredDevice::from(&info());
        stored.link_secret.truncate(31);
        assert!(stored.to_info().is_none());
    }

    #[test]
    fn secrets_land_on_disk_unreadable_to_others() {
        let dir = std::env::temp_dir().join(format!("swaypplet-passkey-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let store = FileDeviceStore::open(&dir).unwrap();
        store
            .store_credential(&EnrolledCredential {
                user: "meros".to_owned(),
                rp_id: RP_ID.to_owned(),
                credential_id: vec![9; 16],
                public_key_cose: vec![8; 77],
            })
            .unwrap();

        let dir_mode = fs::metadata(&dir).unwrap().permissions().mode() & 0o777;
        let file_mode = fs::metadata(dir.join(CREDENTIAL_FILE))
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(dir_mode, 0o700, "state dir must not be traversable");
        assert_eq!(file_mode, 0o600, "credential must not be readable");
        assert_eq!(store.load_credential().unwrap().user, "meros");
        let _ = fs::remove_dir_all(&dir);
    }
}
