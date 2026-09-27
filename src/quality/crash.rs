//! Crashes: the panic hook that leaves a note, and `swaypplet crash-report`,
//! which systemd runs when swaypplet.service fails (`OnFailure=`).
//!
//! A panic inside a GTK callback cannot unwind, so it aborts, and the core
//! dump's stack starts in `abort`: the message and the Rust location are
//! gone by then. The hook writes them to a file first
//! (`$XDG_STATE_HOME/swaypplet/panics/`), and the reporter prefers that
//! file over the dump. A signal with no panic (a segfault in a C library)
//! has only the dump, which `coredumpctl` symbolizes as long as the binary
//! keeps its symbols (Cargo.toml's release profile, and `dontStrip` in the
//! Nix builds).
//!
//! The same crash twice is one issue: the signature (where it happened,
//! not when or at which address) is hashed into a marker in the issue body,
//! and an open `crash` issue by the owner with that marker gets a "seen
//! again" comment instead of a sibling.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use super::body::{self, Crash};

/// How recent a panic file or a core dump must be to belong to this failure.
const RECENT: Duration = Duration::from_secs(10 * 60);

/// Panic files kept, newest first; older ones are removed.
const KEEP_PANICS: usize = 10;

fn panic_dir() -> PathBuf {
    super::state_dir("panics")
}

/// Install the hook. Every process gets it; only the panel's failures reach
/// the reporter, and a note from any other process ages out.
pub fn install_panic_hook() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let location = info
            .location()
            .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()))
            .unwrap_or_default();
        let message = info
            .payload()
            .downcast_ref::<&str>()
            .map(|s| (*s).to_string())
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "(no message)".into());
        let thread = std::thread::current()
            .name()
            .unwrap_or("unnamed")
            .to_string();
        let args: Vec<String> = std::env::args().collect();
        let note = panic_note(
            &message,
            &location,
            &thread,
            &args.join(" "),
            &std::backtrace::Backtrace::force_capture().to_string(),
        );
        let dir = panic_dir();
        if std::fs::create_dir_all(&dir).is_ok() {
            let unix = SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or_default();
            let _ = std::fs::write(dir.join(format!("{unix}-{}.txt", std::process::id())), note);
            prune(&dir);
        }
        previous(info);
    }));
}

/// What the hook writes, and what [`from_panic`] reads back.
fn panic_note(message: &str, location: &str, thread: &str, args: &str, backtrace: &str) -> String {
    format!(
        "message: {message}\nlocation: {location}\nthread: {thread}\nargs: {args}\nrev: {}\n\nbacktrace:\n{backtrace}",
        super::rev()
    )
}

fn prune(dir: &Path) {
    let mut files = panic_files(dir);
    files.sort_by(|a, b| b.1.cmp(&a.1));
    for (path, _) in files.into_iter().skip(KEEP_PANICS) {
        let _ = std::fs::remove_file(path);
    }
}

fn panic_files(dir: &Path) -> Vec<(PathBuf, SystemTime)> {
    std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "txt"))
        .filter_map(|e| Some((e.path(), e.metadata().ok()?.modified().ok()?)))
        .collect()
}

// ── Signatures ──────────────────────────────────────────────────────────

/// What makes two crashes the same crash.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Signature {
    /// Human-readable, and the thing that is hashed.
    pub key: String,
    /// One line for the title.
    pub summary: String,
}

impl Signature {
    pub fn hash(&self) -> u64 {
        super::fnv64(&self.key)
    }
}

/// From a panic note: the location (file and line, not column: a
/// reformatted line is the same bug) and the message with its numbers
/// taken out (an index or a length differs from run to run).
pub fn from_panic(note: &str) -> Option<Signature> {
    let field = |name: &str| {
        note.lines()
            .find_map(|l| l.strip_prefix(&format!("{name}: ")))
            .map(str::trim)
    };
    let message = field("message")?;
    let location = field("location").unwrap_or("");
    let file_line = location.rsplit_once(':').map_or(location, |(fl, _)| fl);
    Some(Signature {
        key: format!("panic {file_line} {}", without_numbers(message)),
        summary: format!("panicked at {file_line}: {message}"),
    })
}

