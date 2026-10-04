//! Channel-independent content. Channel adapters translate formatting only.
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Span {
    pub text: String,
    pub bold: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "format", content = "content", rename_all = "snake_case")]
pub enum Message {
    Media {
        asset_id: String,
        kind: String,
        path: std::path::PathBuf,
        name: String,
        expires_at: Option<i64>,
    },
    Bundle(Vec<Message>),
    File(std::path::PathBuf),
    Plain(String),
    Markdown(String),
    RichText(Vec<Vec<Span>>),
}
impl Message {
    pub fn has_media(&self) -> bool {
        match self {
            Self::Media { .. } | Self::File(_) => true,
            Self::Bundle(parts) => parts.iter().any(Self::has_media),
            _ => false,
        }
    }
    /// Parse only the small, escaped Markdown vocabulary produced by command
    /// formatters. Decode literal values after recognizing emphasis, so user
    /// values cannot inject formatting. Not a general Agent Markdown parser.
    pub fn command_text(text: &str) -> Self {
        fn literal(text: &str) -> String {
            let mut result = String::new();
            let mut chars = text.chars().peekable();
            while let Some(c) = chars.next() {
                if c == '\\' && chars.peek().is_some_and(|c| "\\|*_[]`".contains(*c)) {
                    result.push(chars.next().unwrap());
                } else {
                    result.push(c);
                }
            }
            result
                .replace("&lt;", "<")
                .replace("&gt;", ">")
                .replace("&amp;", "&")
        }
        let rows = flatten_tables(text)
            .lines()
            .map(|line| {
                let mut spans = Vec::new();
                let mut buffer = String::new();
                let mut bold = false;
                let mut chars = line.chars().peekable();
                while let Some(c) = chars.next() {
                    if c == '\\' {
                        buffer.push(c);
                        if let Some(next) = chars.next() {
                            buffer.push(next);
                        }
                    } else if c == '*' && chars.peek() == Some(&'*') {
                        chars.next();
                        if !buffer.is_empty() {
                            spans.push(Span {
                                text: literal(&buffer),
                                bold,
                            });
                            buffer.clear();
                        }
                        bold = !bold;
                    } else {
                        buffer.push(c);
                    }
                }
                if !buffer.is_empty() {
                    spans.push(Span {
                        text: literal(&buffer),
                        bold,
                    });
                }
                spans
            })
            .collect();
        Self::RichText(rows)
    }
    /// Compatibility for older outbox rows and existing text producers.
    pub fn legacy(text: &str) -> Self {
        if serde_json::from_str::<serde_json::Value>(text).is_ok() {
            Self::Plain(text.into())
        } else {
            Self::Markdown(text.into())
        }
    }
    pub fn plain_text(&self) -> String {
        match self {
            Self::Media { name, .. } => name.clone(),
            Self::Bundle(parts) => parts.iter().map(Self::plain_text).collect::<Vec<_>>().join(
                "
",
            ),
            Self::File(path) => path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned(),
            Self::Plain(text) | Self::Markdown(text) => text.clone(),
            Self::RichText(rows) => rows
                .iter()
                .map(|row| {
                    row.iter()
                        .map(|span| span.text.as_str())
                        .collect::<String>()
                })
                .collect::<Vec<_>>()
                .join("\n"),
        }
    }
}

/// Convert GFM tables to labelled records for Lark post Markdown.
pub fn flatten_tables(text: &str) -> String {
    fn cells(line: &str) -> Vec<String> {
        let mut cells = vec![String::new()];
        let mut escaped = false;
        for ch in line
            .trim()
            .trim_start_matches('|')
            .trim_end_matches('|')
            .chars()
        {
            if ch == '|' && !escaped {
                cells.push(String::new());
            } else {
                cells.last_mut().unwrap().push(ch);
            }
            escaped = ch == '\\' && !escaped;
        }
        cells.into_iter().map(|c| c.trim().to_owned()).collect()
    }
    let lines: Vec<_> = text.lines().collect();
    let mut out = Vec::new();
    let mut i = 0;
    let mut fence: Option<char> = None;
    while i < lines.len() {
        let marker = lines[i].trim_start().chars().next();
        if lines[i].trim_start().starts_with("```") || lines[i].trim_start().starts_with("~~~") {
            if fence.is_none() {
                fence = marker;
            } else if fence == marker {
                fence = None;
            }
            out.push(lines[i].to_owned());
            i += 1;
            continue;
        }
        if fence.is_some() {
            out.push(lines[i].to_owned());
            i += 1;
            continue;
        }
        let headers = cells(lines[i]);
        let is_table = lines[i].trim().starts_with('|')
            && i + 1 < lines.len()
            && lines[i + 1].trim().starts_with('|')
            && cells(lines[i + 1])
                .iter()
                .all(|s| s.contains("---") && s.chars().all(|c| matches!(c, '-' | ':' | ' ')));
        if !is_table {
            out.push(lines[i].to_owned());
            i += 1;
            continue;
        }
        i += 2;
        while i < lines.len() && lines[i].trim().starts_with('|') {
            let values = cells(lines[i]);
            if values.len() != headers.len() {
                out.push(lines[i].to_owned());
            } else {
                for (header, value) in headers.iter().zip(values) {
                    out.push(format!("- **{header}**：{value}"));
                }
                out.push(String::new());
            }
            i += 1;
        }
    }
    out.join("\n")
}
