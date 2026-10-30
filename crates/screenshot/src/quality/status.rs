//! Where an issue's fix stands, from the local auto-fix job and the PR.
//!
//! Nothing works on an issue until someone presses Auto-fix in the Quality
//! tab, which starts `dev/autofix/run.sh` as a user unit. The runner leaves
//! its state in `$XDG_STATE_HOME/swaypplet-autofix/jobs/<n>/` (a `phase`
//! line while it runs, a `result` line when it ends), and the PR on branch
//! `autofix/<n>` is what it leaves on GitHub. [`fix`] folds the two into
//! one answer, so the tab and its tests agree on it.
//!
//! ```text
//!   open ──Auto-fix──→ queued → fixing ──→ ready ──Merge & apply──→ merged
//!                                      ├─→ failed        └─Close──→ closed
//!                                      └─→ stopped (Stop)
//! ```

use serde::Deserialize;

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct Label {
    pub name: String,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
pub struct Author {
    #[serde(default)]
    pub login: String,
}

/// An issue as `gh issue list --json number,title,state,labels,author,url,createdAt` gives it.
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Issue {
    pub number: u64,
    #[serde(default)]
    pub title: String,
    /// `OPEN` or `CLOSED`.
    #[serde(default)]
    pub state: String,
    #[serde(default)]
    pub labels: Vec<Label>,
    #[serde(default)]
    pub author: Author,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub created_at: String,
}

impl Issue {
    pub fn has(&self, label: &str) -> bool {
        self.labels.iter().any(|l| l.name == label)
    }

    pub fn is_open(&self) -> bool {
        self.state.eq_ignore_ascii_case("open")
    }
}

/// Whether Auto-fix must ask first: the issue's text is the agent's input,
/// and on a public repository anyone but the owner is a stranger. An exact
/// match: `Meros` or `meros-bot` is someone else.
pub fn needs_confirm(issue: &Issue, owner: &str) -> bool {
    owner.is_empty() || issue.author.login != owner
}

/// A PR as `gh pr list --json number,url,state,headRefName,body` gives it.
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Pr {
    pub number: u64,
    #[serde(default)]
    pub url: String,
    /// `OPEN`, `MERGED` or `CLOSED`.
    #[serde(default)]
    pub state: String,
    #[serde(default)]
    pub head_ref_name: String,
    #[serde(default)]
    pub body: String,
}

impl Pr {
    /// The issue a runner branch (`autofix/<n>`) is for.
    pub fn issue(&self) -> Option<u64> {
        self.head_ref_name.strip_prefix("autofix/")?.parse().ok()
    }

    fn is(&self, state: &str) -> bool {
        self.state.eq_ignore_ascii_case(state)
    }
}

/// A local job, as the runner left it on disk.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Job {
    /// The `phase` line: `3/5 agent working`.
    pub phase: String,
    /// The `result` line, once it has ended.
    pub result: Option<String>,
    /// Seconds since the job last wrote to its log.
    pub idle_s: u64,
    /// The end of the job's log, newest last.
    pub tail: Vec<String>,
}

/// A running job silent for this long died without a result (a reboot, a
/// SIGKILL). The runner bounds the agent at 75 minutes by default, and the
/// agent's phase writes nothing to the runner's log while it works.
pub const STALE_S: u64 = 3 * 3600;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Fix {
    /// Nothing has been tried.
    None,
    /// Waiting for another job to finish.
    Queued,
    /// The runner is in this phase.
    Working(String),
    /// A PR waits for Merge & apply.
    Ready,
    Failed(String),
    Stopped,
    /// A dry run ended where the agent would start.
    DryRun,
    Merged,
    /// The PR was closed without a merge.
    Closed,
}

impl Fix {
    pub fn label(&self) -> &'static str {
        match self {
            Fix::None => "Open",
            Fix::Queued => "Queued",
            Fix::Working(_) => "Fixing",
            Fix::Ready => "Fix ready",
            Fix::Failed(_) => "Failed",
            Fix::Stopped => "Stopped",
            Fix::DryRun => "Dry run",
            Fix::Merged => "Merged",
            Fix::Closed => "Fix closed",
        }
    }
}

/// The fix's state. A running job wins (a second try on an issue with a
/// PR), then what GitHub says about the PR, then how the last job ended.
pub fn fix(job: Option<&Job>, pr: Option<&Pr>) -> Fix {
    if let Some(job) = job
        && job.result.is_none()
    {
        if job.idle_s > STALE_S {
            return Fix::Failed("the runner stopped without a result".into());
        }
        if job.phase.starts_with("1/") || job.phase.is_empty() {
            return Fix::Queued;
        }
        return Fix::Working(job.phase.clone());
    }
    if let Some(pr) = pr {
        if pr.is("merged") {
            return Fix::Merged;
        }
        if pr.is("open") {
            return Fix::Ready;
        }
    }
    if let Some(result) = job.and_then(|j| j.result.as_deref()) {
        let (word, rest) = result.split_once(' ').unwrap_or((result, ""));
        match word {
            // The PR list can lag the runner by a poll.
            "ready" => return Fix::Ready,
            "failed" => return Fix::Failed(rest.to_string()),
            "stopped" => return Fix::Stopped,
            "dry-run" => return Fix::DryRun,
            _ => {}
        }
    }
    if pr.is_some_and(|p| p.is("closed")) {
        return Fix::Closed;
    }
    Fix::None
}