/// From coredumpctl's `info`: the signal and the first three frames of the
/// crashing thread that are in swaypplet itself (or, when none are, the
/// first three frames), by function name. Addresses and symbol hashes
/// differ between builds and are left out.
pub fn from_coredump(info: &str) -> Option<Signature> {
    let signal = info
        .lines()
        .find_map(|l| l.trim().strip_prefix("Signal: "))
        .map(|s| {
            // "11 (SEGV) si_code: SI_KERNEL" → "SIGSEGV"
            s.split_once('(')
                .and_then(|(_, rest)| rest.split_once(')'))
                .map(|(name, _)| format!("SIG{name}"))
                .unwrap_or_else(|| s.to_string())
        })?;
    let frames = stack_frames(info);
    // A dump systemd-coredump could not unwind (a truncated core, a jump to
    // a wild address) names no function: then it says nothing the journal's
    // exit line does not, and every such crash would share one signature.
    if frames.iter().all(|f| f.function == "?") {
        return None;
    }
    let ours: Vec<&Frame> = frames.iter().filter(|f| f.module == "swaypplet").collect();
    let chosen: Vec<String> = if ours.is_empty() {
        frames.iter().take(3).map(|f| f.function.clone()).collect()
    } else {
        ours.iter().take(3).map(|f| f.function.clone()).collect()
    };
    let top = chosen.first().cloned().unwrap_or_else(|| "?".into());
    Some(Signature {
        key: format!("{signal} {}", chosen.join(" < ")),
        summary: format!("{signal} in {top}"),
    })
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Frame {
    pub function: String,
    pub module: String,
}

/// The first thread's frames from coredumpctl's stack trace: lines like
/// `#3  0x000055d1c0e8b3a4 _ZN9swaypplet3bar5clock4tick17h0123456789abcdefE (swaypplet + 0x2a93a4)`.
pub fn stack_frames(info: &str) -> Vec<Frame> {
    let mut frames = Vec::new();
    let mut in_stack = false;
    for line in info.lines() {
        let t = line.trim();
        if t.starts_with("Stack trace of thread") {
            if in_stack {
                break; // only the first (crashing) thread
            }
            in_stack = true;
            continue;
        }
        if !in_stack {
            continue;
        }
        let Some(rest) = t.strip_prefix('#') else {
            if t.is_empty() && !frames.is_empty() {
                break;
            }
            continue;
        };
        let mut words = rest.split_whitespace();
        let _index = words.next();
        let _address = words.next();
        let remainder: Vec<&str> = words.collect();
        let remainder = remainder.join(" ");
        let (function, module) = match remainder.rsplit_once(" (") {
            Some((f, m)) => (f.trim(), m.trim_end_matches(')')),
            None => ("", remainder.trim_start_matches('(').trim_end_matches(')')),
        };
        let module = module.split(" + ").next().unwrap_or("").trim();
        let module = module.rsplit('/').next().unwrap_or(module);
        let function = if function.is_empty() || function == "n/a" {
            "?".to_string()
        } else {
            demangle(function)
        };
        frames.push(Frame {
            function,
            module: module.to_string(),
        });
    }
    frames
}

/// A legacy-mangled Rust symbol (`_ZN…17h<hash>E`) as a path, without the
/// hash; a demangled one loses its `::h<hash>` too. Anything else as it is.
pub fn demangle(symbol: &str) -> String {
    let strip_hash = |parts: &mut Vec<String>| {
        if parts.last().is_some_and(|p| {
            p.len() == 17 && p.starts_with('h') && p[1..].chars().all(|c| c.is_ascii_hexdigit())
        }) {
            parts.pop();
        }
    };
    if let Some(body) = symbol.strip_prefix("_ZN").and_then(|s| s.strip_suffix('E')) {
        let mut parts = Vec::new();
        let mut rest = body;
        while !rest.is_empty() {
            let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
            let Ok(len) = digits.parse::<usize>() else {
                return symbol.to_string();
            };
            rest = &rest[digits.len()..];
            if rest.len() < len {
                return symbol.to_string();
            }
            parts.push(unescape(&rest[..len]));
            rest = &rest[len..];
        }
        strip_hash(&mut parts);
        return parts.join("::");
    }
    let mut parts: Vec<String> = symbol.split("::").map(str::to_string).collect();
    strip_hash(&mut parts);
    parts.join("::")
}

/// The legacy mangling's escapes for the characters a path segment needs.
fn unescape(segment: &str) -> String {
    let s = segment
        .strip_prefix("_$")
        .map_or(segment.to_string(), |s| format!("${s}"));
    s.replace("$LT$", "<")
        .replace("$GT$", ">")
        .replace("$u20$", " ")
        .replace("$u27$", "'")
        .replace("$u5b$", "[")
        .replace("$u5d$", "]")
        .replace("$u7b$", "{")
        .replace("$u7d$", "}")
        .replace("$RF$", "&")
        .replace("$BP$", "*")
        .replace("$LP$", "(")
        .replace("$RP$", ")")
        .replace("$C$", ",")
        .replace("..", "::")
}

fn without_numbers(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_number = false;
    for c in s.chars() {
        if c.is_ascii_digit() {
            if !in_number {
                out.push('N');
            }
            in_number = true;
        } else {
            in_number = false;
            out.push(c);
        }
    }
    out
}

// ── De-duplication ──────────────────────────────────────────────────────

/// The open crash issue by `owner` that already carries `hash`, if any.
/// Someone else's issue with the marker pasted in does not swallow a crash.
pub fn duplicate(issues: &[(super::status::Issue, String)], hash: u64, owner: &str) -> Option<u64> {
    let marker = body::signature_marker(hash);
    issues
        .iter()
        .filter(|(i, _)| i.is_open() && i.author.login == owner && i.has(super::CRASH))
        .find(|(_, text)| text.lines().any(|l| l.trim() == marker))
        .map(|(i, _)| i.number)
}

// ── The reporter ────────────────────────────────────────────────────────

/// `swaypplet crash-report [--unit NAME] [--dry-run]`.
pub fn run(mut args: impl Iterator<Item = String>) {
    let mut unit = "swaypplet.service".to_string();
    while let Some(a) = args.next() {
        match a.as_str() {
            "--unit" => unit = args.next().unwrap_or(unit),
            // SAFETY: the process is single-threaded here; nothing else
            // reads or writes the environment yet.
            "--dry-run" => unsafe { std::env::set_var("SWAYPPLET_DRY_RUN", "1") },
            other => {
                eprintln!("crash-report: unknown argument {other}");
                std::process::exit(2);
            }
        }
    }
    match report(&unit) {
        Ok(what) => println!("crash-report: {what}"),
        Err(e) => {
            eprintln!("crash-report: {e}");
            std::process::exit(1);
        }
    }
}

fn report(unit: &str) -> Result<String, String> {
    let home = super::home();
    let panic = newest_panic();
    let dump = coredump(unit);
    let journal: Vec<String> = journal_tail(unit)
        .into_iter()
        .map(|l| super::redact(&l, &home))
        .collect();

    let signature = panic
        .as_ref()
        .and_then(|(_, note)| from_panic(note))
        .or_else(|| dump.as_deref().and_then(from_coredump))
        .or_else(|| from_journal(&journal))
        .ok_or("nothing says why it stopped: no panic note, no core dump, no exit status")?;
    let hash = signature.hash();

    let owner = super::owner();
    let open: Vec<super::status::Issue> = super::gh::issues(Some(super::CRASH), "open", 30)?
        .into_iter()
        .filter(|i| i.is_open() && i.author.login == owner)
        .collect();
    let with_bodies: Vec<(super::status::Issue, String)> = open
        .into_iter()
        .filter_map(|i| {
            let text = issue_body(i.number).ok()?;
            Some((i, text))
        })
        .collect();

    let when = glib::DateTime::now_local()
        .and_then(|t| t.format("%Y-%m-%d %H:%M"))
        .map(|s| s.to_string())
        .unwrap_or_default();
    let result = if let Some(n) = duplicate(&with_bodies, hash, &owner) {
        super::gh::comment(n, &body::seen_again(super::rev(), &when))?;
        format!("#{n} seen again ({})", signature.key)
    } else {
        let crash = Crash {
            signature: signature.key.clone(),
            rev: super::rev().to_string(),
            summary: signature.summary.clone(),
            panic: panic.as_ref().map(|(_, n)| super::redact(n, &home)),
            stack: dump
                .as_deref()
                .map(|d| super::redact(&crashing_stack(d), &home)),
            journal,
        };
        let url = super::gh::create_issue(
            &body::crash_title(&crash),
            &body::crash_body(&crash, hash),
            &[super::CRASH],
        )?;
        format!("filed {url} ({})", signature.key)
    };
    // Consumed: the next failure must not report this panic again.
    if let Some((path, _)) = panic
        && !super::dry_run()
    {
        let _ = std::fs::rename(&path, path.with_extension("reported"));
    }
    Ok(result)
}

fn issue_body(number: u64) -> Result<String, String> {
    let out = std::process::Command::new(super::gh::which("gh").unwrap_or_else(|| "gh".into()))
        .args([
            "issue",
            "view",
            &number.to_string(),
            "--repo",
            &super::repo(),
            "--json",
            "body",
            "--jq",
            ".body",
        ])
        .output()
        .map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).to_string());
    }
    Ok(String::from_utf8_lossy(&out.stdout).to_string())
}

