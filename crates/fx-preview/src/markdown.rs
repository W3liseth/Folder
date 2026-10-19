//! Markdown → blocs simples que l'UI dessine avec ses propres widgets (pas de HTML).

use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, Parser, Tag, TagEnd};

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MdSpan {
    pub text: String,
    pub bold: bool,
    pub italic: bool,
    pub code: bool,
    pub link: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum MdBlock {
    Heading(u8, Vec<MdSpan>),
    Paragraph(Vec<MdSpan>),
    /// Élément de liste : profondeur (0 = premier niveau), numéro si liste ordonnée.
    Item {
        depth: u8,
        number: Option<u64>,
        spans: Vec<MdSpan>,
    },
    Quote(Vec<MdSpan>),
    Code {
        lang: String,
        text: String,
    },
    Rule,
}

pub fn parse_markdown(src: &str) -> Vec<MdBlock> {
    let mut blocks = Vec::new();
    let mut spans: Vec<MdSpan> = Vec::new();
    let (mut bold, mut italic, mut link) = (0u32, 0u32, 0u32);
    let mut lists: Vec<Option<u64>> = Vec::new();
    let mut quote = 0u32;
    let mut code: Option<(String, String)> = None;
    let mut heading: Option<u8> = None;
    let mut item: Option<(u8, Option<u64>)> = None;

    let push_text =
        |spans: &mut Vec<MdSpan>, t: &str, bold: u32, italic: u32, link: u32, code: bool| {
            spans.push(MdSpan {
                text: t.to_owned(),
                bold: bold > 0,
                italic: italic > 0,
                code,
                link: link > 0,
            });
        };

    for ev in Parser::new(src) {
        match ev {
            Event::Start(Tag::Heading { level, .. }) => {
                heading = Some(match level {
                    HeadingLevel::H1 => 1,
                    HeadingLevel::H2 => 2,
                    HeadingLevel::H3 => 3,
                    _ => 4,
                });
            }
            Event::End(TagEnd::Heading(_)) => {
                blocks.push(MdBlock::Heading(
                    heading.take().unwrap_or(1),
                    std::mem::take(&mut spans),
                ));
            }
            Event::End(TagEnd::Paragraph) => {
                let s = std::mem::take(&mut spans);
                if s.is_empty() {
                    continue;
                }
                if let Some((depth, number)) = item.take() {
                    blocks.push(MdBlock::Item {
                        depth,
                        number,
                        spans: s,
                    });
                } else if quote > 0 {
                    blocks.push(MdBlock::Quote(s));
                } else {
                    blocks.push(MdBlock::Paragraph(s));
                }
            }
            Event::Start(Tag::List(start)) => lists.push(start),
            Event::End(TagEnd::List(_)) => {
                lists.pop();
            }
            Event::Start(Tag::Item) => {
                let depth = lists.len().saturating_sub(1) as u8;
                let number = lists.last_mut().and_then(|n| {
                    let cur = *n;
                    if let Some(v) = n.as_mut() {
                        *v += 1;
                    }
                    cur
                });
                item = Some((depth, number));
            }
            Event::End(TagEnd::Item) => {
                if let Some((depth, number)) = item.take() {
                    blocks.push(MdBlock::Item {
                        depth,
                        number,
                        spans: std::mem::take(&mut spans),
                    });
                }
            }
            Event::Start(Tag::BlockQuote(_)) => quote += 1,
            Event::End(TagEnd::BlockQuote(_)) => quote = quote.saturating_sub(1),
            Event::Start(Tag::CodeBlock(kind)) => {
                let lang = match kind {
                    CodeBlockKind::Fenced(l) => l.to_string(),
                    CodeBlockKind::Indented => String::new(),
                };
                code = Some((lang, String::new()));
            }
            Event::End(TagEnd::CodeBlock) => {
                if let Some((lang, text)) = code.take() {
                    blocks.push(MdBlock::Code {
                        lang,
                        text: text.trim_end().to_owned(),
                    });
                }
            }
            Event::Start(Tag::Strong) => bold += 1,
            Event::End(TagEnd::Strong) => bold = bold.saturating_sub(1),
            Event::Start(Tag::Emphasis) => italic += 1,
            Event::End(TagEnd::Emphasis) => italic = italic.saturating_sub(1),
            Event::Start(Tag::Link { .. }) => link += 1,
            Event::End(TagEnd::Link) => link = link.saturating_sub(1),
            Event::Text(t) => {
                if let Some((_, buf)) = &mut code {
                    buf.push_str(&t);
                } else {
                    push_text(&mut spans, &t, bold, italic, link, false);
                }
            }
            Event::Code(t) => push_text(&mut spans, &t, bold, italic, link, true),
            Event::SoftBreak | Event::HardBreak => {
                push_text(&mut spans, " ", bold, italic, link, false)
            }
            Event::Rule => blocks.push(MdBlock::Rule),
            _ => {}
        }
    }
    blocks
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocks() {
        let b = parse_markdown(
            "# Titre\n\nUn **gras** et `code`.\n\n- a\n- b\n\n```rust\nfn x() {}\n```\n\n---\n",
        );
        assert!(matches!(&b[0], MdBlock::Heading(1, s) if s[0].text == "Titre"));
        assert!(
            matches!(&b[1], MdBlock::Paragraph(s) if s.iter().any(|x| x.bold) && s.iter().any(|x| x.code))
        );
        assert!(matches!(
            &b[2],
            MdBlock::Item {
                depth: 0,
                number: None,
                ..
            }
        ));
        assert!(matches!(&b[4], MdBlock::Code { lang, .. } if lang == "rust"));
        assert_eq!(b[5], MdBlock::Rule);
    }
}
