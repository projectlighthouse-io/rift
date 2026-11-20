//! SQL highlighting in the design's colours, and completions from the cached schema.

use std::ops::Range;
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use gpui::{App, Context, FontStyle, HighlightStyle, SharedString, Task, Window};
use gpui_component::input::{
    CompletionProvider, EditorState, FoldRange, HighlightStyleResolver, InputEdit,
    InputHighlighter, InputHighlighterFactory, Rope,
};
use lsp_types::{CompletionContext, CompletionItem, CompletionItemKind, CompletionResponse};

use crate::theme;

const KEYWORDS: &[&str] = &[
    "add",
    "all",
    "alter",
    "and",
    "as",
    "asc",
    "begin",
    "between",
    "by",
    "case",
    "cast",
    "column",
    "commit",
    "create",
    "cross",
    "default",
    "delete",
    "desc",
    "distinct",
    "drop",
    "else",
    "end",
    "except",
    "exists",
    "explain",
    "false",
    "from",
    "full",
    "group",
    "having",
    "ilike",
    "in",
    "index",
    "inner",
    "insert",
    "intersect",
    "interval",
    "into",
    "is",
    "join",
    "left",
    "like",
    "limit",
    "not",
    "null",
    "offset",
    "on",
    "or",
    "order",
    "outer",
    "over",
    "partition",
    "primary",
    "references",
    "returning",
    "right",
    "rollback",
    "select",
    "set",
    "table",
    "then",
    "true",
    "truncate",
    "union",
    "unique",
    "update",
    "using",
    "values",
    "view",
    "when",
    "where",
    "with",
];

const KEYWORD: u32 = 0x7c3aed;
const FUNCTION: u32 = 0x0369a1;
const STRING: u32 = 0x15803d;
const NUMBER: u32 = 0xb45309;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Keyword,
    Function,
    String,
    Number,
    Comment,
}

/// Tokenizes for colouring only; anything unrecognised is plain text.
fn tokenize(text: &str) -> Vec<(Range<usize>, Kind)> {
    let b = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        let start = i;
        match b[i] {
            b'-' if b.get(i + 1) == Some(&b'-') => {
                i = text[i..].find('\n').map_or(b.len(), |n| i + n);
                out.push((start..i, Kind::Comment));
            }
            b'/' if b.get(i + 1) == Some(&b'*') => {
                i = text[i + 2..].find("*/").map_or(b.len(), |n| i + 4 + n);
                out.push((start..i, Kind::Comment));
            }
            b'\'' => {
                i += 1;
                while i < b.len() && !(b[i] == b'\'' && b.get(i + 1) != Some(&b'\'')) {
                    i += if b[i] == b'\'' { 2 } else { 1 };
                }
                i = (i + 1).min(b.len());
                out.push((start..i, Kind::String));
            }
            c if c.is_ascii_digit() => {
                while i < b.len() && (b[i].is_ascii_digit() || b[i] == b'.') {
                    i += 1;
                }
                out.push((start..i, Kind::Number));
            }
            c if c.is_ascii_alphabetic() || c == b'_' => {
                while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_') {
                    i += 1;
                }
                let word = text[start..i].to_ascii_lowercase();
                let prev_dot = start > 0 && b[start - 1] == b'.';
                if text[i..].trim_start().starts_with('(') && !prev_dot {
                    out.push((start..i, Kind::Function));
                } else if KEYWORDS.contains(&word.as_str()) && !prev_dot {
                    out.push((start..i, Kind::Keyword));
                }
            }
            _ => {
                // Skip one UTF-8 character.
                i += text[i..].chars().next().map_or(1, char::len_utf8);
            }
        }
    }
    out
}

fn style(kind: Kind) -> HighlightStyle {
    let color = match kind {
        Kind::Keyword => KEYWORD,
        Kind::Function => FUNCTION,
        Kind::String => STRING,
        Kind::Number => NUMBER,
        Kind::Comment => theme::FAINT,
    };
    HighlightStyle {
        color: Some(theme::hsla(color)),
        font_style: (kind == Kind::Comment).then_some(FontStyle::Italic),
        ..HighlightStyle::default()
    }
}

/// Highlighter installed into the editor.
#[derive(Default)]
struct SqlHighlighter {
    tokens: Vec<(Range<usize>, Kind)>,
}

impl InputHighlighter for SqlHighlighter {
    fn language(&self) -> SharedString {
        "sql".into()
    }

