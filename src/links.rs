//! What a link's destination is and whether pica may open it. Nothing here
//! opens anything: it only decides. Web and mail addresses go to the
//! platform's opener, local text files to a tab, and a `#fragment` to a
//! heading. Everything else is refused with a reason to show.
use std::{
    ops::Range,
    path::{Path, PathBuf},
};

/// Longer destinations are refused: none is meant for a person to follow.
const MAX_LENGTH: usize = 4096;

/// A destination, sorted by what opening it means.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    /// An `http`, `https` or `mailto` address for the platform's opener, as
    /// written: no control characters and no spaces.
    Web(String),
    /// A local path, resolved against the document's folder, and a heading
    /// to go to in it.
    File {
        path: PathBuf,
        fragment: Option<String>,
    },
    /// A heading of the document the link is in.
    Heading(String),
    /// Not opened, and why.
    Refused(String),
}

/// Decide what `destination` is. Relative paths are taken from `folder`,
/// the folder of the document holding the link.
pub fn classify(destination: &str, folder: &Path) -> Target {
    let destination = destination.trim();
    if destination.is_empty() {
        return Target::Refused("This link has no destination".into());
    }
    if destination.len() > MAX_LENGTH {
        return Target::Refused("This link is too long to open".into());
    }
    if destination.chars().any(char::is_control) {
        return Target::Refused("This link holds control characters".into());
    }
    if let Some(fragment) = destination.strip_prefix('#') {
        return match percent_decode(fragment) {
            Some(fragment) => Target::Heading(fragment),
            None => Target::Refused("This link's heading is not valid text".into()),
        };
    }
    match scheme(destination) {
        Some(scheme) => web(destination, &scheme.to_ascii_lowercase()),
        None => file(destination, folder),
    }
}

/// `scheme:` at the start of a destination, as RFC 3986 spells one. A
/// single letter is a Windows drive, which is a path.
fn scheme(destination: &str) -> Option<&str> {
    let (scheme, _) = destination.split_once(':')?;
    let mut characters = scheme.chars();
    let valid = characters.next().is_some_and(|c| c.is_ascii_alphabetic())
        && characters.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '.' | '-'));
    (valid && scheme.len() > 1).then_some(scheme)
}

/// Only addresses a browser or mail program is meant to be handed.
fn web(destination: &str, scheme: &str) -> Target {
    let rest = &destination[scheme.len() + 1..];
    if destination.chars().any(char::is_whitespace) {
        return Target::Refused("This link holds spaces, so it is not opened".into());
    }
    match scheme {
        "http" | "https" => {
            let host = rest
                .strip_prefix("//")
                .map(|rest| rest.split(['/', '?', '#']).next().unwrap_or(""));
            match host {
                Some(host) if !host.is_empty() => Target::Web(destination.to_owned()),
                _ => Target::Refused("This web link names no site".into()),
            }
        }
        "mailto" => {
            // Some mail programs attach whatever file a link names.
            let query = rest.split_once('?').map_or("", |(_, query)| query);
            let attaches = query.split('&').any(|field| {
                let name = field.split('=').next().unwrap_or("").to_ascii_lowercase();
                matches!(name.as_str(), "attach" | "attachment")
            });
            if attaches {
                Target::Refused("Mail links that attach files are not opened".into())
            } else if rest.split('?').next().unwrap_or("").is_empty() && query.is_empty() {
                Target::Refused("This mail link names no address".into())
            } else {
                Target::Web(destination.to_owned())
            }
        }
        _ => Target::Refused(format!(
            "{scheme}: links are not opened; only http, https, mailto and files"
        )),
    }
}

/// A path beside the document, with any `?query` dropped and `%20` and its
/// like decoded, as a browser would read a relative link.
fn file(destination: &str, folder: &Path) -> Target {
    let (path, fragment) = match destination.split_once('#') {
        Some((path, fragment)) => (path, Some(fragment)),
        None => (destination, None),
    };
    let path = path.split('?').next().unwrap_or("");
    let (Some(path), Some(fragment)) = (
        percent_decode(path),
        fragment.map_or(Some(None), |fragment| percent_decode(fragment).map(Some)),
    ) else {
        return Target::Refused("This link is not valid text once decoded".into());
    };
    if path.is_empty() || path.chars().any(char::is_control) {
        return Target::Refused("This link names no file".into());
    }
    Target::File {
        path: folder.join(path),
        fragment: fragment.filter(|fragment| !fragment.is_empty()),
    }
}

