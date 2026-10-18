//! The forum's search: a query, with its operators, over the index of
//! thread titles and messages that the database keeps (SQLite's FTS5).
//!
//! A query is words, and phrases in quotes, all of which a result has (a
//! word also matches words it begins), and operators that narrow it:
//!
//! - `author:ada`: written by Ada (her agents' messages are hers too), or
//!   for a title, started by her;
//! - `since:2026-03-01`, `since:7d` (or `2w`, `3m`, `1y`), and `until:` the
//!   same: when it was written, or for a title, when its thread was last
//!   active; `until:` a day includes that day;
//! - `in:forum` (discussions), `in:chats`, `in:papers` (threads about a
//!   paper), `in:p/notes` (about that one), `in:library` (about a
//!   declaration), `in:t12` (in that thread);
//! - `kind:thread` (titles only), `kind:message`, `kind:agent` (messages
//!   agents wrote), `kind:person` (messages people wrote).
//!
//! Without words, a query lists what its operators allow, newest first:
//! threads, or messages when it names an author or a kind of message.

use sqlx::AssertSqlSafe;
use stemma_core::Address;
use stemma_core::api::{SearchHit, ThreadKind, User};
use time::{Date, Month, Time};

use crate::db;
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;

const DAY_MS: i64 = 24 * 60 * 60 * 1000;
/// The most words and phrases a query has.
const MAX_TERMS: usize = 12;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Term {
    Word(String),
    Phrase(String),
}

/// Where results are.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Scope {
    Forum,
    Chats,
    Papers,
    Paper(String),
    Library,
    Thread(u32),
}

/// What results are.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HitKind {
    Thread,
    Message,
    Agent,
    Person,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Query {
    pub terms: Vec<Term>,
    pub author: Option<String>,
    /// From this moment on, in milliseconds since the epoch.
    pub since: Option<i64>,
    /// Before this moment.
    pub until: Option<i64>,
    pub scope: Option<Scope>,
    pub kind: Option<HitKind>,
}

impl Query {
    /// Parses a query, with `now` for relative times.
    pub fn parse(text: &str, now: i64) -> Result<Query, String> {
        let mut query = Query::default();
        for token in tokens(text) {
            let (operator, value) = match &token {
                Token::Phrase(phrase) => {
                    query.terms.push(Term::Phrase(phrase.clone()));
                    continue;
                }
                Token::Plain(plain) => match plain.split_once(':') {
                    Some((op, value)) if OPERATORS.contains(&op.to_lowercase().as_str()) => {
                        (op.to_lowercase(), value.to_owned())
                    }
                    _ => {
                        query.terms.push(Term::Word(plain.clone()));
                        continue;
                    }
                },
            };
            let value = value.trim_matches('"');
            if value.is_empty() {
                return Err(format!("Say what {operator}: is"));
            }
            match operator.as_str() {
                "author" => {
                    query.author = Some(value.trim_start_matches('@').to_lowercase());
                }
                "since" => query.since = Some(moment(value, now, false)?),
                "until" => query.until = Some(moment(value, now, true)?),
                "in" => query.scope = Some(scope(value)?),
                "kind" => query.kind = Some(kind(value)?),
                _ => unreachable!("an operator of OPERATORS"),
            }
        }
        // Words with no letter or digit match nothing in the index.
        query.terms.retain(|term| match term {
            Term::Word(w) | Term::Phrase(w) => w.chars().any(char::is_alphanumeric),
        });
        if query.terms.len() > MAX_TERMS {
            return Err(format!("Search for at most {MAX_TERMS} words"));
        }
        Ok(query)
    }

    fn is_empty(&self) -> bool {
        self.terms.is_empty()
            && self.author.is_none()
            && self.since.is_none()
            && self.until.is_none()
            && self.scope.is_none()
            && self.kind.is_none()
    }

    /// The query for the index: each term in quotes, words as prefixes.
    pub fn fts(&self) -> Option<String> {
        if self.terms.is_empty() {
            return None;
        }
        let quoted = |s: &str| format!("\"{}\"", s.replace('"', "\"\""));
        Some(
            self.terms
                .iter()
                .map(|term| match term {
                    Term::Word(word) => format!("{}*", quoted(word)),
                    Term::Phrase(phrase) => quoted(phrase),
                })
                .collect::<Vec<_>>()
                .join(" "),
        )
    }
}

const OPERATORS: &[&str] = &["author", "since", "until", "in", "kind"];

enum Token {
    Plain(String),
    Phrase(String),
}

