//! The text of an issue: a report's and a crash's, and the markers the
//! fixer and the de-duplication read back out of them.
//!
//! Pure, so what goes onto a public page is tested rather than trusted.

/// One output as the report saw it.
#[derive(Clone, Debug, PartialEq)]
pub struct Output {
    pub name: String,
    pub width: i32,
    pub height: i32,
    pub scale: f64,
}

/// What a report says besides the description.
#[derive(Clone, Debug, Default)]
pub struct Report {
    pub description: String,
    pub rev: String,
    pub outputs: Vec<Output>,
    /// `dark` or `light`: the mode on screen when it was filed.
    pub mode: String,
    /// Already redacted, oldest first. Empty when the switch was off.
    pub log: Vec<String>,
    /// The picture's URL on the asset branch, when one was attached.
    pub screenshot: Option<String>,
}

/// Marks an issue body as written by this module, so the runner can tell a
/// report from an issue someone typed by hand.
pub const REPORT_MARKER: &str = "<!-- swaypplet-report v1 -->";

/// How long a title may be before it is cut, in characters.
const TITLE_CHARS: usize = 72;

/// `report: <the description's first line>`, cut at a word.
pub fn report_title(description: &str) -> String {
    let first = description
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("no description");
    format!("report: {}", cut(first, TITLE_CHARS))
}

pub fn report_body(r: &Report) -> String {
    let mut out = String::new();
    out.push_str(REPORT_MARKER);
    out.push_str("\n\n");
    let description = r.description.trim();
    if description.is_empty() {
        out.push_str("_No description._\n");
    } else {
        out.push_str(description);
        out.push('\n');
    }
    if let Some(url) = &r.screenshot {
        out.push_str(&format!("\n![screenshot]({url})\n"));
    }
    out.push_str("\n| | |\n|---|---|\n");
    out.push_str(&format!("| revision | `{}` |\n", r.rev));
    out.push_str(&format!("| mode | {} |\n", r.mode));
    for o in &r.outputs {
        out.push_str(&format!(
            "| output {} | {}×{} at scale {} |\n",
            o.name, o.width, o.height, o.scale
        ));
    }
    if !r.log.is_empty() {
        out.push_str(&format!(
            "\n<details><summary>The last {} log lines</summary>\n\n",
            r.log.len()
        ));
        out.push_str(&fenced(&r.log.join("\n")));
        out.push_str("\n</details>\n");
    }
    out
}

/// A crash, as `crash::run` collected it.
#[derive(Clone, Debug, Default)]
pub struct Crash {
    /// `crash::Signature::key`: what makes two crashes the same crash.
    pub signature: String,
    pub rev: String,
    /// The one-line reason: the panic message, or the signal.
    pub summary: String,
    /// The panic file (message, location, backtrace), when there was one.
    pub panic: Option<String>,
    /// The crashing thread's stack from coredumpctl, when there was one.
    pub stack: Option<String>,
    /// The unit's journal tail, redacted.
    pub journal: Vec<String>,
}

/// The line that makes an issue findable by signature (`crash::duplicate`).
pub fn signature_marker(hash: u64) -> String {
    format!("<!-- crash-signature: {hash:016x} -->")
}

pub fn crash_title(c: &Crash) -> String {
    format!("crash: {}", cut(&c.summary, TITLE_CHARS))
}

pub fn crash_body(c: &Crash, hash: u64) -> String {
    let mut out = String::new();
    out.push_str(&signature_marker(hash));
    out.push_str("\n\n");
    out.push_str(&format!(
        "swaypplet.service stopped: **{}**\n\n",
        one_line(&c.summary)
    ));
    out.push_str(&format!("| revision | `{}` |\n|---|---|\n", c.rev));
    out.push_str(&format!("| signature | `{}` |\n", one_line(&c.signature)));
    if let Some(panic) = &c.panic {
        out.push_str("\n### Panic\n\n");
        out.push_str(&fenced(panic.trim_end()));
        out.push('\n');
    }
    if let Some(stack) = &c.stack {
        out.push_str("\n### Stack of the crashing thread\n\n");
        out.push_str(&fenced(stack.trim_end()));
        out.push('\n');
    }
    if !c.journal.is_empty() {
        out.push_str(&format!(
            "\n<details><summary>The last {} journal lines</summary>\n\n",
            c.journal.len()
        ));
        out.push_str(&fenced(&c.journal.join("\n")));
        out.push_str("\n</details>\n");
    }
    out
}

/// The comment on an open crash issue when the same signature comes back.
pub fn seen_again(rev: &str, when: &str) -> String {
    format!("Seen again at {when}, revision `{rev}`.")
}

/// `text` in a code fence that nothing inside it can close: one backtick
/// longer than the longest run in the text. A log line is not trusted to
/// stay inside three.
pub fn fenced(text: &str) -> String {
    let mut longest = 0;
    let mut run = 0;
    for c in text.chars() {
        if c == '`' {
            run += 1;
            longest = longest.max(run);
        } else {
            run = 0;
        }
    }
    let fence = "`".repeat((longest + 1).max(3));
    format!("{fence}text\n{text}\n{fence}\n")
}

fn one_line(s: &str) -> String {
    s.lines().next().unwrap_or_default().replace('`', "'")
}

