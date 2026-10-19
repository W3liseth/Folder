//! Coloration syntaxique légère : commentaires, chaînes, nombres et mots-clés, par une
//! simple passe lexicale (pas de grammaire complète, pas de dépendance lourde).

use std::ops::Range;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    Rust,
    CLike,
    Python,
    Shell,
    Json,
    Toml,
    Yaml,
    Markup,
    Sql,
    Plain,
}

impl Lang {
    pub fn from_ext(ext: &str) -> Self {
        match ext {
            "rs" => Self::Rust,
            "c" | "h" | "cpp" | "hpp" | "cc" | "cs" | "java" | "js" | "mjs" | "ts" | "tsx"
            | "jsx" | "go" | "kt" | "swift" | "css" | "scss" | "php" | "dart" => Self::CLike,
            "py" | "pyw" => Self::Python,
            "sh" | "bash" | "ps1" | "psm1" | "bat" | "cmd" => Self::Shell,
            "json" | "jsonc" => Self::Json,
            "toml" | "ini" | "cfg" | "conf" => Self::Toml,
            "yaml" | "yml" => Self::Yaml,
            "xml" | "html" | "htm" | "svg" | "xaml" | "csproj" | "vcxproj" => Self::Markup,
            "sql" => Self::Sql,
            _ => Self::Plain,
        }
    }

    fn line_comment(self) -> Option<&'static str> {
        match self {
            Self::Rust | Self::CLike | Self::Json => Some("//"),
            Self::Python | Self::Shell | Self::Toml | Self::Yaml => Some("#"),
            Self::Sql => Some("--"),
            Self::Markup | Self::Plain => None,
        }
    }

    fn block_comment(self) -> Option<(&'static str, &'static str)> {
        match self {
            Self::Rust | Self::CLike | Self::Sql => Some(("/*", "*/")),
            Self::Markup => Some(("<!--", "-->")),
            _ => None,
        }
    }

    fn keywords(self) -> &'static [&'static str] {
        match self {
            Self::Rust => &[
                "as", "async", "await", "break", "const", "continue", "crate", "dyn", "else",
                "enum", "extern", "false", "fn", "for", "if", "impl", "in", "let", "loop", "match",
                "mod", "move", "mut", "pub", "ref", "return", "self", "Self", "static", "struct",
                "super", "trait", "true", "type", "unsafe", "use", "where", "while",
            ],
            Self::CLike => &[
                "auto",
                "break",
                "case",
                "catch",
                "class",
                "const",
                "continue",
                "default",
                "do",
                "else",
                "enum",
                "export",
                "extends",
                "false",
                "final",
                "for",
                "func",
                "function",
                "if",
                "import",
                "interface",
                "let",
                "namespace",
                "new",
                "null",
                "package",
                "private",
                "protected",
                "public",
                "return",
                "static",
                "struct",
                "switch",
                "this",
                "throw",
                "true",
                "try",
                "typedef",
                "var",
                "void",
                "while",
                "using",
                "async",
                "await",
            ],
            Self::Python => &[
                "and", "as", "async", "await", "break", "class", "continue", "def", "elif", "else",
                "except", "False", "finally", "for", "from", "if", "import", "in", "is", "lambda",
                "None", "not", "or", "pass", "raise", "return", "True", "try", "while", "with",
                "yield",
            ],
            Self::Shell => &[
                "if", "then", "else", "fi", "for", "do", "done", "function", "echo", "set",
                "param", "foreach", "goto", "call", "exit",
            ],
            Self::Json | Self::Yaml | Self::Toml => &["true", "false", "null"],
            Self::Sql => &[
                "SELECT", "FROM", "WHERE", "INSERT", "INTO", "UPDATE", "DELETE", "CREATE", "TABLE",
                "JOIN", "LEFT", "ON", "AND", "OR", "NOT", "NULL", "ORDER", "BY", "GROUP", "VALUES",
                "AS", "PRIMARY", "KEY",
            ],
            Self::Markup | Self::Plain => &[],
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenKind {
    Comment,
    String,
    Number,
    Keyword,
    Tag,
}

/// Plage d'octets de `text` et sa nature (le texte non couvert est ordinaire).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Token {
    pub range: Range<usize>,
    pub kind: TokenKind,
}

pub fn highlight(text: &str, lang: Lang) -> Vec<Token> {
    if lang == Lang::Plain {
        return Vec::new();
    }
    let b = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    let kw = lang.keywords();
    while i < b.len() {
        let rest = &text[i..];
        if let Some(lc) = lang.line_comment()
            && rest.starts_with(lc)
        {
            let end = rest.find('\n').map_or(b.len(), |n| i + n);
            out.push(Token {
                range: i..end,
                kind: TokenKind::Comment,
            });
            i = end;
            continue;
        }
        if let Some((open, close)) = lang.block_comment()
            && rest.starts_with(open)
        {
            let end = rest[open.len()..]
                .find(close)
                .map_or(b.len(), |n| i + open.len() + n + close.len());
            out.push(Token {
                range: i..end,
                kind: TokenKind::Comment,
            });
            i = end;
            continue;
        }
        let c = b[i];
        if lang == Lang::Markup && c == b'<' {
            let end = rest
                .find(|ch: char| ch.is_whitespace() || ch == '>')
                .map_or(b.len(), |n| i + n);
            out.push(Token {
                range: i..end,
                kind: TokenKind::Tag,
            });
            i = end.max(i + 1);
            continue;
        }
        if c == b'"' || c == b'\'' || (c == b'`' && lang == Lang::CLike) {
            let mut j = i + 1;
            while j < b.len() && b[j] != c && b[j] != b'\n' {
                j += if b[j] == b'\\' { 2 } else { 1 };
            }
            let end = (j + 1).min(b.len());
            out.push(Token {
                range: i..end,
                kind: TokenKind::String,
            });
            i = end;
            continue;
        }
        if c.is_ascii_digit() && (i == 0 || !is_ident(b[i - 1])) {
            let mut j = i;
            while j < b.len() && (b[j].is_ascii_alphanumeric() || b[j] == b'.' || b[j] == b'_') {
                j += 1;
            }
            out.push(Token {
                range: i..j,
                kind: TokenKind::Number,
            });
            i = j;
            continue;
        }
        if is_ident(c) {
            let mut j = i;
            while j < b.len() && is_ident(b[j]) {
                j += 1;
            }
            let word = &text[i..j];
            let hit = if lang == Lang::Sql {
                kw.iter().any(|k| k.eq_ignore_ascii_case(word))
            } else {
                kw.contains(&word)
            };
            if hit {
                out.push(Token {
                    range: i..j,
                    kind: TokenKind::Keyword,
                });
            }
            i = j;
            continue;
        }
        // Avance d'un caractère UTF-8 complet.
        i += rest.chars().next().map_or(1, char::len_utf8);
    }
    out
}

fn is_ident(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_'
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rust_tokens() {
        let t = "fn main() { let s = \"é\"; // fin\n 42 }";
        let toks = highlight(t, Lang::Rust);
        let kinds: Vec<(&str, TokenKind)> =
            toks.iter().map(|k| (&t[k.range.clone()], k.kind)).collect();
        assert!(kinds.contains(&("fn", TokenKind::Keyword)));
        assert!(kinds.contains(&("\"é\"", TokenKind::String)));
        assert!(kinds.contains(&("// fin", TokenKind::Comment)));
        assert!(kinds.contains(&("42", TokenKind::Number)));
    }
}