/// Splits a query at spaces, keeping what is in quotes together; an
/// operator's value may be in quotes too (`author:"ada"`).
fn tokens(text: &str) -> Vec<Token> {
    let mut found = Vec::new();
    let mut chars = text.chars().peekable();
    while let Some(&c) = chars.peek() {
        if c.is_whitespace() {
            chars.next();
        } else if c == '"' {
            chars.next();
            let phrase: String = chars.by_ref().take_while(|&c| c != '"').collect();
            let phrase = phrase.split_whitespace().collect::<Vec<_>>().join(" ");
            if !phrase.is_empty() {
                found.push(Token::Phrase(phrase));
            }
        } else {
            let mut plain = String::new();
            let mut quoted = false;
            while let Some(&c) = chars.peek() {
                if c.is_whitespace() && !quoted {
                    break;
                }
                if c == '"' {
                    quoted = !quoted;
                }
                plain.push(c);
                chars.next();
            }
            found.push(Token::Plain(plain));
        }
    }
    found
}

/// A moment: a day (its start, or with `end`, its end), or a time ago.
fn moment(value: &str, now: i64, end: bool) -> Result<i64, String> {
    let wrong = || format!("“{value}” is not a day (2026-03-01) nor a time ago (7d, 2w, 3m, 1y)");
    if let Some(unit) = value.chars().last()
        && let Ok(count) = value[..value.len() - unit.len_utf8()].parse::<i64>()
    {
        let days = match unit {
            'd' => 1,
            'w' => 7,
            'm' => 30,
            'y' => 365,
            _ => return Err(wrong()),
        };
        return Ok(now - count.clamp(0, 100_000) * days * DAY_MS);
    }
    let mut parts = value.splitn(3, '-').map(str::parse::<i32>);
    let (Some(Ok(year)), Some(Ok(month)), Some(Ok(day))) =
        (parts.next(), parts.next(), parts.next())
    else {
        return Err(wrong());
    };
    let month = u8::try_from(month)
        .ok()
        .and_then(|m| Month::try_from(m).ok())
        .ok_or_else(wrong)?;
    let date = u8::try_from(day)
        .ok()
        .and_then(|d| Date::from_calendar_date(year, month, d).ok())
        .ok_or_else(wrong)?;
    let start = db::to_millis(date.with_time(Time::MIDNIGHT).assume_utc());
    Ok(if end { start + DAY_MS } else { start })
}

fn scope(value: &str) -> Result<Scope, String> {
    match value.to_lowercase().as_str() {
        "forum" | "discussions" => Ok(Scope::Forum),
        "chats" => Ok(Scope::Chats),
        "papers" => Ok(Scope::Papers),
        "library" => Ok(Scope::Library),
        other => match other.parse::<Address>() {
            Ok(Address::Paper(slug)) => Ok(Scope::Paper(slug)),
            Ok(Address::Thread(number)) => Ok(Scope::Thread(number)),
            _ => Err(format!(
                "in: is forum, chats, papers, library, a paper (p/notes) or a thread (t12), not “{value}”"
            )),
        },
    }
}

fn kind(value: &str) -> Result<HitKind, String> {
    match value.to_lowercase().as_str() {
        "thread" | "threads" | "title" => Ok(HitKind::Thread),
        "message" | "messages" => Ok(HitKind::Message),
        "agent" | "agents" => Ok(HitKind::Agent),
        "person" | "people" => Ok(HitKind::Person),
        _ => Err(format!(
            "kind: is thread, message, agent or person, not “{value}”"
        )),
    }
}

/// A value bound to a query.
enum Bind {
    Text(String),
    Int(i64),
}

/// The conditions on `threads t` a query's scope makes.
fn scope_sql(scope: &Option<Scope>, binds: &mut Vec<Bind>) -> &'static str {
    match scope {
        None => "",
        Some(Scope::Forum) => " AND t.kind = 'discussion'",
        Some(Scope::Chats) => " AND t.kind = 'chat'",
        Some(Scope::Papers) => " AND t.anchor LIKE 'p/%'",
        Some(Scope::Library) => " AND t.anchor LIKE 'lean:%'",
        Some(Scope::Paper(slug)) => {
            binds.push(Bind::Text(format!("p/{slug}")));
            " AND t.anchor = ?"
        }
        Some(Scope::Thread(number)) => {
            binds.push(Bind::Int(i64::from(*number)));
            " AND t.number = ?"
        }
    }
}

type ThreadHitRow = (i64, String, String, i64, i64, String, String);
type MessageHitRow = (
    i64,
    String,
    String,
    i64,
    String,
    Option<String>,
    i64,
    i64,
    String,
    String,
);