    fn update(
        &mut self,
        _: Option<InputEdit>,
        text: &Rope,
        _: bool,
        _: &mut Window,
        _: &mut Context<EditorState>,
    ) {
        self.tokens = tokenize(&text.to_string());
    }

    fn styles(
        &self,
        range: &Range<usize>,
        _: &dyn HighlightStyleResolver,
    ) -> Vec<(Range<usize>, HighlightStyle)> {
        let mut out = Vec::new();
        let mut pos = range.start;
        for (r, kind) in &self.tokens {
            if r.end <= range.start || r.start >= range.end {
                continue;
            }
            let s = r.start.max(range.start);
            let e = r.end.min(range.end);
            if s > pos {
                out.push((pos..s, HighlightStyle::default()));
            }
            out.push((s..e, style(*kind)));
            pos = e;
        }
        if pos < range.end {
            out.push((pos..range.end, HighlightStyle::default()));
        }
        out
    }

    fn fold_ranges(&self, _: &Rope) -> Vec<FoldRange> {
        Vec::new()
    }
}

/// Factory for [`EditorState::set_highlighter_factory`].
pub(crate) fn highlighter_factory() -> InputHighlighterFactory {
    Rc::new(|_| Some(Box::new(SqlHighlighter::default())))
}

/// Names offered by autocomplete: tables, columns and keywords.
#[derive(Clone, Default)]
pub(crate) struct SchemaNames(pub(crate) Arc<Mutex<Vec<(String, CompletionItemKind)>>>);

impl SchemaNames {
    pub(crate) fn set(&self, names: Vec<(String, CompletionItemKind)>) {
        *self.0.lock().expect("schema names poisoned") = names;
    }
}

/// Completes the word before the cursor from [`SchemaNames`] and keywords.
pub(crate) struct SqlCompletions {
    pub(crate) names: SchemaNames,
}

impl CompletionProvider for SqlCompletions {
    fn completions(
        &self,
        text: &Rope,
        offset: usize,
        _: CompletionContext,
        _: &mut Window,
        _: &mut App,
    ) -> Task<anyhow::Result<CompletionResponse>> {
        let text = text.to_string();
        let before = &text[..offset.min(text.len())];
        let word: String = before
            .chars()
            .rev()
            .take_while(|c| c.is_alphanumeric() || *c == '_')
            .collect::<String>()
            .chars()
            .rev()
            .collect();
        let prefix = word.to_lowercase();
        if prefix.is_empty() {
            return Task::ready(Ok(CompletionResponse::Array(Vec::new())));
        }
        let names = self.names.0.lock().expect("schema names poisoned").clone();
        let mut items: Vec<CompletionItem> = names
            .into_iter()
            .chain(
                KEYWORDS
                    .iter()
                    .map(|k| (k.to_uppercase(), CompletionItemKind::KEYWORD)),
            )
            .filter(|(name, _)| {
                name.to_lowercase().starts_with(&prefix) && name.to_lowercase() != prefix
            })
            .map(|(name, kind)| CompletionItem {
                label: name,
                kind: Some(kind),
                ..CompletionItem::default()
            })
            .collect();
        items.sort_by(|a, b| {
            a.label
                .len()
                .cmp(&b.label.len())
                .then(a.label.cmp(&b.label))
        });
        items.dedup_by(|a, b| a.label == b.label);
        items.truncate(40);
        Task::ready(Ok(CompletionResponse::Array(items)))
    }

    fn is_completion_trigger(&self, _: usize, new_text: &str, _: &mut App) -> bool {
        new_text
            .chars()
            .all(|c| c.is_alphanumeric() || c == '_' || c == '.')
    }
}

#[cfg(test)]
mod tests {
    use super::{Kind, tokenize};

    #[test]
    fn colours_keywords_functions_strings_numbers_comments() {
        let sql = "-- c\nSELECT count(o.id), 'x''y' FROM t LIMIT 50";
        let kinds: Vec<(&str, Kind)> = tokenize(sql)
            .into_iter()
            .map(|(r, k)| (&sql[r], k))
            .collect();
        assert!(kinds.contains(&("-- c", Kind::Comment)));
        assert!(kinds.contains(&("SELECT", Kind::Keyword)));
        assert!(kinds.contains(&("count", Kind::Function)));
        assert!(kinds.contains(&("'x''y'", Kind::String)));
        assert!(kinds.contains(&("50", Kind::Number)));
        assert!(!kinds.iter().any(|(t, _)| *t == "id"));
    }
}
