//! A small parser for the subset of Markdown that Discord renders.

use crate::model::MediaRef;

#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub struct Style {
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strike: bool,
    pub spoiler: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Span {
    Text { text: String, style: Style },
    Code { text: String, style: Style },
    Link { text: String, url: String, style: Style },
    Emoji { image: Option<MediaRef>, fallback: String, style: Style },
    Mention { text: String, style: Style },
    Channel { name: String, style: Style },
}

#[derive(Clone, Debug, PartialEq)]
pub enum Block {
    Line(Vec<Span>),
    Heading(u8, Vec<Span>),
    Subtext(Vec<Span>),
    ListItem(Vec<Span>),
    Quote(Vec<Block>),
    Code { text: String },
    Blank,
}

/// What the inline parser needs to know about the message it's parsing.
pub struct Context<'a> {
    /// (pattern as it appears in content, image) for custom and unicode emoji.
    pub emojis: &'a [(&'a str, Option<MediaRef>)],
    /// Display names that are real mentions in this message.
    pub mentions: &'a [&'a str],
    pub channel_exists: &'a dyn Fn(&str) -> bool,
}

pub fn parse(src: &str, cx: &Context) -> Vec<Block> {
    let mut out = Vec::new();
    let mut rest = src;
    let mut quote_lines: Vec<String> = Vec::new();

    let flush_quote = |q: &mut Vec<String>, out: &mut Vec<Block>| {
        if !q.is_empty() {
            let inner = q.join("\n");
            q.clear();
            out.push(Block::Quote(parse(&inner, cx)));
        }
    };

    while !rest.is_empty() {
        // Fenced code block.
        if let Some(after) = rest.strip_prefix("```")
            && let Some(end) = after.find("```")
        {
            flush_quote(&mut quote_lines, &mut out);
            let body = &after[..end];
            // A first line with no spaces is a language tag.
            let body = match body.split_once('\n') {
                Some((first, tail)) if !first.contains(' ') && !tail.is_empty() => tail,
                _ => body,
            };
            out.push(Block::Code { text: body.trim_end_matches('\n').to_string() });
            rest = &after[end + 3..];
            rest = rest.strip_prefix('\n').unwrap_or(rest);
            continue;
        }

        let (line, tail) = match rest.find('\n') {
            Some(i) => (&rest[..i], &rest[i + 1..]),
            None => (rest, ""),
        };

        // ">>> " quotes everything that follows.
        if let Some(q) = line.strip_prefix(">>> ") {
            flush_quote(&mut quote_lines, &mut out);
            let all = format!("{q}{}{tail}", if tail.is_empty() { "" } else { "\n" });
            out.push(Block::Quote(parse(&all, cx)));
            break;
        }
        if let Some(q) = line.strip_prefix("> ").or(if line == ">" { Some("") } else { None }) {
            quote_lines.push(q.to_string());
            rest = tail;
            continue;
        }
        flush_quote(&mut quote_lines, &mut out);

        // A code block opener without a matching close on this line: look ahead.
        if line.contains("```") && !line.starts_with("```") {
            let i = rest.find("```").unwrap();
            if rest[i + 3..].contains("```") {
                out.push(Block::Line(inline(&rest[..i], cx)));
                rest = &rest[i..];
                continue;
            }
        }

        let block = if line.trim().is_empty() {
            Block::Blank
        } else if let Some(h) = line.strip_prefix("### ") {
            Block::Heading(3, inline(h, cx))
        } else if let Some(h) = line.strip_prefix("## ") {
            Block::Heading(2, inline(h, cx))
        } else if let Some(h) = line.strip_prefix("# ") {
            Block::Heading(1, inline(h, cx))
        } else if let Some(h) = line.strip_prefix("-# ") {
            Block::Subtext(inline(h, cx))
        } else if let Some(h) = line.strip_prefix("- ").or_else(|| line.strip_prefix("* ")) {
            Block::ListItem(inline(h, cx))
        } else {
            Block::Line(inline(line, cx))
        };
        out.push(block);
        rest = tail;
    }
    flush_quote(&mut quote_lines, &mut out);

    // Drop leading/trailing blank lines.
    while out.last() == Some(&Block::Blank) {
        out.pop();
    }
    while out.first() == Some(&Block::Blank) {
        out.remove(0);
    }
    out
}

pub fn inline(s: &str, cx: &Context) -> Vec<Span> {
    let mut out = Vec::new();
    parse_inline(s, Style::default(), cx, &mut out);
    merge_text(out)
}

fn merge_text(spans: Vec<Span>) -> Vec<Span> {
    let mut out: Vec<Span> = Vec::with_capacity(spans.len());
    for s in spans {
        if let (Some(Span::Text { text: a, style: sa }), Span::Text { text: b, style: sb }) = (out.last_mut(), &s)
            && sa == sb
        {
            a.push_str(b);
            continue;
        }
        out.push(s);
    }
    out
}

