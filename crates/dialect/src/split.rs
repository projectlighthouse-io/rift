//! Splitting a script into statements.

use std::ops::Range;

/// Byte ranges of the statements in `sql`, without the trailing `;`, leading whitespace or
/// leading comments. Handles `'…'`, `"…"`, `` `…` ``, `$tag$…$tag$`, `-- …` and `/* … */`.
/// Empty and comment-only statements are skipped.
#[must_use]
pub fn statements(sql: &str) -> Vec<Range<usize>> {
    let bytes = sql.as_bytes();
    let mut out = Vec::new();
    let mut start = 0;
    let mut i = 0;
    let mut has_code = false;
    let mut code_start = None;
    while i < bytes.len() {
        let b = bytes[i];
        match b {
            b'-' if bytes.get(i + 1) == Some(&b'-') => {
                i = sql[i..].find('\n').map_or(bytes.len(), |n| i + n);
                continue;
            }
            b'/' if bytes.get(i + 1) == Some(&b'*') => {
                i = sql[i + 2..]
                    .find("*/")
                    .map_or(bytes.len(), |n| i + 2 + n + 2);
                continue;
            }
            b'\'' | b'"' | b'`' => {
                code_start.get_or_insert(i);
                has_code = true;
                i += 1;
                while i < bytes.len() {
                    if bytes[i] == b {
                        if bytes.get(i + 1) == Some(&b) {
                            i += 2;
                            continue;
                        }
                        break;
                    }
                    if bytes[i] == b'\\' && b == b'`' {
                        i += 1;
                    }
                    i += 1;
                }
                i += 1;
                continue;
            }
            b'$' => {
                code_start.get_or_insert(i);
                has_code = true;
                if let Some(tag_len) = dollar_tag(&sql[i..]) {
                    let tag = &sql[i..i + tag_len];
                    i = sql[i + tag_len..]
                        .find(tag)
                        .map_or(bytes.len(), |n| i + tag_len + n + tag_len);
                    continue;
                }
            }
            b';' => {
                if has_code {
                    out.push(trim_end(sql, code_start.unwrap_or(start), i));
                }
                start = i + 1;
                has_code = false;
                code_start = None;
            }
            b if b.is_ascii_whitespace() => {}
            _ => {
                code_start.get_or_insert(i);
                has_code = true;
            }
        }
        i += 1;
    }
    if has_code {
        out.push(trim_end(sql, code_start.unwrap_or(start), bytes.len()));
    }
    out
}

/// Length of a `$tag$` opener at the start of `s`, if any.
fn dollar_tag(s: &str) -> Option<usize> {
    let rest = &s[1..];
    let end = rest.find('$')?;
    let tag = &rest[..end];
    tag.chars()
        .all(|c| c.is_alphanumeric() || c == '_')
        .then_some(end + 2)
}

fn trim_end(sql: &str, start: usize, end: usize) -> Range<usize> {
    let trimmed = sql[start..end].trim_end();
    start..start + trimmed.len()
}

/// The statement containing (or ending just before) byte offset `cursor`.
#[must_use]
pub fn statement_at(sql: &str, cursor: usize) -> Option<Range<usize>> {
    let all = statements(sql);
    all.iter()
        .find(|r| cursor >= r.start && cursor <= r.end + 1)
        .or_else(|| all.iter().rev().find(|r| r.end <= cursor))
        .or_else(|| all.first())
        .cloned()
}

#[cfg(test)]
mod tests {
    use super::{statement_at, statements};

    fn texts(sql: &str) -> Vec<&str> {
        statements(sql).into_iter().map(|r| &sql[r]).collect()
    }

    #[test]
    fn splits_on_semicolons_outside_quotes_and_comments() {
        let sql = "-- top\nSELECT 'a;b', \"c;d\" FROM t;\n/* x; y */ SELECT 2 -- tail;\n;SELECT $$ ; $$, $f$;$f$";
        assert_eq!(
            texts(sql),
            [
                "SELECT 'a;b', \"c;d\" FROM t",
                "SELECT 2 -- tail;\n",
                "SELECT $$ ; $$, $f$;$f$"
            ]
            .map(|s| s.trim_end())
        );
    }

    #[test]
    fn skips_empty_and_comment_only_statements() {
        assert_eq!(texts(";; -- nothing\n;\n  "), Vec::<&str>::new());
        assert_eq!(texts("select 1;  \n"), ["select 1"]);
    }

    #[test]
    fn finds_statement_under_cursor() {
        let sql = "SELECT 1;\n\nSELECT 2;";
        assert_eq!(statement_at(sql, 3).map(|r| &sql[r]), Some("SELECT 1"));
        assert_eq!(statement_at(sql, 9).map(|r| &sql[r]), Some("SELECT 1"));
        assert_eq!(statement_at(sql, 14).map(|r| &sql[r]), Some("SELECT 2"));
    }
}