/// Runs a query in a group: titles first, then messages, each the best
/// matches first (or the newest, without words).
pub async fn run(
    state: &AppState,
    group_id: i64,
    text: &str,
    limit: u32,
) -> ApiResult<Vec<SearchHit>> {
    let query = Query::parse(text, db::now()).map_err(ApiError::invalid)?;
    if query.is_empty() {
        return Err(ApiError::invalid("Search for at least a word"));
    }
    let fts = query.fts();
    let kind = match (query.kind, &fts, &query.author) {
        (Some(kind), _, _) => Some(kind),
        (None, Some(_), _) => None,
        (None, None, None) => Some(HitKind::Thread),
        (None, None, Some(_)) => Some(HitKind::Message),
    };
    let mut hits = Vec::new();
    if matches!(kind, None | Some(HitKind::Thread)) {
        hits.extend(threads(state, group_id, &query, fts.as_deref(), limit).await?);
    }
    if kind != Some(HitKind::Thread) {
        hits.extend(messages(state, group_id, &query, fts.as_deref(), kind, limit).await?);
    }
    hits.truncate(limit as usize);
    Ok(hits)
}

async fn threads(
    state: &AppState,
    group_id: i64,
    query: &Query,
    fts: Option<&str>,
    limit: u32,
) -> ApiResult<Vec<SearchHit>> {
    let mut binds = vec![Bind::Int(group_id)];
    let mut sql = String::from(
        "SELECT t.number, t.title, t.kind, t.created_at, u.id, u.username, u.display_name
         FROM threads t JOIN users u ON u.id = t.created_by",
    );
    if fts.is_some() {
        sql.push_str(" JOIN threads_fts ON threads_fts.rowid = t.id");
    }
    sql.push_str(" WHERE t.group_id = ?");
    if let Some(fts) = fts {
        sql.push_str(" AND threads_fts MATCH ?");
        binds.push(Bind::Text(fts.to_owned()));
    }
    sql.push_str(scope_sql(&query.scope, &mut binds));
    if let Some(author) = &query.author {
        sql.push_str(" AND u.username = ?");
        binds.push(Bind::Text(author.clone()));
    }
    if let Some(since) = query.since {
        sql.push_str(" AND t.last_activity_at >= ?");
        binds.push(Bind::Int(since));
    }
    if let Some(until) = query.until {
        sql.push_str(" AND t.last_activity_at < ?");
        binds.push(Bind::Int(until));
    }
    sql.push_str(if fts.is_some() {
        " ORDER BY bm25(threads_fts), t.last_activity_at DESC LIMIT ?"
    } else {
        " ORDER BY t.last_activity_at DESC LIMIT ?"
    });
    binds.push(Bind::Int(i64::from(limit)));

    // Only placeholders are added to the query: every value is bound.
    let mut statement = sqlx::query_as::<_, ThreadHitRow>(AssertSqlSafe(sql));
    for bind in binds {
        statement = match bind {
            Bind::Text(text) => statement.bind(text),
            Bind::Int(int) => statement.bind(int),
        };
    }
    let rows = statement.fetch_all(&state.db).await?;
    Ok(rows
        .into_iter()
        .map(
            |(number, title, kind, created_at, id, username, display_name)| SearchHit {
                address: Address::Thread(number as u32),
                snippet: title.clone(),
                thread_title: title,
                thread_kind: ThreadKind::parse(&kind).unwrap_or(ThreadKind::Discussion),
                author: Some(User {
                    id,
                    username,
                    display_name,
                }),
                agent: None,
                created_at: db::from_millis(created_at),
            },
        )
        .collect())
}

