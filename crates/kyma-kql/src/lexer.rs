//! KQL lexer: raw source → token stream.

#[derive(Debug, Clone, PartialEq)]
pub enum Token {
    Ident(String), // identifiers AND keywords; parser decides
    Int(i64),
    Float(f64),
    Str(String),
    Duration(String), // e.g. "5m", "2h" — parser decodes
    Pipe,             // |
    Comma,
    LParen,
    RParen,
    LBracket,
    RBracket,
    Dot,
    DotDot, // ..
    Eq,     // ==
    Ne,     // !=
    Lt,
    Le,
    Gt,
    Ge,
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    Assign, // = (extend binding)
}

#[derive(Debug)]
pub struct LexError(pub String);
impl std::fmt::Display for LexError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "lex error: {}", self.0)
    }
}

pub fn tokenize(src: &str) -> Result<Vec<Token>, LexError> {
    let mut out = Vec::with_capacity(src.len() / 4);
    let mut chars = src.chars().peekable();
    while let Some(&c) = chars.peek() {
        match c {
            c if c.is_whitespace() => {
                chars.next();
            }
            '/' if peek2(&chars, "//") => {
                // line comment
                for ch in chars.by_ref() {
                    if ch == '\n' {
                        break;
                    }
                }
            }
            '|' => {
                chars.next();
                out.push(Token::Pipe);
            }
            ',' => {
                chars.next();
                out.push(Token::Comma);
            }
            '(' => {
                chars.next();
                out.push(Token::LParen);
            }
            ')' => {
                chars.next();
                out.push(Token::RParen);
            }
            '[' => {
                chars.next();
                out.push(Token::LBracket);
            }
            ']' => {
                chars.next();
                out.push(Token::RBracket);
            }
            '.' => {
                chars.next();
                if chars.peek() == Some(&'.') {
                    chars.next();
                    out.push(Token::DotDot);
                } else {
                    out.push(Token::Dot);
                }
            }
            '+' => {
                chars.next();
                out.push(Token::Plus);
            }
            '-' => {
                chars.next();
                out.push(Token::Minus);
            }
            '*' => {
                chars.next();
                out.push(Token::Star);
            }
            '%' => {
                chars.next();
                out.push(Token::Percent);
            }
            '/' => {
                chars.next();
                out.push(Token::Slash);
            }
            '=' => {
                chars.next();
                if chars.peek() == Some(&'=') {
                    chars.next();
                    out.push(Token::Eq);
                } else {
                    out.push(Token::Assign);
                }
            }
            '!' => {
                chars.next();
                if chars.next() != Some('=') {
                    return Err(LexError("expected '=' after '!'".into()));
                }
                out.push(Token::Ne);
            }
            '<' => {
                chars.next();
                if chars.peek() == Some(&'=') {
                    chars.next();
                    out.push(Token::Le);
                } else {
                    out.push(Token::Lt);
                }
            }
            '>' => {
                chars.next();
                if chars.peek() == Some(&'=') {
                    chars.next();
                    out.push(Token::Ge);
                } else {
                    out.push(Token::Gt);
                }
            }
            '"' => {
                chars.next();
                let mut s = String::new();
                loop {
                    let ch = chars
                        .next()
                        .ok_or_else(|| LexError("unterminated double-quoted string".into()))?;
                    if ch == '"' {
                        break;
                    }
                    if ch == '\\' {
                        let next = chars
                            .next()
                            .ok_or_else(|| LexError("dangling backslash".into()))?;
                        match next {
                            'n' => s.push('\n'),
                            't' => s.push('\t'),
                            'r' => s.push('\r'),
                            '\\' => s.push('\\'),
                            '"' => s.push('"'),
                            other => s.push(other),
                        }
                    } else {
                        s.push(ch);
                    }
                }
                out.push(Token::Str(s));
            }
            '\'' => {
                chars.next();
                let mut s = String::new();
                loop {
                    let ch = chars
                        .next()
                        .ok_or_else(|| LexError("unterminated single-quoted string".into()))?;
                    if ch == '\'' {
                        break;
                    }
                    s.push(ch);
                }
                out.push(Token::Str(s));
            }
            c if c.is_ascii_digit() => {
                let mut lit = String::new();
                let mut is_float = false;
                while let Some(&d) = chars.peek() {
                    if d.is_ascii_digit() {
                        lit.push(d);
                        chars.next();
                    } else if d == '.' {
                        let mut tmp = chars.clone();
                        tmp.next();
                        if tmp.peek().map(|cc| cc.is_ascii_digit()).unwrap_or(false) {
                            is_float = true;
                            lit.push('.');
                            chars.next();
                        } else {
                            break;
                        }
                    } else {
                        break;
                    }
                }
                // Duration suffix: s/m/h/d — but only if preceded by digits
                // and immediately followed by non-identifier character.
                if let Some(&suffix) = chars.peek() {
                    if matches!(suffix, 's' | 'm' | 'h' | 'd') {
                        // Look ahead: next char must not be an identifier start.
                        let mut tmp = chars.clone();
                        tmp.next();
                        let ok = match tmp.peek() {
                            None => true,
                            Some(&nc) => !nc.is_ascii_alphanumeric() && nc != '_',
                        };
                        if ok {
                            lit.push(suffix);
                            chars.next();
                            out.push(Token::Duration(lit));
                            continue;
                        }
                    }
                }
                if is_float {
                    out.push(Token::Float(lit.parse().map_err(
                        |e: std::num::ParseFloatError| LexError(format!("bad float: {e}")),
                    )?));
                } else {
                    out.push(Token::Int(lit.parse().map_err(
                        |e: std::num::ParseIntError| LexError(format!("bad int: {e}")),
                    )?));
                }
            }
            c if c.is_ascii_alphabetic() || c == '_' => {
                let mut ident = String::new();
                while let Some(&cc) = chars.peek() {
                    if cc.is_ascii_alphanumeric() || cc == '_' || cc == '-' {
                        // Special handling for `project-away` so the dash is
                        // part of the keyword rather than a minus.
                        // We include '-' only when forming identifier-like
                        // keywords mid-word.
                        if cc == '-' {
                            // Peek one more: must be alpha to continue
                            // ident (so `status - 200` isn't mis-lexed).
                            let mut tmp = chars.clone();
                            tmp.next();
                            if tmp
                                .peek()
                                .map(|nc| nc.is_ascii_alphabetic())
                                .unwrap_or(false)
                            {
                                ident.push('-');
                                chars.next();
                                continue;
                            } else {
                                break;
                            }
                        }
                        ident.push(cc);
                        chars.next();
                    } else {
                        break;
                    }
                }
                out.push(Token::Ident(ident));
            }
            other => {
                return Err(LexError(format!("unexpected character: {other:?}")));
            }
        }
    }
    Ok(out)
}

fn peek2(chars: &std::iter::Peekable<std::str::Chars<'_>>, tag: &str) -> bool {
    let mut it = chars.clone();
    for t in tag.chars() {
        if it.next() != Some(t) {
            return false;
        }
    }
    true
}
