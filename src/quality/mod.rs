//! Quality: reports, crashes, and the fixes an agent prepares for them.
//!
//! ```text
//!   swaypplet report ──→ card ──→ issue [report]
//!   OnFailure=swaypplet-crash-report ──→ issue [crash], or "seen again"
//!                                        on the open one
//!   Settings → Quality: every open issue
//!        Auto-fix ──→ systemd-run dev/autofix/run.sh --issue n
//!                     worktree, claude -p, tests, gate, before/after
//!                     ──→ PR "Fixes #n"
//!        Merge & apply ──→ gh pr merge --squash ──→ nx
//! ```
//!
//! Everything talks to GitHub through `gh` (`gh.rs`), so there is one
//! credential and one place a dry run is decided: `SWAYPPLET_DRY_RUN=1`
//! prints every write instead of sending it. Reads still go out.
//!
//! The repository is public and anyone can file an issue on it. Issue text
//! is input to an agent, so nothing runs until the owner presses Auto-fix,
//! and on an issue someone else filed the button names the author and asks
//! first (`status::needs_confirm`).

pub mod body;
pub mod crash;
pub mod gh;
pub mod logring;
pub mod report;
pub mod status;

use std::path::PathBuf;

/// Where reports and crashes go. `SWAYPPLET_QUALITY_REPO` points a test at
/// a fork.
pub fn repo() -> String {
    std::env::var("SWAYPPLET_QUALITY_REPO")
        .ok()
        .filter(|r| r.contains('/'))
        .unwrap_or_else(|| "meros/swaypplet".into())
}

/// The only author whose issues the fixer acts on: the repository's owner.
pub fn owner() -> String {
    let repo = repo();
    repo.split('/').next().unwrap_or_default().to_string()
}

/// `SWAYPPLET_DRY_RUN=1`: print every write to GitHub instead of sending it.
pub fn dry_run() -> bool {
    std::env::var_os("SWAYPPLET_DRY_RUN").is_some_and(|v| !v.is_empty() && v != "0")
}

/// The git revision this binary was built from, as a short hash: the
/// flake passes the full one (`SWAYPPLET_REV`, flake.nix, which the System
/// tab reads too); a `cargo build` has none.
pub fn rev() -> &'static str {
    short_rev(option_env!("SWAYPPLET_REV"))
}

fn short_rev(rev: Option<&'static str>) -> &'static str {
    match rev {
        Some(r) if r.len() >= 12 => &r[..12],
        Some(r) if !r.is_empty() => r,
        _ => "unknown (a cargo build)",
    }
}

/// The labels a report and a crash are filed with.
pub const REPORT: &str = "report";
pub const CRASH: &str = "crash";

/// The orphan branch the pictures live on (`gh::upload_asset`, for why).
pub const ASSET_BRANCH: &str = "report-assets";

/// How many log lines a report or a crash carries.
pub const LOG_LINES: usize = 80;

/// `$XDG_STATE_HOME/swaypplet/<leaf>`.
pub fn state_dir(leaf: &str) -> PathBuf {
    base_dir("XDG_STATE_HOME", ".local/state").join(leaf)
}

/// `$XDG_CACHE_HOME/swaypplet/<leaf>`.
pub fn cache_dir(leaf: &str) -> PathBuf {
    base_dir("XDG_CACHE_HOME", ".cache").join(leaf)
}

fn base_dir(var: &str, fallback: &str) -> PathBuf {
    let base = std::env::var_os(var)
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var_os("HOME").unwrap_or_else(|| "/tmp".into())).join(fallback)
        });
    base.join("swaypplet")
}

/// A line for a public issue: the home directory as `~`, so the account's
/// path is not in it. The rest of a line is what the log said.
pub fn redact(line: &str, home: &str) -> String {
    if home.len() > 1 {
        line.replace(home, "~")
    } else {
        line.to_string()
    }
}

/// `$HOME`, for [`redact`].
pub fn home() -> String {
    std::env::var("HOME").unwrap_or_default()
}

/// Standard base64, for the contents API (no crate for thirty lines).
pub fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            chunk.get(1).copied().unwrap_or(0),
            chunk.get(2).copied().unwrap_or(0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(ALPHABET[((n >> (18 - 6 * i)) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// FNV-1a, 64 bit: a stable hash for a crash signature. Not for secrets.
pub fn fnv64(s: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in s.bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h
}

/// A file name for a picture on the asset branch: sortable, unique enough.
pub fn stamp() -> String {
    glib::DateTime::now_utc()
        .and_then(|t| t.format("%Y%m%d-%H%M%S"))
        .map(|s| s.to_string())
        .unwrap_or_else(|_| "unknown".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_matches_the_rfc_vectors() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foob"), "Zm9vYg==");
        assert_eq!(base64(b"fooba"), "Zm9vYmE=");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
        assert_eq!(base64(&[0xff, 0xfe, 0xfd]), "//79");
    }

    #[test]
    fn fnv_is_stable_and_spreads() {
        assert_eq!(fnv64(""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(fnv64("a"), 0xaf63_dc4c_8601_ec8c);
        assert_ne!(fnv64("panic at a.rs:1"), fnv64("panic at a.rs:2"));
    }

    #[test]
    fn the_revision_is_short_or_says_why_it_is_missing() {
        assert_eq!(
            short_rev(Some("0123456789abcdef0123456789abcdef01234567")),
            "0123456789ab"
        );
        assert_eq!(short_rev(Some("abc-dirty")), "abc-dirty");
        assert_eq!(short_rev(Some("")), "unknown (a cargo build)");
        assert_eq!(short_rev(None), "unknown (a cargo build)");
    }

    #[test]
    fn the_home_directory_leaves_a_public_line() {
        assert_eq!(
            redact("could not read /home/ada/.config/x", "/home/ada"),
            "could not read ~/.config/x"
        );
        // An empty or root home redacts nothing rather than every slash.
        assert_eq!(redact("/etc/x", "/"), "/etc/x");
        assert_eq!(redact("/etc/x", ""), "/etc/x");
    }

    #[test]
    fn the_owner_is_the_repository_owner() {
        // The default; SWAYPPLET_QUALITY_REPO is not set under cargo test.
        if std::env::var_os("SWAYPPLET_QUALITY_REPO").is_none() {
            assert_eq!(repo(), "meros/swaypplet");
            assert_eq!(owner(), "meros");
        }
    }
}