/// Finds a closing delimiter, skipping backslash escapes. Requires non-empty content.
fn find_close(s: &str, delim: &str) -> Option<usize> {
    let bytes = s.as_bytes();
    let mut i = 1;
    while i + delim.len() <= s.len() {
        if bytes[i] == b'\\' {
            i += 2;
            continue;
        }
        if s.is_char_boundary(i) && s[i..].starts_with(delim) {
            return Some(i);
        }
        i += 1;
    }
    None
}

fn url_end(s: &str) -> usize {
    let mut end = s.find(|c: char| c.is_whitespace() || c == '<' || c == '>').unwrap_or(s.len());
    // Trailing punctuation usually isn't part of the URL, unless balancing a paren.
    while end > 0 {
        let last = s[..end].chars().last().unwrap();
        let unbalanced_paren = last == ')' && s[..end].matches('(').count() < s[..end].matches(')').count();
        if matches!(last, '.' | ',' | ':' | ';' | '!' | '?' | '"' | '\'' | '*' | '_' | '~' | '|') || unbalanced_paren {
            end -= last.len_utf8();
        } else {
            break;
        }
    }
    end
}

fn parse_inline(s: &str, style: Style, cx: &Context, out: &mut Vec<Span>) {
    let mut text = String::new();
    let mut i = 0;
    let flush = |text: &mut String, out: &mut Vec<Span>| {
        if !text.is_empty() {
            out.push(Span::Text { text: std::mem::take(text), style });
        }
    };

    while i < s.len() {
        let rest = &s[i..];
        let prev = s[..i].chars().last();

        // Escapes
        if let Some(r) = rest.strip_prefix('\\')
            && let Some(c) = r.chars().next()
            && c.is_ascii_punctuation()
        {
            text.push(c);
            i += 1 + c.len_utf8();
            continue;
        }

        // Emoji (custom ":name:" or unicode), longest pattern first.
        if let Some((pat, img)) = cx.emojis.iter().find(|(p, _)| !p.is_empty() && rest.starts_with(*p)) {
            flush(&mut text, out);
            out.push(Span::Emoji { image: img.clone(), fallback: pat.to_string(), style });
            i += pat.len();
            continue;
        }

        // Inline code: `x` or ``x``
        if rest.starts_with('`') {
            let ticks = if rest.starts_with("``") { "``" } else { "`" };
            if let Some(end) = find_close(&rest[ticks.len() - 1..], ticks).map(|e| e + ticks.len() - 1)
                && end > ticks.len() - 1
            {
                let inner = &rest[ticks.len()..end];
                if !inner.is_empty() {
                    flush(&mut text, out);
                    out.push(Span::Code { text: inner.to_string(), style });
                    i += end + ticks.len();
                    continue;
                }
            }
        }

        // Paired delimiters.
        type Apply = fn(&mut Style);
        let pairs: [(&str, Apply); 6] = [
            ("||", |s| s.spoiler = true),
            ("**", |s| s.bold = true),
            ("__", |s| s.underline = true),
            ("~~", |s| s.strike = true),
            ("*", |s| s.italic = true),
            ("_", |s| s.italic = true),
        ];
        let mut matched = false;
        for (delim, apply) in pairs {
            if !rest.starts_with(delim) {
                continue;
            }
            let after = &rest[delim.len()..];
            // `*` and `_` need non-space content right after the opener; `_` also needs a word boundary.
            if delim.len() == 1 {
                if after.starts_with(char::is_whitespace) || after.starts_with(delim) {
                    continue;
                }
                if delim == "_" && prev.is_some_and(|c| c.is_alphanumeric()) {
                    continue;
                }
            }
            let Some(end) = find_close(&format!(" {after}"), delim).map(|e| e - 1) else { continue };
            if end == 0 {
                continue;
            }
            let inner = &after[..end];
            if delim.len() == 1 {
                if inner.ends_with(char::is_whitespace) {
                    continue;
                }
                let next = after[end + 1..].chars().next();
                if delim == "_" && next.is_some_and(|c| c.is_alphanumeric()) {
                    continue;
                }
            }
            flush(&mut text, out);
            let mut st = style;
            apply(&mut st);
            parse_inline(inner, st, cx, out);
            i += delim.len() + end + delim.len();
            matched = true;
            break;
        }
        if matched {
            continue;
        }

        // Masked links: [text](url)
        if rest.starts_with('[')
            && let Some(close) = rest.find("](")
            && let Some(paren) = rest[close + 2..].find(')')
        {
            let label = &rest[1..close];
            let url = rest[close + 2..close + 2 + paren].trim_matches(['<', '>']);
            if !label.is_empty() && !label.contains('\n') && (url.starts_with("http://") || url.starts_with("https://")) {
                flush(&mut text, out);
                out.push(Span::Link { text: label.to_string(), url: url.to_string(), style });
                i += close + 2 + paren + 1;
                continue;
            }
        }

        // <https://...> (embed-suppressed link)
        if let Some(r) = rest.strip_prefix('<')
            && (r.starts_with("http://") || r.starts_with("https://"))
            && let Some(end) = r.find('>')
        {
            flush(&mut text, out);
            let url = &r[..end];
            out.push(Span::Link { text: url.to_string(), url: url.to_string(), style });
            i += end + 2;
            continue;
        }

        // Bare URLs
        if (rest.starts_with("http://") || rest.starts_with("https://")) && !prev.is_some_and(|c| c.is_alphanumeric()) {
            let end = url_end(rest);
            if end > 8 {
                flush(&mut text, out);
                let url = &rest[..end];
                out.push(Span::Link { text: url.to_string(), url: url.to_string(), style });
                i += end;
                continue;
            }
        }

        // Mentions: DCE writes them as "@display name".
        if rest.starts_with('@') && !prev.is_some_and(|c| c.is_alphanumeric()) {
            let r = &rest[1..];
            let special = ["everyone", "here"].into_iter().find(|n| r.starts_with(n));
            let name = special.or_else(|| {
                cx.mentions
                    .iter()
                    .filter(|n| !n.is_empty() && r.starts_with(**n))
                    .max_by_key(|n| n.len())
                    .copied()
            });
            if let Some(name) = name {
                flush(&mut text, out);
                out.push(Span::Mention { text: format!("@{name}"), style });
                i += 1 + name.len();
                continue;
            }
        }

        // Channel mentions: "#channel-name"
        if rest.starts_with('#') && !prev.is_some_and(|c| c.is_alphanumeric() || c == '&') {
            let r = &rest[1..];
            let end = r.find(|c: char| !(c.is_alphanumeric() || c == '-' || c == '_')).unwrap_or(r.len());
            if end > 0 && (cx.channel_exists)(&r[..end]) {
                flush(&mut text, out);
                out.push(Span::Channel { name: r[..end].to_string(), style });
                i += 1 + end;
                continue;
            }
        }

        let c = rest.chars().next().unwrap();
        text.push(c);
        i += c.len_utf8();
    }
    flush(&mut text, out);
}

