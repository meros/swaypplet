//! GitHub, through the `gh` CLI.
//!
//! `gh` rather than an HTTP client: it holds the owner's credential already
//! (the same one `git push` and the runner use), it is on every host the
//! shell runs on, and it keeps a TLS stack and a token store out of this
//! binary. Every call blocks, so every caller is on a worker thread.
//!
//! Writes go through [`write`], which is where `SWAYPPLET_DRY_RUN` is
//! honoured: a dry run prints the command and its input to stderr and
//! returns a stand-in answer, so the rest of the flow runs as it would.
//!
//! # Pictures
//!
//! A screenshot has to be somewhere GitHub will render it from. The options,
//! and why the contents API won:
//!
//! - The web UI's attachment upload (`user-attachments`) has no public API.
//! - A gist holds text; a binary through the gist API is mangled.
//! - A release asset needs a release, and its URL is a redirect that
//!   issue markdown does not follow reliably.
//! - The contents API (`PUT /repos/{repo}/contents/{path}`) is documented,
//!   takes the credential `gh` already has, and a raw URL of a public
//!   repository renders in an issue through GitHub's image proxy.
//!
//! The files go to an orphan branch ([`super::ASSET_BRANCH`]), so they
//! never enter main's history or a clone of it, and the URL names the
//! commit, not the branch, so a picture cannot change under an issue.

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use super::status::{Issue, Pr};

/// A program by name: `PATH`, then the places a NixOS user profile puts
/// it. The panel runs as a systemd user unit, whose `PATH` need not include
/// the user's profile.
pub fn which(name: &str) -> Option<PathBuf> {
    let user = std::env::var("USER").unwrap_or_default();
    let home = super::home();
    let mut dirs: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).collect())
        .unwrap_or_default();
    dirs.push(PathBuf::from(format!("/etc/profiles/per-user/{user}/bin")));
    dirs.push(Path::new(&home).join(".nix-profile/bin"));
    dirs.push(PathBuf::from("/run/current-system/sw/bin"));
    dirs.into_iter().map(|d| d.join(name)).find(|p| p.is_file())
}

fn gh_path() -> PathBuf {
    std::env::var_os("SWAYPPLET_GH")
        .map(PathBuf::from)
        .or_else(|| which("gh"))
        .unwrap_or_else(|| PathBuf::from("gh"))
}