/// `%41` to `A`. A `%` that is not followed by two hex digits stays as it
/// is; bytes that do not make UTF-8 give `None`.
fn percent_decode(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        let hex = |at: usize| bytes.get(at).and_then(|byte| (*byte as char).to_digit(16));
        match (bytes[index], hex(index + 1), hex(index + 2)) {
            (b'%', Some(high), Some(low)) => {
                decoded.push((high * 16 + low) as u8);
                index += 3;
            }
            (byte, _, _) => {
                decoded.push(byte);
                index += 1;
            }
        }
    }
    String::from_utf8(decoded).ok()
}

/// A web or mail address written out in plain text around `offset`, in any
/// kind of file: its range in `text`. It runs to the next space or bracket
/// and leaves out the punctuation a sentence puts after it.
pub fn bare_url(text: &str, offset: usize) -> Option<Range<usize>> {
    // No address worth opening is longer than `MAX_LENGTH`, so that much
    // text on either side is all there is to read: the status line asks
    // this on every frame, and a line can be megabytes long.
    let boundary = |mut at: usize| {
        while !text.is_char_boundary(at) {
            at -= 1;
        }
        at
    };
    let offset = boundary(offset.min(text.len()));
    let window_start = boundary(offset.saturating_sub(MAX_LENGTH));
    let window_end = boundary((offset + MAX_LENGTH).min(text.len()));
    let line_start = text[window_start..offset]
        .rfind(['\n', '\r'])
        .map_or(window_start, |at| window_start + at + 1);
    let line_end = text[offset..window_end]
        .find(['\n', '\r'])
        .map_or(window_end, |at| offset + at);
    let line = &text[line_start..line_end];
    let mut from = 0;
    while from < line.len() {
        // The earliest address at or after `from`.
        let Some(start) = ["https://", "http://", "mailto:"]
            .iter()
            .filter_map(|scheme| find_ignoring_case(&line[from..], scheme))
            .min()
            .map(|at| from + at)
        else {
            break;
        };
        let rest = &line[start..];
        // The address ends at a space, a quote, or a bracket it did not
        // open: `(see https://example.com/a_(b))` ends after `(b)`.
        let mut end = rest.len();
        let (mut round, mut square) = (0usize, 0usize);
        for (at, c) in rest.char_indices() {
            let closes = |depth: &mut usize| match depth.checked_sub(1) {
                Some(less) => {
                    *depth = less;
                    false
                }
                None => true,
            };
            let stops = match c {
                '(' => {
                    round += 1;
                    false
                }
                '[' => {
                    square += 1;
                    false
                }
                ')' => closes(&mut round),
                ']' => closes(&mut square),
                c => c.is_whitespace() || c.is_control() || "<>\"'`|\\^{}".contains(c),
            };
            if stops {
                end = at;
                break;
            }
        }
        // A sentence's own punctuation is not part of it either.
        while let Some(last) = rest[..end].chars().next_back()
            && ".,;:!?*_~".contains(last)
        {
            end -= last.len_utf8();
        }
        let range = line_start + start..line_start + start + end;
        let scheme_end = rest.find(':').map_or(0, |at| at + 1);
        let named = rest[scheme_end..end].trim_start_matches('/');
        if !named.is_empty() && range.start <= offset && offset <= range.end {
            return Some(range);
        }
        from = start + end.max(1);
    }
    None
}

/// Where `needle`, which is ASCII lowercase, begins in `haystack` whatever
/// case it is written in there.
fn find_ignoring_case(haystack: &str, needle: &str) -> Option<usize> {
    haystack
        .as_bytes()
        .windows(needle.len())
        .position(|window| window.eq_ignore_ascii_case(needle.as_bytes()))
}

/// A heading's anchor as GitHub and most renderers derive it: lowercase,
/// punctuation dropped, spaces as hyphens.
pub fn slug(title: &str) -> String {
    title
        .trim()
        .chars()
        .filter_map(|c| match c {
            ' ' => Some('-'),
            '-' | '_' => Some(c),
            c if c.is_alphanumeric() => Some(c),
            _ => None,
        })
        .flat_map(char::to_lowercase)
        .collect()
}

