//! Minimal `org.gnome.DisplayManager` so GNOME's "Switch User…" works on a
//! machine that runs greetd instead of GDM.
//!
//! gnome-shell routes that menu item through libgdm, which first looks for a
//! logind session with class "greeter" *and* PAM service
//! "gdm-launch-environment" — a greetd session never matches — and then falls
//! back to `LocalDisplayFactory.CreateTransientDisplay` on the system bus
//! (gdm 50, common/gdm-common.c). With no GDM that call fails and gnome-shell
//! swallows the error, so the menu item silently does nothing.
//!
//! This owns the bus name and maps that one method onto the same
//! [`switch_user::host::goto_greeter`] every other affordance uses. Runs as
//! root, D-Bus-activated on first use, resident afterwards. Locking the
//! caller's session is a no-op here (root has none) — landing on a greeter or
//! a locked session always demands the target user's own auth, which is
//! exactly GDM's model.

use zbus::zvariant::OwnedObjectPath;

use crate::switch_user::host;

const BUS_NAME: &str = "org.gnome.DisplayManager";
const OBJECT_PATH: &str = "/org/gnome/DisplayManager/LocalDisplayFactory";

struct LocalDisplayFactory;

#[zbus::interface(name = "org.gnome.DisplayManager.LocalDisplayFactory")]
impl LocalDisplayFactory {
    /// libgdm only logs the display path it gets back, so any valid object
    /// path satisfies the `(o)` reply signature.
    async fn create_transient_display(&self) -> zbus::fdo::Result<OwnedObjectPath> {
        let cfg = host::config()
            .ok_or_else(|| zbus::fdo::Error::Failed("no switch-user config on this host".into()))?;
        let conn = zbus::Connection::system().await?;
        host::goto_greeter(&conn, &cfg)
            .await
            .map_err(zbus::fdo::Error::Failed)?;
        Ok(OwnedObjectPath::try_from("/org/gnome/DisplayManager/Displays/_0").unwrap())
    }
}

pub fn run() {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");
    if let Err(e) = rt.block_on(serve()) {
        eprintln!("swaypplet gdm-shim: {e}");
        std::process::exit(1);
    }
}

async fn serve() -> zbus::Result<()> {
    let _conn = zbus::connection::Builder::system()?
        .name(BUS_NAME)?
        .serve_at(OBJECT_PATH, LocalDisplayFactory)?
        .build()
        .await?;
    log::info!("gdm-shim: owning {BUS_NAME}");
    std::future::pending::<()>().await;
    Ok(())
}