/// Run `gh`, with `input` on stdin; stdout on success, stderr on failure.
fn run(args: &[&str], input: Option<&[u8]>) -> Result<Vec<u8>, String> {
    let mut child = Command::new(gh_path())
        .args(args)
        .stdin(if input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        // Plain output, never a pager or a prompt: nobody is at this terminal.
        .env("GH_PROMPT_DISABLED", "1")
        .env("GH_NO_UPDATE_NOTIFIER", "1")
        .env("NO_COLOR", "1")
        .spawn()
        .map_err(|e| format!("gh: {e}"))?;
    if let (Some(bytes), Some(mut stdin)) = (input, child.stdin.take()) {
        stdin
            .write_all(bytes)
            .map_err(|e| format!("gh stdin: {e}"))?;
    }
    let out = child.wait_with_output().map_err(|e| format!("gh: {e}"))?;
    if out.status.success() {
        Ok(out.stdout)
    } else {
        let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
        Err(format!("gh {}: {err}", args.first().unwrap_or(&"")))
    }
}

fn read(args: &[&str]) -> Result<String, String> {
    run(args, None).map(|b| String::from_utf8_lossy(&b).to_string())
}

/// A write, or in a dry run its description on stderr and `stand_in`.
fn write(args: &[&str], input: Option<&[u8]>, stand_in: &str) -> Result<String, String> {
    if super::dry_run() {
        let mut shown = format!("DRY RUN: gh {}", shell_words(args));
        if let Some(input) = input {
            let text = String::from_utf8_lossy(input);
            let text = if text.len() > 4000 {
                let mut end = 4000;
                while !text.is_char_boundary(end) {
                    end -= 1;
                }
                format!("{}… ({} bytes)", &text[..end], input.len())
            } else {
                text.to_string()
            };
            shown.push_str(&format!("\n--- stdin ---\n{text}\n--- end ---"));
        }
        eprintln!("{shown}");
        log::info!("quality: {}", shown.lines().next().unwrap_or_default());
        return Ok(stand_in.to_string());
    }
    run(args, input).map(|b| String::from_utf8_lossy(&b).to_string())
}

fn shell_words(args: &[&str]) -> String {
    args.iter()
        .map(|a| {
            if a.chars()
                .all(|c| c.is_ascii_alphanumeric() || "-_/.:=,@+".contains(c))
            {
                (*a).to_string()
            } else {
                format!("'{}'", a.replace('\'', r"'\''"))
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// File an issue; its URL.
pub fn create_issue(title: &str, body: &str, labels: &[&str]) -> Result<String, String> {
    let repo = super::repo();
    let mut args = vec![
        "issue",
        "create",
        "--repo",
        &repo,
        "--title",
        title,
        "--body-file",
        "-",
    ];
    for label in labels {
        args.push("--label");
        args.push(label);
    }
    let stand_in = format!("https://github.com/{repo}/issues/0");
    write(&args, Some(body.as_bytes()), &stand_in).map(|out| out.trim().to_string())
}

/// Comment on an issue.
pub fn comment(number: u64, body: &str) -> Result<(), String> {
    let repo = super::repo();
    let n = number.to_string();
    write(
        &["issue", "comment", &n, "--repo", &repo, "--body-file", "-"],
        Some(body.as_bytes()),
        "",
    )
    .map(|_| ())
}

/// Squash-merge a PR and delete its branch.
pub fn merge(pr: u64) -> Result<(), String> {
    let repo = super::repo();
    let n = pr.to_string();
    write(
        &[
            "pr",
            "merge",
            &n,
            "--repo",
            &repo,
            "--squash",
            "--delete-branch",
        ],
        None,
        "",
    )
    .map(|_| ())
}

/// Close a PR without merging it, and delete its branch.
pub fn close_pr(pr: u64) -> Result<(), String> {
    let repo = super::repo();
    let n = pr.to_string();
    write(
        &["pr", "close", &n, "--repo", &repo, "--delete-branch"],
        None,
        "",
    )
    .map(|_| ())
}

/// Issues in `state` (`open`, `all`), newest first, with `label` if given.
pub fn issues(label: Option<&str>, state: &str, limit: u32) -> Result<Vec<Issue>, String> {
    let repo = super::repo();
    let limit = limit.to_string();
    let mut args = vec![
        "issue",
        "list",
        "--repo",
        &repo,
        "--state",
        state,
        "--limit",
        &limit,
        "--json",
        "number,title,state,labels,author,url,createdAt",
    ];
    if let Some(label) = label {
        args.extend(["--label", label]);
    }
    let out = read(&args)?;
    serde_json::from_str(&out).map_err(|e| format!("gh issue list: {e}"))
}

/// The runner's PRs (branches `autofix/<n>`), newest first.
pub fn prs(limit: u32) -> Result<Vec<Pr>, String> {
    let repo = super::repo();
    let limit = limit.to_string();
    let out = read(&[
        "pr",
        "list",
        "--repo",
        &repo,
        "--state",
        "all",
        "--limit",
        &limit,
        "--search",
        "head:autofix/",
        "--json",
        "number,url,state,headRefName,body",
    ])?;
    let prs: Vec<Pr> = serde_json::from_str(&out).map_err(|e| format!("gh pr list: {e}"))?;
    Ok(prs.into_iter().filter(|p| p.issue().is_some()).collect())
}

/// A file from the asset branch, raw.
pub fn fetch_asset(path: &str) -> Result<Vec<u8>, String> {
    let repo = super::repo();
    let endpoint = format!("repos/{repo}/contents/{path}?ref={}", super::ASSET_BRANCH);
    run(
        &["api", "-H", "Accept: application/vnd.github.raw", &endpoint],
        None,
    )
}

/// Put `bytes` at `path` on the asset branch; the raw URL, pinned to the
/// commit that added it.
pub fn upload_asset(path: &str, bytes: &[u8]) -> Result<String, String> {
    let repo = super::repo();
    ensure_asset_branch(&repo)?;
    let request = serde_json::json!({
        "message": format!("add {path}"),
        "content": super::base64(bytes),
        "branch": super::ASSET_BRANCH,
    })
    .to_string();
    let endpoint = format!("repos/{repo}/contents/{path}");
    let stand_in = r#"{"commit":{"sha":"DRY-RUN"}}"#;
    let out = write(
        &["api", "--method", "PUT", &endpoint, "--input", "-"],
        Some(request.as_bytes()),
        stand_in,
    )?;
    let answer: serde_json::Value =
        serde_json::from_str(&out).map_err(|e| format!("contents API: {e}"))?;
    let sha = answer["commit"]["sha"]
        .as_str()
        .ok_or("contents API: no commit in the answer")?;
    Ok(raw_url(&repo, sha, path))
}

pub fn raw_url(repo: &str, commit: &str, path: &str) -> String {
    format!("https://raw.githubusercontent.com/{repo}/{commit}/{path}")
}

/// Create the orphan asset branch the first time: a README in a commit with
/// no parent, and a ref to it. Nothing on main is touched.
fn ensure_asset_branch(repo: &str) -> Result<(), String> {
    let branch = super::ASSET_BRANCH;
    match read(&["api", &format!("repos/{repo}/branches/{branch}")]) {
        Ok(_) => return Ok(()),
        Err(e) if e.contains("404") || e.contains("Not Found") => {}
        Err(e) => return Err(e),
    }
    if super::dry_run() {
        eprintln!(
            "DRY RUN: would create the orphan branch {branch} on {repo} (tree, parentless commit, ref)"
        );
        return Ok(());
    }
    let readme = "Pictures for swaypplet's reports and auto-fix PRs \
                  (src/quality/gh.rs). Nothing here is built.\n";
    let tree = json_post(
        &format!("repos/{repo}/git/trees"),
        serde_json::json!({
            "tree": [{"path": "README.md", "mode": "100644", "type": "blob", "content": readme}]
        }),
    )?;
    let commit = json_post(
        &format!("repos/{repo}/git/commits"),
        serde_json::json!({
            "message": "report assets",
            "tree": tree["sha"],
            "parents": [],
        }),
    )?;
    json_post(
        &format!("repos/{repo}/git/refs"),
        serde_json::json!({ "ref": format!("refs/heads/{branch}"), "sha": commit["sha"] }),
    )?;
    Ok(())
}

fn json_post(endpoint: &str, body: serde_json::Value) -> Result<serde_json::Value, String> {
    let out = run(
        &["api", "--method", "POST", endpoint, "--input", "-"],
        Some(body.to_string().as_bytes()),
    )?;
    serde_json::from_slice(&out).map_err(|e| format!("{endpoint}: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_raw_url_names_the_commit() {
        assert_eq!(
            raw_url("meros/swaypplet", "abc", "reports/x.png"),
            "https://raw.githubusercontent.com/meros/swaypplet/abc/reports/x.png"
        );
    }

    #[test]
    fn dry_run_lines_quote_what_a_shell_would_split() {
        assert_eq!(
            shell_words(&["issue", "create", "--title", "report: it's broken"]),
            r"issue create --title 'report: it'\''s broken'"
        );
    }
}
