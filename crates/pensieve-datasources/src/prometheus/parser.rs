//! OpenMetrics / Prometheus text-exposition-format parser.
//!
//! Intentionally handwritten and small: the grammar is simple and we avoid
//! pulling a dependency. Returns one `Sample` per data line; `HELP`/`TYPE`/
//! `UNIT` lines are consumed but not retained.

use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq)]
pub struct Sample {
    pub name: String,
    pub labels: BTreeMap<String, String>,
    /// `None` for `NaN` / `+Inf` / `-Inf` (emitted as JSON null downstream).
    pub value: Option<f64>,
}

#[derive(Debug, thiserror::Error)]
#[error("parse error at line {line}: {msg}")]
pub struct ParseError {
    pub line: usize,
    pub msg: String,
}

pub fn parse_openmetrics(text: &str) -> Result<Vec<Sample>, ParseError> {
    let mut out = Vec::new();
    for (idx, raw) in text.lines().enumerate() {
        let line_no = idx + 1;
        let s = raw.trim();
        if s.is_empty() || s.starts_with('#') {
            continue;
        }
        out.push(parse_sample_line(s, line_no)?);
    }
    Ok(out)
}

fn parse_sample_line(s: &str, line: usize) -> Result<Sample, ParseError> {
    // grammar:  NAME[{label="value",...}]  SPACE  FLOAT  [SPACE TIMESTAMP]
    //
    // We ignore any trailing per-sample timestamp — scrape time wins.

    let (head, value_str) = split_last_whitespace(s).ok_or_else(|| ParseError {
        line,
        msg: format!("no value: {s:?}"),
    })?;

    let (name, labels_part) = match head.find('{') {
        Some(open) => {
            let close = head.rfind('}').ok_or_else(|| ParseError {
                line,
                msg: "unbalanced {".into(),
            })?;
            if close <= open {
                return Err(ParseError {
                    line,
                    msg: "unbalanced {}".into(),
                });
            }
            (&head[..open], Some(&head[open + 1..close]))
        }
        None => (head, None),
    };

    if name.is_empty() || !is_valid_name(name) {
        return Err(ParseError {
            line,
            msg: format!("bad metric name {name:?}"),
        });
    }

    let labels = match labels_part {
        None => BTreeMap::new(),
        Some(body) => parse_labels(body, line)?,
    };

    let value = parse_float(value_str);

    Ok(Sample {
        name: name.to_string(),
        labels,
        value,
    })
}

fn split_last_whitespace(s: &str) -> Option<(&str, &str)> {
    // Find where the head ends (either the `}` or the first whitespace gap).
    let head_end = if let Some(close) = s.rfind('}') {
        close + 1
    } else {
        // first whitespace position separates name from value tokens
        let bytes = s.as_bytes();
        let mut first = None;
        for (i, &b) in bytes.iter().enumerate() {
            if b == b' ' || b == b'\t' {
                first = Some(i);
                break;
            }
        }
        first?
    };
    let head = &s[..head_end];
    // The value token is the FIRST whitespace-delimited token after the head.
    // Per OpenMetrics spec we ignore any trailing per-sample timestamp.
    let value = s[head_end..].split_ascii_whitespace().next()?;
    Some((head, value))
}

fn is_valid_name(s: &str) -> bool {
    let mut iter = s.chars();
    let Some(first) = iter.next() else {
        return false;
    };
    if !(first.is_ascii_alphabetic() || first == '_' || first == ':') {
        return false;
    }
    iter.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == ':')
}

fn parse_labels(body: &str, line: usize) -> Result<BTreeMap<String, String>, ParseError> {
    let mut out = BTreeMap::new();
    let mut bytes = body.as_bytes();
    loop {
        bytes = trim_start(bytes);
        if bytes.is_empty() {
            break;
        }
        let name_end = bytes
            .iter()
            .position(|&b| b == b'=')
            .ok_or_else(|| ParseError {
                line,
                msg: "label missing =".into(),
            })?;
        let label_name = std::str::from_utf8(&bytes[..name_end])
            .map_err(|_| ParseError {
                line,
                msg: "non-utf8 label".into(),
            })?
            .trim();
        bytes = &bytes[name_end + 1..];
        if bytes.first() != Some(&b'"') {
            return Err(ParseError {
                line,
                msg: "label value missing quote".into(),
            });
        }
        bytes = &bytes[1..];
        let mut val = String::new();
        loop {
            // Find next special byte.
            let mut i = 0;
            while i < bytes.len() && bytes[i] != b'"' && bytes[i] != b'\\' {
                i += 1;
            }
            if i > 0 {
                // Safe: we only broke on single-byte ASCII boundaries; bytes[..i] is
                // valid UTF-8 because the input was &str.
                let chunk = std::str::from_utf8(&bytes[..i]).map_err(|_| ParseError {
                    line,
                    msg: "non-utf8 label value".into(),
                })?;
                val.push_str(chunk);
                bytes = &bytes[i..];
            }
            match bytes.first().copied() {
                None => {
                    return Err(ParseError {
                        line,
                        msg: "unterminated label".into(),
                    })
                }
                Some(b'"') => {
                    bytes = &bytes[1..];
                    break;
                }
                Some(b'\\') => match bytes.get(1).copied() {
                    Some(b'"') => {
                        val.push('"');
                        bytes = &bytes[2..];
                    }
                    Some(b'\\') => {
                        val.push('\\');
                        bytes = &bytes[2..];
                    }
                    Some(b'n') => {
                        val.push('\n');
                        bytes = &bytes[2..];
                    }
                    Some(c) if c < 0x80 => {
                        val.push(c as char);
                        bytes = &bytes[2..];
                    }
                    Some(_) => {
                        return Err(ParseError {
                            line,
                            msg: "invalid escape".into(),
                        })
                    }
                    None => {
                        return Err(ParseError {
                            line,
                            msg: "dangling escape".into(),
                        })
                    }
                },
                Some(_) => unreachable!("scanner only stops on \" or \\ or end"),
            }
        }
        out.insert(label_name.to_string(), val);
        bytes = trim_start(bytes);
        match bytes.first() {
            Some(&b',') => {
                bytes = &bytes[1..];
            }
            Some(_) | None => break,
        }
    }
    Ok(out)
}

fn trim_start(mut b: &[u8]) -> &[u8] {
    while let Some(&c) = b.first() {
        if c == b' ' || c == b'\t' {
            b = &b[1..];
        } else {
            break;
        }
    }
    b
}

fn parse_float(s: &str) -> Option<f64> {
    match s {
        "NaN" | "nan" | "+Inf" | "-Inf" | "inf" | "+inf" | "-inf" => None,
        _ => s
            .parse::<f64>()
            .ok()
            .and_then(|v| if v.is_finite() { Some(v) } else { None }),
    }
}
