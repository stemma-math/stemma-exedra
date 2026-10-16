//! Addresses as people and agents write them, which are not always the
//! canonical spelling: a reference with its brackets (`[[t12#45-50]]`), a
//! range with a dash from typography (`t12#45–50`), or a link to the web
//! app (`https://stemma.example.org/g/geometry/t/12#45`), which also says
//! the group.

use crate::address::{Address, AddressError};

/// An address, and the group it is in when what was written says so.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Located {
    pub group: Option<String>,
    pub address: Address,
}

/// Reads an address in any of the forms people write it.
pub fn locate(input: &str) -> Result<Located, AddressError> {
    let mut s = input.trim();
    if let Some(inner) = s.strip_prefix("[[").and_then(|s| s.strip_suffix("]]")) {
        s = inner.trim();
    }
    if let Some(located) = web_link(s) {
        return located;
    }
    Ok(Located {
        group: None,
        address: normalize(s).parse()?,
    })
}

/// Typographic dashes in a range become the ASCII one.
fn normalize(s: &str) -> String {
    s.replace(['\u{2013}', '\u{2014}', '\u{2012}', '\u{2212}'], "-")
}

/// A link or path into the web app: `…/g/<group>/t/12#45`.
fn web_link(s: &str) -> Option<Result<Located, AddressError>> {
    let start = s.find("/g/")?;
    // A path, or a URL whose path starts after the host.
    let before = &s[..start];
    if !(before.is_empty() || before.contains("://")) {
        return None;
    }
    let rest = &s[start + 3..];
    let (group, rest) = rest.split_once('/')?;
    let (kind, rest) = rest.split_once('/')?;
    let (path, fragment) = match rest.split_once('#') {
        Some((path, fragment)) => (path, Some(fragment)),
        None => (rest, None),
    };
    let path = path.trim_end_matches('/');
    let canonical = match (kind, fragment) {
        ("t", Some(fragment)) => format!("t{path}#{fragment}"),
        ("t", None) => format!("t{path}"),
        ("p", Some(section)) => format!("p/{path}#{section}"),
        ("p", None) => format!("p/{path}"),
        ("lib", _) => format!("lean:{path}"),
        ("w", _) => format!("w{path}"),
        _ => return None,
    };
    Some(normalize(&canonical).parse().map(|address| Located {
        group: Some(group.to_owned()),
        address,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn located(s: &str) -> (Option<String>, String) {
        let found = locate(s).unwrap_or_else(|e| panic!("{s:?} should be located: {e}"));
        (found.group, found.address.to_string())
    }

    #[test]
    fn reads_the_canonical_and_short_forms() {
        assert_eq!(located("t12#45-50"), (None, "t12#45-50".into()));
        assert_eq!(located("  t12 "), (None, "t12".into()));
        assert_eq!(located("[[t12#45-50]]"), (None, "t12#45-50".into()));
        assert_eq!(located("[[ w7 ]]"), (None, "w7".into()));
        assert_eq!(located("t12#45–50"), (None, "t12#45-50".into()));
        assert_eq!(located("lean:Foo.bar"), (None, "lean:Foo.bar".into()));
    }

    #[test]
    fn reads_links_to_the_web_app() {
        let group = Some("geometry".to_owned());
        assert_eq!(
            located("https://stemma.example.org/g/geometry/t/12#45"),
            (group.clone(), "t12#45".into())
        );
        assert_eq!(
            located("http://127.0.0.1:8080/g/geometry/t/12"),
            (group.clone(), "t12".into())
        );
        assert_eq!(
            located("/g/geometry/t/12#45-50"),
            (group.clone(), "t12#45-50".into())
        );
        assert_eq!(
            located("/g/geometry/p/notes#gluing"),
            (group.clone(), "p/notes#gluing".into())
        );
        assert_eq!(
            located("/g/geometry/lib/Foo.bar"),
            (group.clone(), "lean:Foo.bar".into())
        );
        assert_eq!(located("/g/geometry/w/7"), (group, "w7".into()));
    }

    #[test]
    fn rejects_what_is_no_address() {
        assert!(locate("t0").is_err());
        assert!(locate("/g/geometry/t/zero").is_err());
        assert!(locate("hello").is_err());
    }
}