/// The heading `fragment` names among `titles`, in order. A repeated title
/// takes `-1`, `-2` and so on, as those renderers number them.
pub fn heading(titles: &[&str], fragment: &str) -> Option<usize> {
    let wanted = fragment.trim().to_lowercase();
    let mut seen: Vec<(String, usize)> = Vec::new();
    for (index, title) in titles.iter().enumerate() {
        let base = slug(title);
        let count = match seen.iter_mut().find(|(known, _)| *known == base) {
            Some((_, count)) => {
                *count += 1;
                *count
            }
            None => {
                seen.push((base.clone(), 0));
                0
            }
        };
        let anchor = if count == 0 {
            base
        } else {
            format!("{base}-{count}")
        };
        if anchor == wanted {
            return Some(index);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn refused(destination: &str) -> bool {
        matches!(
            classify(destination, Path::new("/notes")),
            Target::Refused(_)
        )
    }

    #[test]
    fn only_web_and_mail_addresses_go_to_the_opener() {
        let folder = Path::new("/notes");
        for address in [
            "https://example.com",
            "http://example.com/a/b?c=d&e=f#g",
            "HTTPS://Example.COM/Path",
            "https://example.com/a%20b",
            "https://user@example.com:8443/x;y=1",
            "https://[::1]:8080/",
            "mailto:someone@example.com",
            "mailto:someone@example.com?subject=Hello%20there&body=Hi",
            "MailTo:a@b.example",
        ] {
            assert_eq!(
                classify(address, folder),
                Target::Web(address.into()),
                "{address}"
            );
        }
        // Surrounding space is not part of a destination.
        assert_eq!(
            classify("  https://example.com \n", folder),
            Target::Web("https://example.com".into())
        );
        for address in [
            "",
            "   ",
            "javascript:alert(1)",
            "data:text/html,<script>alert(1)</script>",
            "file:///etc/passwd",
            "file://server/share/x",
            "ftp://example.com/file",
            "ssh://host/path",
            "vbscript:msgbox(1)",
            "tel:+15551234567",
            "x-apple.systempreferences:com.apple.preference",
            "smb://server/share",
            "about:blank",
            "chrome://settings",
            "vscode://file/etc/passwd",
            "https:",
            "https://",
            "https:///path-without-a-site",
            "http:example.com",
            "https://exa mple.com/",
            "https://example.com/a b",
            "https://example.com/\u{1b}[31m",
            "https://example.com/\u{0}",
            "mailto:",
            "mailto:a@b.example?attach=/etc/passwd",
            "mailto:a@b.example?subject=x&Attachment=~/.ssh/id_rsa",
            "mailto:?ATTACH=/etc/passwd",
            "-rf",
        ] {
            assert!(
                refused(address) || !matches!(classify(address, folder), Target::Web(_)),
                "{address:?} must not reach the opener: {:?}",
                classify(address, folder)
            );
        }
        // Nothing the opener is handed can be read as an option or split
        // into words, whatever the destination.
        for address in [
            "https://example.com/$(reboot);`id`&x|y",
            "mailto:a@b.example",
        ] {
            let Target::Web(url) = classify(address, folder) else {
                panic!("{address}");
            };
            assert!(!url.starts_with('-') && !url.contains(char::is_whitespace));
            assert_eq!(url, address, "passed as written, as one argument");
        }
        assert!(refused(&format!(
            "https://example.com/{}",
            "a".repeat(MAX_LENGTH)
        )));
    }

    #[test]
    fn paths_resolve_beside_the_document_and_decode_like_a_browser() {
        let folder = Path::new("/notes/project");
        let file = |path: &str, fragment: Option<&str>| Target::File {
            path: PathBuf::from(path),
            fragment: fragment.map(str::to_owned),
        };
        for (destination, expected) in [
            ("docs/keys.md", file("/notes/project/docs/keys.md", None)),
            ("./todo.md", file("/notes/project/./todo.md", None)),
            ("../other.md", file("/notes/project/../other.md", None)),
            ("/etc/hosts", file("/etc/hosts", None)),
            (
                "docs/keys.md#moving-and-selecting",
                file("/notes/project/docs/keys.md", Some("moving-and-selecting")),
            ),
            ("my%20notes.md", file("/notes/project/my notes.md", None)),
            (
                "caf%C3%A9.md#%C3%A9t%C3%A9",
                file("/notes/project/café.md", Some("été")),
            ),
            (
                "page.md?raw=1#top",
                file("/notes/project/page.md", Some("top")),
            ),
            ("100%.md", file("/notes/project/100%.md", None)),
            ("notes.md#", file("/notes/project/notes.md", None)),
            // A drive letter is a path, never a scheme.
            (
                "C:/Users/me/notes.md",
                file("/notes/project/C:/Users/me/notes.md", None),
            ),
        ] {
            assert_eq!(classify(destination, folder), expected, "{destination}");
        }
        assert_eq!(
            classify("#getting-started", folder),
            Target::Heading("getting-started".into())
        );
        assert_eq!(
            classify("#%C3%A9t%C3%A9", folder),
            Target::Heading("été".into())
        );
        for destination in ["bad%FFname.md", "#%FF", "a%00b.md", "?query-only"] {
            assert!(refused(destination), "{destination}");
        }
    }

    #[test]
    fn bare_addresses_are_found_around_the_caret_without_their_punctuation() {
        let text = "See https://example.com/a_(b), then (http://x.example/y).\nMail mailto:me@x.example! Not a link: https:// or www.example.com\n<https://in.brackets/>  \"https://quoted.example/q\" HTTPS://UPPER.example";
        let found = |needle: &str, inside: usize| {
            let at = text.find(needle).unwrap() + inside;
            bare_url(text, at).map(|range| &text[range])
        };
        assert_eq!(
            found("https://example", 0),
            Some("https://example.com/a_(b)")
        );
        assert_eq!(
            found("https://example", 12),
            Some("https://example.com/a_(b)")
        );
        // The end of an address still touches it.
        assert_eq!(found("), then", 1), Some("https://example.com/a_(b)"));
        assert_eq!(found("http://x", 3), Some("http://x.example/y"));
        assert_eq!(found("mailto:", 9), Some("mailto:me@x.example"));
        assert_eq!(found("https://in", 4), Some("https://in.brackets/"));
        assert_eq!(
            found("https://quoted", 20),
            Some("https://quoted.example/q")
        );
        assert_eq!(found("HTTPS://UPPER", 2), Some("HTTPS://UPPER.example"));
        for (needle, inside) in [
            ("See", 1),
            ("then", 2),
            ("Not a link", 3),
            ("https:// or", 3),
            ("www.example", 4),
            ("Mail ", 2),
        ] {
            assert_eq!(found(needle, inside), None, "{needle}");
        }
        // Offsets on either side of a line ending, and past the end.
        let newline = text.find('\n').unwrap();
        assert_eq!(bare_url(text, newline), None);
        assert_eq!(
            bare_url(text, text.len() + 10).map(|r| &text[r]),
            Some("HTTPS://UPPER.example")
        );
        assert_eq!(bare_url("", 0), None);
        // A bracket the address did not open ends it, wherever it falls;
        // an address inside another's path does not.
        let nested = "(see https://example.com/a_(b))x [https://x.example/y]z https://web.example/save/https://kept.example/page";
        for (needle, address) in [
            ("example.com", "https://example.com/a_(b)"),
            ("x.example", "https://x.example/y"),
            (
                "kept.example",
                "https://web.example/save/https://kept.example/page",
            ),
        ] {
            let at = nested.find(needle).unwrap();
            assert_eq!(
                bare_url(nested, at).map(|range| &nested[range]),
                Some(address)
            );
        }
        // Only the text near the caret is read, however long the line: an
        // address there is found whole, and the far ends are not scanned.
        let filler = "界x ".repeat(300_000);
        let long = format!("{filler}see https://near.example/page here {filler}");
        let at = long.find("near.example").unwrap();
        assert_eq!(
            bare_url(&long, at).map(|range| &long[range]),
            Some("https://near.example/page")
        );
        for offset in [0, 1, 2, 3, at - 5_000, at + 5_000, long.len()] {
            assert_eq!(bare_url(&long, offset), None, "offset {offset}");
        }
        let started = std::time::Instant::now();
        for _ in 0..200 {
            std::hint::black_box(bare_url(&long, at));
        }
        assert!(
            started.elapsed() < std::time::Duration::from_secs(2),
            "a megabyte line must not be scanned on every frame"
        );
        let wide = "界 https://例え.example/パス 界";
        assert_eq!(
            bare_url(wide, 9).map(|range| &wide[range]),
            Some("https://例え.example/パス")
        );
    }

    #[test]
    fn headings_are_found_by_the_anchor_renderers_give_them() {
        assert_eq!(slug("Moving and selecting"), "moving-and-selecting");
        assert_eq!(slug("  What's new in 0.4.0?  "), "whats-new-in-040");
        assert_eq!(slug("C++ & Rust: a_b-c"), "c--rust-a_b-c");
        assert_eq!(slug("Été à Zürich"), "été-à-zürich");
        let titles = ["Intro", "Usage", "Intro", "FAQ", "Intro"];
        assert_eq!(heading(&titles, "intro"), Some(0));
        assert_eq!(heading(&titles, "Intro-1"), Some(2));
        assert_eq!(heading(&titles, "intro-2"), Some(4));
        assert_eq!(heading(&titles, "faq"), Some(3));
        assert_eq!(heading(&titles, "missing"), None);
        assert_eq!(heading(&[], "intro"), None);
    }
}
