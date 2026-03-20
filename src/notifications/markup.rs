/// Sanitize notification body markup for safe use with Pango.
///
/// The Desktop Notifications spec allows a subset of HTML:
///   `<b>`, `<i>`, `<u>`, `<a href="...">`, `<img>`, `<br>`
///
/// Pango markup is XML-based and supports the same tags (minus `<img>`).
/// We strip unsupported tags and escape raw `&`/`<` that aren't part of
/// recognized markup, so `set_markup()` won't choke on malformed input.

/// Allowed paired tags (lowercased).
const ALLOWED_PAIRED: &[&str] = &["b", "i", "u", "a"];

/// Sanitize a notification body string for Pango markup.
///
/// - Keeps `<b>`, `<i>`, `<u>`, `<a href="…">`, `<br/>` (converted to `\n`)
/// - Strips all other tags (content is kept, tag is removed)
/// - Escapes stray `&` and `<` that don't form valid entities/tags
/// - Converts `<br>` / `<br/>` to newline
pub fn sanitize(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let bytes = input.as_bytes();
    let len = bytes.len();
    let mut i = 0;

    while i < len {
        if bytes[i] == b'&' {
            // Try to match entity: &name; or &#digits; or &#xhex;
            if let Some(end) = find_entity_end(input, i + 1) {
                let entity = &input[i..=end]; // includes & and ;
                out.push_str(entity);
                i = end + 1;
            } else {
                out.push_str("&amp;");
                i += 1;
            }
        } else if bytes[i] == b'<' {
            // Try to match a tag: <...>
            if let Some(end) = find_tag_end(bytes, i + 1) {
                let tag_content = &input[i + 1..end]; // between < and >
                handle_tag(tag_content, &mut out);
                i = end + 1;
            } else {
                out.push_str("&lt;");
                i += 1;
            }
        } else {
            // Regular character — just copy
            let ch = input[i..].chars().next().unwrap();
            out.push(ch);
            i += ch.len_utf8();
        }
    }

    out
}

/// Find the `;` that closes an entity starting after `&` at position `start`.
/// Returns `Some(index_of_semicolon)` if valid, `None` otherwise.
fn find_entity_end(input: &str, start: usize) -> Option<usize> {
    let bytes = input.as_bytes();
    let mut j = start;

    // Max entity length: &#x10FFFF; = 10 chars after &
    let limit = (start + 10).min(bytes.len());

    while j < limit {
        if bytes[j] == b';' {
            let name = &input[start..j];
            if is_valid_entity(name) {
                return Some(j);
            }
            return None;
        }
        if !bytes[j].is_ascii_alphanumeric() && bytes[j] != b'#' && bytes[j] != b'x' {
            return None;
        }
        j += 1;
    }
    None
}

fn is_valid_entity(name: &str) -> bool {
    matches!(name, "amp" | "lt" | "gt" | "quot" | "apos")
        || (name.starts_with('#') && name.len() > 1)
}

/// Find the `>` that closes a tag starting after `<` at position `start`.
fn find_tag_end(bytes: &[u8], start: usize) -> Option<usize> {
    let limit = (start + 500).min(bytes.len());
    for j in start..limit {
        if bytes[j] == b'>' {
            return Some(j);
        }
        // Nested `<` means this isn't a well-formed tag
        if bytes[j] == b'<' {
            return None;
        }
    }
    None
}

/// Process a parsed tag (content between < and >) and emit safe markup.
fn handle_tag(tag: &str, out: &mut String) {
    let tag_trimmed = tag.trim();
    if tag_trimmed.is_empty() {
        return;
    }

    let is_closing = tag_trimmed.starts_with('/');
    let is_self_closing = tag_trimmed.ends_with('/');

    let content = tag_trimmed
        .trim_start_matches('/')
        .trim_end_matches('/')
        .trim();

    // Extract tag name (first word)
    let tag_name = content
        .split_whitespace()
        .next()
        .unwrap_or("")
        .to_lowercase();

    // Handle <br> / <br/>
    if tag_name == "br" {
        out.push('\n');
        return;
    }

    // Handle <img> — strip (Pango can't render images)
    if tag_name == "img" {
        return;
    }

    if is_closing {
        if ALLOWED_PAIRED.contains(&tag_name.as_str()) {
            out.push('<');
            out.push('/');
            out.push_str(&tag_name);
            out.push('>');
        }
    } else if ALLOWED_PAIRED.contains(&tag_name.as_str()) {
        out.push('<');
        if tag_name == "a" {
            out.push('a');
            if let Some(href) = extract_href(content) {
                out.push_str(&format!(" href=\"{}\"", escape_attr(&href)));
            }
        } else {
            out.push_str(&tag_name);
        }
        if is_self_closing {
            out.push('/');
        }
        out.push('>');
    }
    // Unknown tags: silently dropped (content between open/close still shown)
}

/// Extract href value from an `a` tag's attribute string.
fn extract_href(tag_content: &str) -> Option<String> {
    let lower = tag_content.to_lowercase();
    let pos = lower.find("href")?;
    let rest = &tag_content[pos + 4..];
    let rest = rest.trim_start();
    let rest = rest.strip_prefix('=')?;
    let rest = rest.trim_start();

    let (quote, rest) = if rest.starts_with('"') {
        ('"', &rest[1..])
    } else if rest.starts_with('\'') {
        ('\'', &rest[1..])
    } else {
        return None;
    };

    let end = rest.find(quote)?;
    Some(rest[..end].to_string())
}

/// Escape a string for use in an XML attribute value.
fn escape_attr(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_text_unchanged() {
        assert_eq!(sanitize("Hello world"), "Hello world");
    }

    #[test]
    fn allowed_tags_kept() {
        assert_eq!(sanitize("<b>bold</b>"), "<b>bold</b>");
        assert_eq!(sanitize("<i>italic</i>"), "<i>italic</i>");
        assert_eq!(sanitize("<u>under</u>"), "<u>under</u>");
    }

    #[test]
    fn unknown_tags_stripped() {
        assert_eq!(sanitize("<div>text</div>"), "text");
        assert_eq!(sanitize("<script>bad</script>"), "bad");
    }

    #[test]
    fn br_becomes_newline() {
        assert_eq!(sanitize("line1<br>line2"), "line1\nline2");
        assert_eq!(sanitize("line1<br/>line2"), "line1\nline2");
    }

    #[test]
    fn stray_ampersand_escaped() {
        assert_eq!(sanitize("a & b"), "a &amp; b");
    }

    #[test]
    fn valid_entities_preserved() {
        assert_eq!(sanitize("&amp; &lt;"), "&amp; &lt;");
    }

    #[test]
    fn anchor_href_kept() {
        let input = r#"<a href="https://example.com">link</a>"#;
        let expected = r#"<a href="https://example.com">link</a>"#;
        assert_eq!(sanitize(input), expected);
    }

    #[test]
    fn img_stripped() {
        assert_eq!(sanitize(r#"text<img src="x"/>more"#), "textmore");
    }

    #[test]
    fn unicode_preserved() {
        assert_eq!(sanitize("Hej på dig!"), "Hej på dig!");
    }

    #[test]
    fn nested_markup() {
        assert_eq!(
            sanitize("<b>bold <i>and italic</i></b>"),
            "<b>bold <i>and italic</i></b>"
        );
    }

    #[test]
    fn mixed_valid_invalid() {
        assert_eq!(
            sanitize("<b>ok</b><span>nope</span>"),
            "<b>ok</b>nope"
        );
    }
}