/// The PR URL a `ready` result names.
pub fn ready_url(job: &Job) -> Option<&str> {
    job.result.as_deref()?.strip_prefix("ready ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn issue(author: &str) -> Issue {
        Issue {
            number: 7,
            title: "x".into(),
            state: "OPEN".into(),
            labels: vec![],
            author: Author {
                login: author.into(),
            },
            url: String::new(),
            created_at: String::new(),
        }
    }

    fn pr(state: &str) -> Pr {
        Pr {
            number: 99,
            url: "https://github.com/meros/swaypplet/pull/99".into(),
            state: state.into(),
            head_ref_name: "autofix/7".into(),
            body: String::new(),
        }
    }

    fn job(phase: &str, result: Option<&str>) -> Job {
        Job {
            phase: phase.into(),
            result: result.map(str::to_string),
            idle_s: 5,
            tail: vec![],
        }
    }

    #[test]
    fn a_stranger_is_asked_about_and_the_owner_is_not() {
        assert!(!needs_confirm(&issue("meros"), "meros"));
        assert!(needs_confirm(&issue("mallory"), "meros"));
        assert!(needs_confirm(&issue("Meros"), "meros"));
        assert!(needs_confirm(&issue("meros-bot"), "meros"));
        assert!(needs_confirm(&issue("meros"), ""));
    }

    #[test]
    fn a_job_walks_queued_working_and_its_ends() {
        assert_eq!(fix(None, None), Fix::None);
        assert_eq!(fix(Some(&job("1/5 queued", None)), None), Fix::Queued);
        assert_eq!(
            fix(Some(&job("3/5 agent working", None)), None),
            Fix::Working("3/5 agent working".into())
        );
        assert_eq!(
            fix(Some(&job("5/5", Some("ready https://x/pull/9"))), None),
            Fix::Ready
        );
        assert_eq!(
            fix(Some(&job("4/5", Some("failed check.sh failed"))), None),
            Fix::Failed("check.sh failed".into())
        );
        assert_eq!(fix(Some(&job("3/5", Some("stopped"))), None), Fix::Stopped);
        assert_eq!(fix(Some(&job("2/5", Some("dry-run"))), None), Fix::DryRun);
    }

    #[test]
    fn github_outranks_an_old_result_and_a_running_job_outranks_github() {
        let failed = job("4/5", Some("failed x"));
        assert_eq!(fix(Some(&failed), Some(&pr("OPEN"))), Fix::Ready);
        assert_eq!(fix(Some(&failed), Some(&pr("MERGED"))), Fix::Merged);
        assert_eq!(fix(None, Some(&pr("CLOSED"))), Fix::Closed);
        // A job's own end is newer news than a closed PR from before it.
        assert_eq!(
            fix(Some(&failed), Some(&pr("CLOSED"))),
            Fix::Failed("x".into())
        );
        let again = job("3/5 agent working", None);
        assert!(matches!(
            fix(Some(&again), Some(&pr("OPEN"))),
            Fix::Working(_)
        ));
    }

    #[test]
    fn a_job_that_stopped_moving_is_a_failure() {
        let mut dead = job("3/5 agent working", None);
        dead.idle_s = STALE_S + 1;
        assert!(matches!(fix(Some(&dead), None), Fix::Failed(_)));
    }

    #[test]
    fn gh_json_parses_and_the_branch_names_the_issue() {
        let json = r#"[{"number":7,"title":"crash: x","state":"OPEN","labels":[{"id":"L","name":"crash","description":"","color":"x"}],"author":{"id":"U","is_bot":false,"login":"meros","name":"M"},"url":"https://github.com/meros/swaypplet/issues/7","createdAt":"2026-09-27T10:00:00Z"}]"#;
        let issues: Vec<Issue> = serde_json::from_str(json).unwrap();
        assert!(issues[0].has("crash"));
        assert!(!needs_confirm(&issues[0], "meros"));
        assert_eq!(pr("OPEN").issue(), Some(7));
        let mut other = pr("OPEN");
        other.head_ref_name = "feature/7".into();
        assert_eq!(other.issue(), None);
        assert_eq!(
            ready_url(&job("5/5", Some("ready https://x/pull/9"))),
            Some("https://x/pull/9")
        );
    }
}