/// The newest panic note written in the last ten minutes and not reported.
fn newest_panic() -> Option<(PathBuf, String)> {
    let now = SystemTime::now();
    panic_files(&panic_dir())
        .into_iter()
        .filter(|(_, t)| now.duration_since(*t).is_ok_and(|age| age < RECENT))
        .max_by_key(|(_, t)| *t)
        .and_then(|(p, _)| Some((p.clone(), std::fs::read_to_string(&p).ok()?)))
}

/// `coredumpctl info` for the newest dump of `unit`'s process in the last
/// ten minutes. By unit, not by name: the wrapped binary's comm is
/// `.swaypplet-wrap`, and a harness build's is `swaypplet`. systemd-coredump
/// writes the entry a moment after the unit fails, so this waits up to 30 s.
fn coredump(unit: &str) -> Option<String> {
    let matching = format!("COREDUMP_USER_UNIT={unit}");
    for _ in 0..15 {
        let out = std::process::Command::new("coredumpctl")
            .args(["info", "--no-pager", "--since=-10min", "-1", &matching])
            .output()
            .ok()?;
        let text = String::from_utf8_lossy(&out.stdout).to_string();
        if out.status.success() && text.contains("Stack trace of thread") {
            return Some(text);
        }
        std::thread::sleep(Duration::from_secs(2));
    }
    None
}

