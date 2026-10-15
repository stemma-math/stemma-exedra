//! Working papers: Markdown whose headings are its sections.
//!
//! A section is an ATX heading (`## Gluing the charts`) and what follows it,
//! up to the next heading of the same level or above. Its address is the
//! paper's with the heading's slug: `p/compactification#gluing-the-charts`.
//! The web app gives headings the same slugs, tested against the same
//! vectors.

use std::ops::Range;

use unicode_normalization::UnicodeNormalization;
use unicode_normalization::char::is_combining_mark;

use crate::reference::{fence_marker, is_closing};

/// A section of a paper.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Section {
    /// Its slug, unique in the paper: the `gluing` of `p/notes#gluing`.
    pub slug: String,
    /// The heading's text, as written.
    pub title: String,
    /// The heading's level, from 1 to 6.
    pub level: u8,
    /// Where it is in the text: its heading, and everything up to the next
    /// heading of the same level or above.
    pub span: Range<usize>,
}

/// The slug of a heading: its letters and digits in lowercase, without
/// accents, with a dash for whatever else separates them. A heading with
/// none is `section`.
pub fn slugify(title: &str) -> String {
    let mut slug = String::new();
    let mut pending_dash = false;
    for c in title.nfd() {
        if is_combining_mark(c) {
            continue;
        }
        if c.is_ascii_alphanumeric() {
            if pending_dash && !slug.is_empty() {
                slug.push('-');
            }
            pending_dash = false;
            slug.push(c.to_ascii_lowercase());
        } else {
            pending_dash = true;
        }
    }
    if slug.is_empty() {
        slug.push_str("section");
    }
    slug
}

/// An ATX heading: its level and text, if `line` is one.
fn heading(line: &str) -> Option<(u8, &str)> {
    let line = line.trim_end_matches(['\n', '\r']);
    let indent = line.len() - line.trim_start_matches(' ').len();
    if indent > 3 {
        return None;
    }
    let rest = &line[indent..];
    let level = rest.bytes().take_while(|&b| b == b'#').count();
    if !(1..=6).contains(&level) {
        return None;
    }
    let text = &rest[level..];
    if !(text.is_empty() || text.starts_with([' ', '\t'])) {
        return None;
    }
    // A closing run of `#`, after a space, is not part of the text.
    let mut text = text.trim();
    let without = text.trim_end_matches('#');
    if without.is_empty() || without.ends_with([' ', '\t']) {
        text = without.trim_end();
    }
    Some((level as u8, text))
}

/// The sections of a paper, in order, with unique slugs: a second
/// `Proof` is `proof-2`.
pub fn sections(body: &str) -> Vec<Section> {
    let mut found: Vec<Section> = Vec::new();
    let mut fence: Option<(u8, usize)> = None;
    let mut offset = 0;
    for line in body.split_inclusive('\n') {
        let start = offset;
        offset += line.len();
        if let Some(marker) = fence_marker(line) {
            match fence {
                None => fence = Some(marker),
                Some((ch, len)) if marker.0 == ch && marker.1 >= len && is_closing(line) => {
                    fence = None
                }
                Some(_) => {}
            }
            continue;
        }
        if fence.is_some() {
            continue;
        }
        let Some((level, title)) = heading(line) else {
            continue;
        };
        let base = slugify(title);
        let mut slug = base.clone();
        let mut n = 1;
        while found.iter().any(|s| s.slug == slug) {
            n += 1;
            slug = format!("{base}-{n}");
        }
        found.push(Section {
            slug,
            title: title.to_owned(),
            level,
            span: start..body.len(),
        });
    }
    // Each section ends where the next one of its level or above begins.
    for i in 0..found.len() {
        let level = found[i].level;
        if let Some(next) = found[i + 1..].iter().find(|s| s.level <= level) {
            found[i].span.end = next.span.start;
        }
    }
    found
}

/// The text of the section `slug`, heading included.
pub fn section<'a>(body: &'a str, slug: &str) -> Option<&'a str> {
    sections(body)
        .into_iter()
        .find(|s| s.slug == slug)
        .map(|s| &body[s.span])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugs_of_headings() {
        assert_eq!(slugify("Gluing the charts"), "gluing-the-charts");
        assert_eq!(
            slugify("  The map $f$ is *proper*!  "),
            "the-map-f-is-proper"
        );
        assert_eq!(slugify("Théorème de Gauß"), "theoreme-de-gau");
        assert_eq!(slugify("$\\alpha$-maps, 2nd try"), "alpha-maps-2nd-try");
        assert_eq!(slugify("∀ ∃"), "section");
    }

    #[test]
    fn finds_sections_and_where_they_end() {
        let body = "Intro.\n\n# Setting\n\nText.\n\n## Charts ##\n\nMore.\n\n```\n# not a heading\n```\n\n## Charts\n\n#hashtag\n\n# Open questions\n";
        let found = sections(body);
        let slugs: Vec<&str> = found.iter().map(|s| s.slug.as_str()).collect();
        assert_eq!(slugs, ["setting", "charts", "charts-2", "open-questions"]);
        assert_eq!(found[1].title, "Charts");
        assert_eq!(found[1].level, 2);
        assert_eq!(
            &body[found[0].span.clone()],
            "# Setting\n\nText.\n\n## Charts ##\n\nMore.\n\n```\n# not a heading\n```\n\n## Charts\n\n#hashtag\n\n"
        );
        assert_eq!(
            section(body, "charts"),
            Some("## Charts ##\n\nMore.\n\n```\n# not a heading\n```\n\n")
        );
        assert_eq!(section(body, "open-questions"), Some("# Open questions\n"));
        assert_eq!(section(body, "nowhere"), None);
    }

    #[test]
    fn headings() {
        assert_eq!(heading("### A # b ###\n"), Some((3, "A # b")));
        assert_eq!(heading("#\n"), Some((1, "")));
        assert_eq!(heading("   ## x"), Some((2, "x")));
        assert_eq!(heading("    ## x"), None);
        assert_eq!(heading("####### x"), None);
        assert_eq!(heading("#x"), None);
        assert_eq!(heading("## C#"), Some((2, "C#")));
    }
}
