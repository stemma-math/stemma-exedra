//! What changed between two texts, line by line: for the revisions of a
//! working paper.

use std::fmt::Write;

/// One line of a comparison.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Line<'a> {
    Same(&'a str),
    Removed(&'a str),
    Added(&'a str),
}

/// The most cells the comparison of the changed middle may take; beyond
/// it, the middle is taken as replaced whole.
const MAX_CELLS: usize = 4_000_000;

/// Compares two texts line by line: the lines of `new` kept from `old`,
/// those added and those removed, in order.
pub fn lines<'a>(old: &'a str, new: &'a str) -> Vec<Line<'a>> {
    let a: Vec<&str> = old.lines().collect();
    let b: Vec<&str> = new.lines().collect();
    let prefix = a.iter().zip(&b).take_while(|(x, y)| x == y).count();
    let suffix = a[prefix..]
        .iter()
        .rev()
        .zip(b[prefix..].iter().rev())
        .take_while(|(x, y)| x == y)
        .count();
    let (am, bm) = (&a[prefix..a.len() - suffix], &b[prefix..b.len() - suffix]);

    let mut out: Vec<Line> = a[..prefix].iter().map(|l| Line::Same(l)).collect();
    if am.len().saturating_mul(bm.len()) > MAX_CELLS {
        out.extend(am.iter().map(|l| Line::Removed(l)));
        out.extend(bm.iter().map(|l| Line::Added(l)));
    } else {
        // The longest common subsequence, from the end.
        let (n, m) = (am.len(), bm.len());
        let mut table = vec![0u32; (n + 1) * (m + 1)];
        let at = |i: usize, j: usize| i * (m + 1) + j;
        for i in (0..n).rev() {
            for j in (0..m).rev() {
                table[at(i, j)] = if am[i] == bm[j] {
                    table[at(i + 1, j + 1)] + 1
                } else {
                    table[at(i + 1, j)].max(table[at(i, j + 1)])
                };
            }
        }
        let (mut i, mut j) = (0, 0);
        while i < n || j < m {
            if i < n && j < m && am[i] == bm[j] {
                out.push(Line::Same(am[i]));
                i += 1;
                j += 1;
            } else if i < n && (j == m || table[at(i + 1, j)] >= table[at(i, j + 1)]) {
                // What goes comes before what replaces it.
                out.push(Line::Removed(am[i]));
                i += 1;
            } else {
                out.push(Line::Added(bm[j]));
                j += 1;
            }
        }
    }
    out.extend(a[a.len() - suffix..].iter().map(|l| Line::Same(l)));
    out
}

/// What changed, as people and agents read it: each change with a few
/// lines around it, `-` before what was removed and `+` before what was
/// added, and the line of the new text it is at.
pub fn unified(old: &str, new: &str, context: usize) -> String {
    let diff = lines(old, new);
    let changed: Vec<usize> = diff
        .iter()
        .enumerate()
        .filter(|(_, l)| !matches!(l, Line::Same(_)))
        .map(|(i, _)| i)
        .collect();
    if changed.is_empty() {
        return String::new();
    }
    // The stretches to show: each change and its context, merged.
    let mut hunks: Vec<(usize, usize)> = Vec::new();
    for &i in &changed {
        let (from, to) = (i.saturating_sub(context), (i + context + 1).min(diff.len()));
        match hunks.last_mut() {
            Some(last) if from <= last.1 => last.1 = to,
            _ => hunks.push((from, to)),
        }
    }
    let mut out = String::new();
    for (from, to) in hunks {
        let new_line = diff[..from]
            .iter()
            .filter(|l| !matches!(l, Line::Removed(_)))
            .count()
            + 1;
        let _ = writeln!(out, "@@ line {new_line}");
        for line in &diff[from..to] {
            let _ = match line {
                Line::Same(text) => writeln!(out, "  {text}"),
                Line::Removed(text) => writeln!(out, "- {text}"),
                Line::Added(text) => writeln!(out, "+ {text}"),
            };
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_what_changed() {
        let old = "a\nb\nc\nd\n";
        let new = "a\nB\nc\nd\ne\n";
        assert_eq!(
            lines(old, new),
            [
                Line::Same("a"),
                Line::Removed("b"),
                Line::Added("B"),
                Line::Same("c"),
                Line::Same("d"),
                Line::Added("e"),
            ]
        );
        assert!(lines(old, old).iter().all(|l| matches!(l, Line::Same(_))));
    }

    #[test]
    fn writes_changes_with_context() {
        let old = "1\n2\n3\n4\n5\n6\n7\n8\n9\n";
        let new = "1\n2\n3\nfour\n5\n6\n7\n8\n9\nten\n";
        assert_eq!(
            unified(old, new, 1),
            "@@ line 3\n  3\n- 4\n+ four\n  5\n@@ line 9\n  9\n+ ten\n"
        );
        assert_eq!(unified(old, old, 2), "");
        assert_eq!(unified("", "new\n", 2), "@@ line 1\n+ new\n");
    }
}