/// At most `max` characters, cut at the last space when there is one in the
/// second half, with an ellipsis.
pub fn cut(s: &str, max: usize) -> String {
    let s = s.trim();
    if s.chars().count() <= max {
        return s.to_string();
    }
    let head: String = s.chars().take(max - 1).collect();
    let at = head
        .rfind(' ')
        .filter(|i| *i > head.len() / 2)
        .unwrap_or(head.len());
    format!("{}\u{2026}", head[..at].trim_end())
}

/// The picture paths a runner PR names in its body, as
/// `<!-- autofix-assets before=reports/a.png after=reports/b.png -->`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Assets {
    pub before: Vec<String>,
    pub after: Vec<String>,
}

pub fn parse_assets(body: &str) -> Assets {
    let mut assets = Assets::default();
    for line in body.lines() {
        let Some(rest) = line.trim().strip_prefix("<!-- autofix-assets") else {
            continue;
        };
        let rest = rest.trim_end_matches("-->");
        for word in rest.split_whitespace() {
            // Only paths on the asset branch's own folder: the tab fetches
            // these through the contents API, and a PR body is not a place
            // to take arbitrary paths from.
            let safe = |p: &str| {
                p.starts_with("reports/")
                    && !p.contains("..")
                    && p.chars()
                        .all(|c| c.is_ascii_alphanumeric() || "/-_.".contains(c))
            };
            if let Some(p) = word.strip_prefix("before=").filter(|p| safe(p)) {
                assets.before.push(p.to_string());
            } else if let Some(p) = word.strip_prefix("after=").filter(|p| safe(p)) {
                assets.after.push(p.to_string());
            }
        }
    }
    assets
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report() -> Report {
        Report {
            description: "The clock overlaps the battery\n\nAt 125 % scale.".into(),
            rev: "abc123def456".into(),
            outputs: vec![Output {
                name: "eDP-1".into(),
                width: 2880,
                height: 1800,
                scale: 1.25,
            }],
            mode: "dark".into(),
            log: vec!["12:00:01 INFO  bar: clock tick".into()],
            screenshot: Some(
                "https://raw.githubusercontent.com/meros/swaypplet/0123/reports/x.png".into(),
            ),
        }
    }

    #[test]
    fn a_report_carries_every_fact_and_the_marker() {
        let body = report_body(&report());
        assert!(body.starts_with(REPORT_MARKER));
        assert!(body.contains("The clock overlaps the battery\n\nAt 125 % scale."));
        assert!(body.contains("![screenshot](https://raw.githubusercontent.com/"));
        assert!(body.contains("| revision | `abc123def456` |"));
        assert!(body.contains("| mode | dark |"));
        assert!(body.contains("| output eDP-1 | 2880×1800 at scale 1.25 |"));
        assert!(body.contains("The last 1 log lines"));
        assert!(body.contains("```text\n12:00:01 INFO  bar: clock tick\n```"));
    }

    #[test]
    fn a_report_without_picture_or_log_says_less() {
        let r = Report {
            screenshot: None,
            log: Vec::new(),
            description: "  ".into(),
            ..report()
        };
        let body = report_body(&r);
        assert!(body.contains("_No description._"));
        assert!(!body.contains("![screenshot]"));
        assert!(!body.contains("<details>"));
    }

    #[test]
    fn a_log_line_cannot_close_its_fence() {
        let f = fenced("before\n```\n# injected heading\n````x");
        assert!(f.starts_with("`````text\n"), "{f}");
        assert!(f.ends_with("\n`````\n"));
    }

    #[test]
    fn titles_take_the_first_line_and_are_cut_at_a_word() {
        assert_eq!(
            report_title("\n  Bar flickers \nmore"),
            "report: Bar flickers"
        );
        assert_eq!(report_title(""), "report: no description");
        let long = "word ".repeat(40);
        let t = report_title(&long);
        assert!(t.chars().count() <= "report: ".len() + TITLE_CHARS, "{t}");
        assert!(t.ends_with("word\u{2026}"), "{t}");
    }

    #[test]
    fn a_crash_body_leads_with_its_signature_marker() {
        let c = Crash {
            signature: "panic src/bar/clock.rs:88 index out of bounds".into(),
            rev: "abc".into(),
            summary: "panicked at src/bar/clock.rs:88: index out of bounds".into(),
            panic: Some("message: index out of bounds\nbacktrace:\n  0: x".into()),
            stack: None,
            journal: vec!["line".into()],
        };
        let body = crash_body(&c, 0xdead_beef);
        assert!(body.starts_with("<!-- crash-signature: 00000000deadbeef -->"));
        assert!(body.contains("### Panic"));
        assert!(!body.contains("### Stack"));
        assert!(body.contains("The last 1 journal lines"));
        assert_eq!(
            crash_title(&c),
            "crash: panicked at src/bar/clock.rs:88: index out of bounds"
        );
    }

    #[test]
    fn assets_are_read_from_the_marker_and_nothing_else() {
        let body = "Fixes #12\n\n<!-- autofix-assets before=reports/12-before-dark.png after=reports/12-after-dark.png before=../../etc/passwd after=http://x/y.png -->\n![b](x)";
        let a = parse_assets(body);
        assert_eq!(a.before, vec!["reports/12-before-dark.png"]);
        assert_eq!(a.after, vec!["reports/12-after-dark.png"]);
        assert_eq!(parse_assets("no marker"), Assets::default());
    }
}
