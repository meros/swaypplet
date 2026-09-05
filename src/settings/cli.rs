//! `swaypplet settings`: the settings file from a keybind or a script.
//!
//! The same file the pane writes, by dotted key, so a sway binding can flip
//! a switch (`swaypplet settings set bar.osd_in_bar true`) and a dotfile can
//! seed an account without opening the panel. Every reader follows the file
//! on its own (`store.rs`), so nothing here signals anything.
//!
//! `apply` is what `exec_always` in the sway config runs: a `swaymsg reload`
//! re-runs the config's own `bg` line, and this puts the saved pick back on
//! top of it.
//!
//! Standalone process, no GTK, and routed in `main.rs` before `app::run`,
//! which would otherwise treat the word as a request to the running panel.

use serde_json::Value;

use super::store::Settings;
use super::wallpaper;

const USAGE: &str = "\
usage: swaypplet settings                      the settings in force, as JSON
       swaypplet settings get <section.field>  one value
       swaypplet settings set <section.field> <value>
       swaypplet settings reset [section]      drop the override for one section, or all
       swaypplet settings nix <section>        the section as theme/settings.nix holds it
       swaypplet settings apply                re-apply the saved wallpaper (for exec_always)

sections and fields (data/settings-defaults.json has every default):
  wallpaper  path, mode (fill|fit|stretch|center|tile)
  look       motion (full|reduced|off)
  idle       dim_after_s, dim_level, lock_after_s, blank_after_s,
             suspend_after_s (0 is never), walk_away_lock, face_unlock
  bar        clock_24h, clock_date, osd_in_bar, board, media, tray, battery, presence
  keys       volume_step, brightness_step, volume_boost
  alerts     linger (short|normal|long), corner (top_right|top_left|bottom_right|bottom_left),
             stack, quiet, quiet_from_h, quiet_to_h
  capture    folder, after (both|save|copy), annotate
A value is JSON where it parses (600, true) and a string otherwise (fit, /path/to.png).";

pub fn run(args: impl Iterator<Item = String>) {
    let args: Vec<String> = args.collect();
    let words: Vec<&str> = args.iter().map(String::as_str).collect();
    match execute(&words) {
        Ok(out) => {
            if !out.is_empty() {
                println!("{out}");
            }
        }
        Err(msg) => {
            eprintln!("swaypplet settings: {msg}");
            std::process::exit(2);
        }
    }
}

/// The verbs, with the file as the only side effect, so the parsing and the
/// wording can be tested without a compositor.
fn execute(words: &[&str]) -> Result<String, String> {
    match words {
        [] => {
            serde_json::to_string_pretty(&Settings::load().effective()).map_err(|e| e.to_string())
        }
        ["get", key] => {
            let settings = Settings::load();
            let value = settings.get(key).or_else(|| {
                // No pick means the config's wallpaper is in force; say
                // which, as the pane does, rather than "no such key".
                let field = key.strip_prefix("wallpaper.")?;
                let system = serde_json::to_value(wallpaper::system_default()?).ok()?;
                system.get(field).cloned()
            });
            value
                .map(|v| render(&v))
                .ok_or_else(|| format!("no such key `{key}`\n{USAGE}"))
        }
        ["set", key, value] => {
            let mut settings = Settings::load();
            settings.set(key, parse_value(value))?;
            settings.save();
            // The one section with a reader that does not follow the file:
            // the compositor.
            if key.starts_with("wallpaper.")
                && let Some(w) = &settings.wallpaper
            {
                wallpaper::apply_blocking(w)?;
            }
            Ok(String::new())
        }
        ["reset"] | ["reset", _] => {
            let mut settings = Settings::load();
            settings.reset(words.get(1).copied())?;
            settings.save();
            if matches!(words.get(1), None | Some(&"wallpaper")) {
                match wallpaper::system_default() {
                    Some(w) => wallpaper::apply_blocking(&w)?,
                    None => log::warn!("wallpaper: no system default to reset to"),
                }
            }
            Ok(String::new())
        }
        ["nix", section] => Settings::load().section_as_nix(section).ok_or_else(|| {
            format!(
                "`{section}` has no Nix form; one of {}",
                Settings::NIX_SECTIONS.join(", ")
            )
        }),
        ["apply"] => match Settings::load().wallpaper {
            Some(w) => wallpaper::apply_blocking(&w),
            None => Ok(()),
        }
        .map(|()| String::new()),
        ["help"] | ["--help"] | ["-h"] => Ok(USAGE.to_string()),
        _ => Err(USAGE.to_string()),
    }
}

/// `600` and `true` as JSON, anything else as the string it is.
fn parse_value(raw: &str) -> Value {
    match serde_json::from_str::<Value>(raw) {
        Ok(v @ (Value::Number(_) | Value::Bool(_))) => v,
        _ => Value::String(raw.to_string()),
    }
}

/// A scalar without JSON's quotes, so `$(swaypplet settings get wallpaper.path)`
/// is a path.
fn render(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_parse_as_json_where_they_can() {
        assert_eq!(parse_value("600"), serde_json::json!(600));
        assert_eq!(parse_value("true"), serde_json::json!(true));
        assert_eq!(parse_value("fit"), serde_json::json!("fit"));
        // A quoted string on the command line is still the string, not
        // JSON's idea of it, so a path that happens to be valid JSON stays
        // a path.
        assert_eq!(parse_value("[1]"), serde_json::json!("[1]"));
        assert_eq!(parse_value("/tmp/a.png"), serde_json::json!("/tmp/a.png"));
    }

    #[test]
    fn scalars_render_bare() {
        assert_eq!(render(&serde_json::json!("/tmp/a.png")), "/tmp/a.png");
        assert_eq!(render(&serde_json::json!(600)), "600");
        assert_eq!(render(&serde_json::json!(false)), "false");
    }

    #[test]
    fn unknown_verbs_and_keys_fail_with_the_usage() {
        assert!(execute(&["frobnicate"]).unwrap_err().contains("usage:"));
        assert!(
            execute(&["get", "idle.nope"])
                .unwrap_err()
                .contains("no such key")
        );
        assert!(execute(&["nix", "wallpaper"]).is_err());
        assert!(execute(&["help"]).unwrap().starts_with("usage:"));
    }
}
