//! Secrets in a text: what an agent's answer must not carry into a thread
//! unseen. Stemma's own tokens, the keys and tokens of the agents'
//! providers and of GitHub, OAuth access tokens (JWTs), and private keys.
//!
//! An answer with one is held for its requester to decide; an error, which
//! may end with a runner's output, has them hidden.

use serde::{Deserialize, Serialize};

/// A secret found in a text: where, in bytes, and what it seems to be.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Secret {
    pub start: usize,
    pub end: usize,
    pub kind: &'static str,
}

/// What a secret is replaced with when hidden.
pub const HIDDEN: &str = "[hidden]";

/// A prefix that starts a secret, what it is, and how many characters must
/// follow it, of those `allowed` takes.
struct Prefixed {
    prefix: &'static str,
    kind: &'static str,
    min: usize,
    allowed: fn(u8) -> bool,
}

fn url_safe(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_' || c == b'-'
}

fn word(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_'
}

fn alphanumeric(c: u8) -> bool {
    c.is_ascii_alphanumeric()
}

/// Longer prefixes first, so that `sk-ant-` is not taken for `sk-`.
const PREFIXED: &[Prefixed] = &[
    Prefixed {
        prefix: "github_pat_",
        kind: "a GitHub token",
        min: 22,
        allowed: word,
    },
    Prefixed {
        prefix: "sk-proj-",
        kind: "an OpenAI key",
        min: 20,
        allowed: url_safe,
    },
    Prefixed {
        prefix: "sk-ant-",
        kind: "an Anthropic key or token",
        min: 20,
        allowed: url_safe,
    },
    Prefixed {
        prefix: "sk-",
        kind: "an OpenAI key",
        min: 20,
        allowed: url_safe,
    },
    Prefixed {
        prefix: "stm_",
        kind: "a Stemma token",
        min: 20,
        allowed: url_safe,
    },
    Prefixed {
        prefix: "stj_",
        kind: "a Stemma job's token",
        min: 20,
        allowed: url_safe,
    },
    Prefixed {
        prefix: "ghp_",
        kind: "a GitHub token",
        min: 30,
        allowed: alphanumeric,
    },
    Prefixed {
        prefix: "gho_",
        kind: "a GitHub token",
        min: 30,
        allowed: alphanumeric,
    },
    Prefixed {
        prefix: "ghu_",
        kind: "a GitHub token",
        min: 30,
        allowed: alphanumeric,
    },
    Prefixed {
        prefix: "ghs_",
        kind: "a GitHub token",
        min: 30,
        allowed: alphanumeric,
    },
    Prefixed {
        prefix: "ghr_",
        kind: "a GitHub token",
        min: 30,
        allowed: alphanumeric,
    },
];

/// How many characters at `from` satisfy `allowed`.
fn run(bytes: &[u8], from: usize, allowed: fn(u8) -> bool) -> usize {
    bytes[from..].iter().take_while(|&&c| allowed(c)).count()
}

/// Whether a secret may start at `at`: not inside a longer word.
fn starts_word(bytes: &[u8], at: usize) -> bool {
    at == 0 || !url_safe(bytes[at - 1])
}

/// A JSON Web Token at `at`: three base64url parts, the first `eyJ…`.
fn jwt(bytes: &[u8], at: usize) -> Option<usize> {
    if !bytes[at..].starts_with(b"eyJ") {
        return None;
    }
    let mut end = at;
    for part in 0..3 {
        if part > 0 {
            if bytes.get(end) != Some(&b'.') {
                return None;
            }
            end += 1;
        }
        let length = run(bytes, end, |c| {
            c.is_ascii_alphanumeric() || c == b'_' || c == b'-'
        });
        if length < 10 {
            return None;
        }
        end += length;
    }
    Some(end)
}

/// A private key block at `at`, to its end line or the text's end.
fn private_key(text: &str, at: usize) -> Option<usize> {
    let rest = &text[at..];
    let line = rest.lines().next()?;
    if !(line.starts_with("-----BEGIN ") && line.trim_end().ends_with("PRIVATE KEY-----")) {
        return None;
    }
    let after = at + line.len();
    // To the end of its last line, or of the text if it has none.
    Some(match text[after..].find("-----END ") {
        Some(found) => {
            let last = after + found;
            text[last..].find('\n').map_or(text.len(), |i| last + i)
        }
        None => text.len(),
    })
}

/// The secrets in `text`, in order, none overlapping.
pub fn find(text: &str) -> Vec<Secret> {
    let bytes = text.as_bytes();
    let mut found = Vec::new();
    let mut at = 0;
    while at < bytes.len() {
        if !text.is_char_boundary(at) || !starts_word(bytes, at) {
            at += 1;
            continue;
        }
        if let Some(end) = private_key(text, at) {
            found.push(Secret {
                start: at,
                end,
                kind: "a private key",
            });
            at = end;
            continue;
        }
        if let Some(end) = jwt(bytes, at) {
            found.push(Secret {
                start: at,
                end,
                kind: "an access token",
            });
            at = end;
            continue;
        }
        let prefixed = PREFIXED.iter().find_map(|p| {
            if !bytes[at..].starts_with(p.prefix.as_bytes()) {
                return None;
            }
            let length = run(bytes, at + p.prefix.len(), p.allowed);
            (length >= p.min).then_some((p, at + p.prefix.len() + length))
        });
        if let Some((p, end)) = prefixed {
            found.push(Secret {
                start: at,
                end,
                kind: p.kind,
            });
            at = end;
            continue;
        }
        at += 1;
    }
    found
}

