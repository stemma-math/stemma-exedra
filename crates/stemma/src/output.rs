//! Terminal composition uses only Stemma's roles, symbols and colour policy.
use stemma_design_terminal::{Output, Role, Stream};
pub fn stdout() -> Output {
    Output::detect(Stream::Stdout)
}
pub fn text(text: &str) {
    let out = stdout();
    for line in text.split_inclusive('\n') {
        if line.starts_with('#') {
            print!("{}", out.paint(Role::Strong, line));
            continue;
        }
        for word in line.split_inclusive(char::is_whitespace) {
            let token = word.trim();
            let role = if token.starts_with('@')
                || token.starts_with("p/")
                || token.starts_with("lean:")
                || (token.starts_with('t')
                    && token.as_bytes().get(1).is_some_and(u8::is_ascii_digit))
            {
                Some(Role::Identifier)
            } else if ["queued", "waiting", "offline"]
                .iter()
                .any(|s| token.eq_ignore_ascii_case(s))
            {
                Some(Role::Warning)
            } else if ["done", "connected", "proved"]
                .iter()
                .any(|s| token.eq_ignore_ascii_case(s))
            {
                Some(Role::Success)
            } else if ["failed", "error", "sorry"]
                .iter()
                .any(|s| token.eq_ignore_ascii_case(s))
            {
                Some(Role::Danger)
            } else {
                None
            };
            match role {
                Some(role) => print!("{}", out.paint(role, word)),
                None => print!("{word}"),
            }
        }
    }
}