async fn messages(
    state: &AppState,
    group_id: i64,
    query: &Query,
    fts: Option<&str>,
    kind: Option<HitKind>,
    limit: u32,
) -> ApiResult<Vec<SearchHit>> {
    let mut binds = vec![Bind::Int(group_id)];
    let mut sql = String::from(if fts.is_some() {
        "SELECT t.number, t.title, t.kind, m.number,
                snippet(messages_fts, 0, '', '', '…', 24), m.agent, m.created_at,
                u.id, u.username, u.display_name
         FROM messages m JOIN threads t ON t.id = m.thread_id
         JOIN users u ON u.id = m.author_id
         JOIN messages_fts ON messages_fts.rowid = m.id"
    } else {
        "SELECT t.number, t.title, t.kind, m.number, substr(m.body, 1, 400), m.agent,
                m.created_at, u.id, u.username, u.display_name
         FROM messages m JOIN threads t ON t.id = m.thread_id
         JOIN users u ON u.id = m.author_id"
    });
    sql.push_str(" WHERE t.group_id = ? AND m.deleted_at IS NULL");
    if let Some(fts) = fts {
        sql.push_str(" AND messages_fts MATCH ?");
        binds.push(Bind::Text(fts.to_owned()));
    }
    sql.push_str(scope_sql(&query.scope, &mut binds));
    match kind {
        Some(HitKind::Agent) => sql.push_str(" AND m.agent IS NOT NULL"),
        Some(HitKind::Person) => sql.push_str(" AND m.agent IS NULL"),
        _ => {}
    }
    if let Some(author) = &query.author {
        sql.push_str(" AND u.username = ?");
        binds.push(Bind::Text(author.clone()));
    }
    if let Some(since) = query.since {
        sql.push_str(" AND m.created_at >= ?");
        binds.push(Bind::Int(since));
    }
    if let Some(until) = query.until {
        sql.push_str(" AND m.created_at < ?");
        binds.push(Bind::Int(until));
    }
    sql.push_str(if fts.is_some() {
        " ORDER BY bm25(messages_fts), m.created_at DESC LIMIT ?"
    } else {
        " ORDER BY m.created_at DESC LIMIT ?"
    });
    binds.push(Bind::Int(i64::from(limit)));

    let mut statement = sqlx::query_as::<_, MessageHitRow>(AssertSqlSafe(sql));
    for bind in binds {
        statement = match bind {
            Bind::Text(text) => statement.bind(text),
            Bind::Int(int) => statement.bind(int),
        };
    }
    let rows = statement.fetch_all(&state.db).await?;
    Ok(rows
        .into_iter()
        .map(
            |(thread, title, kind, number, text, agent, created_at, id, username, display_name)| {
                SearchHit {
                    address: Address::Message {
                        thread: thread as u32,
                        message: number as u32,
                    },
                    thread_title: title,
                    thread_kind: ThreadKind::parse(&kind).unwrap_or(ThreadKind::Discussion),
                    author: Some(User {
                        id,
                        username,
                        display_name,
                    }),
                    agent,
                    snippet: one_line(&text),
                    created_at: db::from_millis(created_at),
                }
            },
        )
        .collect())
}

/// A piece of text on one line, at most a line's worth.
fn one_line(text: &str) -> String {
    const MAX: usize = 240;
    let line = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if line.chars().count() <= MAX {
        return line;
    }
    let mut cut: String = line.chars().take(MAX).collect();
    cut.push('…');
    cut
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_800_000_000_000;

    fn parse(text: &str) -> Query {
        Query::parse(text, NOW).unwrap()
    }

    #[test]
    fn words_phrases_and_operators() {
        let query = parse(r#"gluing "proper map" author:@Ada in:chats kind:agent"#);
        assert_eq!(
            query.terms,
            vec![
                Term::Word("gluing".into()),
                Term::Phrase("proper map".into())
            ]
        );
        assert_eq!(query.author.as_deref(), Some("ada"));
        assert_eq!(query.scope, Some(Scope::Chats));
        assert_eq!(query.kind, Some(HitKind::Agent));
        assert_eq!(query.fts().as_deref(), Some(r#""gluing"* "proper map""#));
    }

    #[test]
    fn places() {
        assert_eq!(
            parse("in:p/notes").scope,
            Some(Scope::Paper("notes".into()))
        );
        assert_eq!(parse("in:t12").scope, Some(Scope::Thread(12)));
        assert_eq!(parse("in:papers").scope, Some(Scope::Papers));
        assert_eq!(parse("in:forum").scope, Some(Scope::Forum));
        assert!(Query::parse("in:nowhere", NOW).is_err());
        assert!(Query::parse("kind:poem", NOW).is_err());
        assert!(Query::parse("author:", NOW).is_err());
    }

    #[test]
    fn moments() {
        assert_eq!(parse("since:7d").since, Some(NOW - 7 * DAY_MS));
        assert_eq!(parse("since:2w").since, Some(NOW - 14 * DAY_MS));
        // 2026-03-01 at midnight, UTC.
        assert_eq!(parse("since:2026-03-01").since, Some(1_772_323_200_000));
        assert_eq!(
            parse("until:2026-03-01").until,
            Some(1_772_323_200_000 + DAY_MS)
        );
        assert!(Query::parse("since:yesterday", NOW).is_err());
        assert!(Query::parse("since:2026-02-30", NOW).is_err());
    }

    #[test]
    fn what_is_not_an_operator_is_a_word() {
        let query = parse("re:compactness $ \"\" C*-algebra");
        assert_eq!(
            query.terms,
            vec![
                Term::Word("re:compactness".into()),
                Term::Word("C*-algebra".into())
            ]
        );
        // Quotes in a word are doubled in the index's query.
        assert_eq!(parse("a\"b").fts().as_deref(), Some(r#""a""b"*"#));
    }

    #[test]
    fn long_text_is_cut() {
        assert_eq!(one_line("a\n  b"), "a b");
        assert!(one_line(&"x ".repeat(400)).ends_with('…'));
    }
}