/// `text`, with every secret in it replaced by [`HIDDEN`].
pub fn hide(text: &str) -> String {
    let mut hidden = String::with_capacity(text.len());
    let mut from = 0;
    for secret in find(text) {
        hidden.push_str(&text[from..secret.start]);
        hidden.push_str(HIDDEN);
        from = secret.end;
    }
    hidden.push_str(&text[from..]);
    hidden
}

/// A text cut into its plain parts and its secrets, as people are shown
/// it: each secret marked, with what it seems to be.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Part {
    pub text: String,
    /// What it seems to be, when it is a secret.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secret: Option<String>,
}

/// `text`, cut into [`Part`]s.
pub fn parts(text: &str) -> Vec<Part> {
    let mut parts = Vec::new();
    let mut from = 0;
    for secret in find(text) {
        if secret.start > from {
            parts.push(Part {
                text: text[from..secret.start].to_owned(),
                secret: None,
            });
        }
        parts.push(Part {
            text: text[secret.start..secret.end].to_owned(),
            secret: Some(secret.kind.to_owned()),
        });
        from = secret.end;
    }
    if from < text.len() {
        parts.push(Part {
            text: text[from..].to_owned(),
            secret: None,
        });
    }
    parts
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(text: &str) -> Vec<&'static str> {
        find(text).into_iter().map(|s| s.kind).collect()
    }

    const B64: &str = "AbCdEfGhIjKlMnOpQrStUvWxYz0123456789_-Ab";

    #[test]
    fn finds_stemmas_tokens() {
        assert_eq!(kinds(&format!("token stm_{B64} here")), ["a Stemma token"]);
        assert_eq!(kinds(&format!("stj_{B64}")), ["a Stemma job's token"]);
        // Near misses: the prefix alone, a short tail, inside a word.
        assert!(kinds("stm_ and stm_short").is_empty());
        assert!(kinds(&format!("xstm_{B64}")).is_empty());
    }

    #[test]
    fn finds_providers_keys() {
        assert_eq!(
            kinds("sk-ant-api03-AbCdEfGhIjKlMnOpQrStUv"),
            ["an Anthropic key or token"]
        );
        assert_eq!(
            kinds("CLAUDE_CODE_OAUTH_TOKEN=sk-ant-oat01-AbCdEfGhIjKlMnOpQrStUv"),
            ["an Anthropic key or token"]
        );
        assert_eq!(kinds("sk-proj-AbCdEfGhIjKlMnOpQrStUv"), ["an OpenAI key"]);
        assert_eq!(kinds("sk-AbCdEfGhIjKlMnOpQrStUvWx"), ["an OpenAI key"]);
        // Near misses: risk-free, sk-learn, a short key.
        assert!(kinds("a risk-free sk-learn model, sk-abc").is_empty());
        assert!(kinds("task-AbCdEfGhIjKlMnOpQrStUvWx").is_empty());
    }

    #[test]
    fn finds_githubs_tokens() {
        assert_eq!(
            kinds("ghp_AbCdEfGhIjKlMnOpQrStUvWxYz0123456789"),
            ["a GitHub token"]
        );
        assert_eq!(
            kinds("github_pat_11AbCdEfGh_IjKlMnOpQrStUvWxYz"),
            ["a GitHub token"]
        );
        assert!(kinds("ghp_short and github_pat_x").is_empty());
    }

    #[test]
    fn finds_access_tokens() {
        let jwt = "eyJhbGciOiJSUzI1NiJ9.eyJzdWIiOiJhZGEifQ.c2lnbmF0dXJlLWJ5dGVz";
        assert_eq!(
            kinds(&format!("\"access_token\": \"{jwt}\"")),
            ["an access token"]
        );
        // Near miss: something that only starts like one.
        assert!(kinds("eyJhbGciOi.short").is_empty());
    }

    #[test]
    fn finds_private_keys() {
        let key =
            "-----BEGIN OPENSSH PRIVATE KEY-----\nb3BlbnNzaA==\n-----END OPENSSH PRIVATE KEY-----";
        let text = format!("Here:\n{key}\nDone.");
        let found = find(&text);
        assert_eq!(found.len(), 1);
        assert_eq!(&text[found[0].start..found[0].end], key);
        // Near miss: a public key.
        assert!(kinds("-----BEGIN PUBLIC KEY-----\nAAAA\n-----END PUBLIC KEY-----").is_empty());
    }

    #[test]
    fn hides_and_parts_keep_the_rest() {
        let text = format!("Your token is stm_{B64}, keep it. Done: $x^2$");
        assert_eq!(hide(&text), "Your token is [hidden], keep it. Done: $x^2$");
        let parts = parts(&text);
        assert_eq!(parts.len(), 3);
        assert_eq!(parts[1].secret.as_deref(), Some("a Stemma token"));
        assert_eq!(
            parts.iter().map(|p| p.text.as_str()).collect::<String>(),
            text
        );
        assert_eq!(hide("Nothing — here, «ñ»."), "Nothing — here, «ñ».");
    }
}