/// The crashing thread's block of a coredumpctl report, for the issue.
fn crashing_stack(info: &str) -> String {
    let mut out = Vec::new();
    let mut in_stack = false;
    for line in info.lines() {
        let t = line.trim();
        if t.starts_with("Stack trace of thread") {
            if in_stack {
                break;
            }
            in_stack = true;
        }
        if in_stack {
            if t.is_empty() {
                break;
            }
            out.push(t.to_string());
        }
    }
    // The signal line helps a reader who skipped the title.
    if let Some(sig) = info.lines().find(|l| l.trim().starts_with("Signal: ")) {
        out.insert(0, sig.trim().to_string());
    }
    out.join("\n")
}

fn journal_tail(unit: &str) -> Vec<String> {
    std::process::Command::new("journalctl")
        .args([
            "--user",
            "--unit",
            unit,
            "--lines",
            &super::LOG_LINES.to_string(),
            "--no-pager",
            "--output",
            "short-iso",
        ])
        .output()
        .map(|o| {
            String::from_utf8_lossy(&o.stdout)
                .lines()
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

/// The last resort: systemd's own line about how the process ended, e.g.
/// `Main process exited, code=killed, status=6/ABRT`.
pub fn from_journal(lines: &[String]) -> Option<Signature> {
    let line = lines
        .iter()
        .rev()
        .find(|l| l.contains("Main process exited"))?;
    let status = line.split("status=").nth(1)?.trim().trim_end_matches('.');
    Some(Signature {
        key: format!("exit {status}"),
        summary: format!("exited with status {status}"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::quality::status::{Author, Issue, Label};

    const DUMP: &str = "           PID: 4242 (swaypplet)
        Signal: 11 (SEGV)
     Timestamp: Sat 2026-09-27 10:00:00 CEST
  Command Line: /nix/store/abc-swaypplet/bin/swaypplet
       Message: Process 4242 (swaypplet) of user 1000 dumped core.

                Stack trace of thread 4242:
                #0  0x00007f1c2a28e9cc __pthread_kill_implementation (libc.so.6 + 0x8e9cc)
                #1  0x00007f1c2a3a0000 n/a (libgtk-4.so.1 + 0x3a0000)
                #2  0x000055d1c0e8b3a4 _ZN9swaypplet3bar5clock4tick17h0123456789abcdefE (swaypplet + 0x2a93a4)
                #3  0x000055d1c0e8b400 _ZN9swaypplet3bar3run28_$u7b$$u7b$closure$u7d$$u7d$17hfedcba9876543210E (swaypplet + 0x2a9400)
                #4  0x000055d1c0e8b500 swaypplet::app::run::h00112233aabbccdd (swaypplet + 0x2a9500)
                #5  0x000055d1c0e8b600 main (swaypplet + 0x2a9600)

                Stack trace of thread 4243:
                #0  0x00007f1c2a28e9cc poll (libc.so.6 + 0x8e9cc)
";

    #[test]
    fn a_dump_signs_with_the_signal_and_our_top_frames() {
        let sig = from_coredump(DUMP).unwrap();
        assert_eq!(
            sig.key,
            "SIGSEGV swaypplet::bar::clock::tick < swaypplet::bar::run::{{closure}} < swaypplet::app::run"
        );
        assert_eq!(sig.summary, "SIGSEGV in swaypplet::bar::clock::tick");
    }

    #[test]
    fn the_same_crash_from_another_build_signs_the_same() {
        let moved = DUMP
            .replace("0x000055d1c0e8b3a4", "0x000055aaaaaaaaaa")
            .replace("17h0123456789abcdefE", "17h9999999999999999E")
            .replace("PID: 4242", "PID: 7");
        assert_eq!(
            from_coredump(DUMP).unwrap().hash(),
            from_coredump(&moved).unwrap().hash()
        );
    }

    #[test]
    fn a_dump_with_no_function_names_signs_nothing() {
        let wild = "        Signal: 11 (SEGV) si_code: SI_KERNEL
       Message: Process 1276808 (.swaypplet-wrap) of user 1000 dumped core.

                Module /nix/store/x-swaypplet-0.1.0/bin/.swaypplet-wrapped without build-id.
                Stack trace of thread 1276808:
                #0  0x00007eed0847a214 n/a (n/a + 0x0)
                ELF object binary architecture: AMD x86-64
";
        assert_eq!(from_coredump(wild), None);
        let abrt = DUMP.replace("Signal: 11 (SEGV)", "Signal: 6 (ABRT) si_code: SI_TKILL");
        assert!(from_coredump(&abrt).unwrap().key.starts_with("SIGABRT "));
    }

    #[test]
    fn only_the_crashing_threads_frames_are_read() {
        let frames = stack_frames(DUMP);
        assert_eq!(frames.len(), 6);
        assert_eq!(frames[1].function, "?");
        assert_eq!(frames[1].module, "libgtk-4.so.1");
        assert_eq!(frames[5].function, "main");
        assert!(crashing_stack(DUMP).starts_with("Signal: 11 (SEGV)\nStack trace of thread 4242:"));
        assert!(!crashing_stack(DUMP).contains("4243"));
    }

    #[test]
    fn a_panic_signs_with_file_line_and_a_numberless_message() {
        let note = "message: index out of bounds: the len is 3 but the index is 7\nlocation: src/bar/clock.rs:88:17\nthread: main\n";
        let sig = from_panic(note).unwrap();
        assert_eq!(
            sig.key,
            "panic src/bar/clock.rs:88 index out of bounds: the len is N but the index is N"
        );
        let other = note.replace("is 7", "is 12").replace(":17", ":21");
        assert_eq!(from_panic(&other).unwrap().hash(), sig.hash());
        let elsewhere = note.replace(":88:", ":89:");
        assert_ne!(from_panic(&elsewhere).unwrap().hash(), sig.hash());
        assert!(from_panic("no fields here").is_none());
    }

    #[test]
    fn the_hooks_note_reads_back_as_its_signature() {
        let note = panic_note(
            "called `Option::unwrap()` on a `None` value",
            "src/jump/pin.rs:212:40",
            "main",
            "swaypplet",
            "   0: std::backtrace::Backtrace::force_capture\n",
        );
        let sig = from_panic(&note).unwrap();
        assert_eq!(
            sig.key,
            "panic src/jump/pin.rs:212 called `Option::unwrap()` on a `None` value"
        );
        assert!(note.contains(&format!("rev: {}", crate::quality::rev())));
    }

    #[test]
    fn systemds_exit_line_is_the_last_resort() {
        let lines = vec![
            "x swaypplet[1]: something".to_string(),
            "x systemd[1]: swaypplet.service: Main process exited, code=dumped, status=6/ABRT"
                .to_string(),
        ];
        assert_eq!(from_journal(&lines).unwrap().key, "exit 6/ABRT");
        assert!(from_journal(&lines[..1]).is_none());
    }

    #[test]
    fn demangling_drops_the_hash_and_keeps_the_path() {
        assert_eq!(
            demangle("_ZN4core9panicking5panic17h0123456789abcdefE"),
            "core::panicking::panic"
        );
        assert_eq!(
            demangle("swaypplet::x::y::h0123456789abcdef"),
            "swaypplet::x::y"
        );
        assert_eq!(demangle("g_main_loop_run"), "g_main_loop_run");
        assert_eq!(demangle("_ZN3bad99E"), "_ZN3bad99E");
    }

    fn crash_issue(n: u64, author: &str, open: bool) -> Issue {
        Issue {
            number: n,
            title: "crash: x".into(),
            state: if open { "OPEN" } else { "CLOSED" }.into(),
            labels: vec![
                Label {
                    name: "crash".into(),
                },
                Label {
                    name: "auto-fix".into(),
                },
            ],
            author: Author {
                login: author.into(),
            },
            url: String::new(),
            created_at: String::new(),
        }
    }

    #[test]
    fn a_known_signature_finds_the_owners_open_issue() {
        let hash = 0x1234;
        let marked = format!("{}\n\nbody", body::signature_marker(hash));
        let issues = vec![
            (crash_issue(3, "mallory", true), marked.clone()),
            (crash_issue(4, "meros", false), marked.clone()),
            (crash_issue(5, "meros", true), "other".to_string()),
            (crash_issue(6, "meros", true), marked.clone()),
        ];
        assert_eq!(duplicate(&issues, hash, "meros"), Some(6));
        assert_eq!(duplicate(&issues, 0x9999, "meros"), None);
        assert_eq!(duplicate(&issues[..3], hash, "meros"), None);
    }
}