/// True if the content is only emoji (and whitespace), which Discord shows large.
pub fn is_emoji_only(blocks: &[Block]) -> bool {
    let mut n = 0;
    for b in blocks {
        match b {
            Block::Line(spans) => {
                for s in spans {
                    match s {
                        Span::Emoji { .. } => n += 1,
                        Span::Text { text, .. } if text.trim().is_empty() => {}
                        _ => return false,
                    }
                }
            }
            Block::Blank => {}
            _ => return false,
        }
    }
    n > 0 && n <= 27
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cx() -> Context<'static> {
        Context { emojis: &[], mentions: &["juni"], channel_exists: &|n| n == "art" }
    }

    fn texts(spans: &[Span]) -> Vec<(String, Style)> {
        spans
            .iter()
            .filter_map(|s| match s {
                Span::Text { text, style } => Some((text.clone(), *style)),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn bold_italic() {
        let s = inline("a **b** *c* _d_ snake_case_word", &cx());
        let t = texts(&s);
        assert_eq!(t[0].0, "a ");
        assert!(t[1].1.bold && t[1].0 == "b");
        assert!(t[3].1.italic && t[3].0 == "c");
        assert!(t[5].1.italic && t[5].0 == "d");
        assert_eq!(t[6].0, " snake_case_word");
    }

    #[test]
    fn links_and_mentions() {
        let s = inline("@juni see https://x.com/a_b. and #art", &cx());
        assert!(matches!(&s[0], Span::Mention { text, .. } if text == "@juni"));
        assert!(matches!(&s[2], Span::Link { url, .. } if url == "https://x.com/a_b"));
        assert!(matches!(s.last(), Some(Span::Channel { name, .. }) if name == "art"));
    }

    #[test]
    fn spoilers_and_code() {
        let s = inline("||secret|| `x*y*`", &cx());
        assert!(matches!(&s[0], Span::Text { style, .. } if style.spoiler));
        assert!(matches!(&s[2], Span::Code { text, .. } if text == "x*y*"));
    }

    #[test]
    fn blocks() {
        let b = parse("> quoted\n> more\nplain\n```rs\nfn x() {}\n```\n# Big", &cx());
        assert!(matches!(&b[0], Block::Quote(inner) if inner.len() == 2));
        assert!(matches!(&b[1], Block::Line(_)));
        assert!(matches!(&b[2], Block::Code { text } if text == "fn x() {}"));
        assert!(matches!(&b[3], Block::Heading(1, _)));
    }

    #[test]
    fn unclosed_is_literal() {
        let t = texts(&inline("2 * 3 = 6 and **oops", &cx()));
        assert_eq!(t[0].0, "2 * 3 = 6 and **oops");
    }
}
