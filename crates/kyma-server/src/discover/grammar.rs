//! Parses the user's search-bar string into structured clauses.
//!
//! Grammar (mirrors the frontend's `discoverGrammar.ts`):
//!
//! | Token form    | Clause                                |
//! |---------------|----------------------------------------|
//! | `auth`        | `Substring("auth")`                    |
//! | `"foo bar"`   | `Substring("foo bar")`                 |
//! | `field:value` | `Eq("field", "value")`                 |
//! | `-field:value`| `Neq("field", "value")`                |
//! | `field:>N`    | `Cmp("field", Op::Gt, "N")`            |
//! | `field:>=N`   | `Cmp("field", Op::Ge, "N")`            |
//! | `field:<N`    | `Cmp("field", Op::Lt, "N")`            |
//! | `field:<=N`   | `Cmp("field", Op::Le, "N")`            |
//! | `field:*`     | `Exists("field")`                      |
//!
//! Multiple clauses combine with implicit AND.

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Clause {
    Substring {
        value: String,
    },
    Eq {
        field: String,
        value: String,
    },
    Neq {
        field: String,
        value: String,
    },
    Cmp {
        field: String,
        op: CmpOp,
        value: String,
    },
    Exists {
        field: String,
    },
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CmpOp {
    Gt,
    Ge,
    Lt,
    Le,
}

#[derive(Debug, thiserror::Error)]
pub enum GrammarError {
    #[error("unterminated quoted string")]
    UnterminatedQuote,
    #[error("dangling '-' with no following token")]
    DanglingNegation,
    #[error("negation requires a field — substrings cannot be negated")]
    NegatedSubstring,
}

pub fn parse(input: &str) -> Result<Vec<Clause>, GrammarError> {
    let mut out = Vec::new();
    let mut chars = input.chars().peekable();
    while let Some(&c) = chars.peek() {
        if c.is_whitespace() {
            chars.next();
            continue;
        }
        let negated = if c == '-' {
            chars.next();
            true
        } else {
            false
        };
        let token = read_token(&mut chars)?;
        if token.is_empty() {
            if negated {
                return Err(GrammarError::DanglingNegation);
            }
            continue;
        }
        out.push(token_to_clause(token, negated)?);
    }
    Ok(out)
}

fn read_token<I>(chars: &mut std::iter::Peekable<I>) -> Result<String, GrammarError>
where
    I: Iterator<Item = char>,
{
    let mut buf = String::new();
    while let Some(&c) = chars.peek() {
        if c.is_whitespace() {
            break;
        }
        if c == '"' {
            chars.next();
            let mut quoted = String::new();
            let mut closed = false;
            while let Some(c2) = chars.next() {
                if c2 == '"' {
                    closed = true;
                    break;
                }
                quoted.push(c2);
            }
            if !closed {
                return Err(GrammarError::UnterminatedQuote);
            }
            buf.push_str(&quoted);
            continue;
        }
        chars.next();
        buf.push(c);
    }
    Ok(buf)
}

fn token_to_clause(token: String, negated: bool) -> Result<Clause, GrammarError> {
    if let Some((field, value)) = split_field(&token) {
        if value == "*" {
            return Ok(Clause::Exists {
                field: field.to_string(),
            });
        }
        if let Some(rest) = value.strip_prefix(">=") {
            return Ok(Clause::Cmp {
                field: field.to_string(),
                op: CmpOp::Ge,
                value: rest.to_string(),
            });
        }
        if let Some(rest) = value.strip_prefix("<=") {
            return Ok(Clause::Cmp {
                field: field.to_string(),
                op: CmpOp::Le,
                value: rest.to_string(),
            });
        }
        if let Some(rest) = value.strip_prefix('>') {
            return Ok(Clause::Cmp {
                field: field.to_string(),
                op: CmpOp::Gt,
                value: rest.to_string(),
            });
        }
        if let Some(rest) = value.strip_prefix('<') {
            return Ok(Clause::Cmp {
                field: field.to_string(),
                op: CmpOp::Lt,
                value: rest.to_string(),
            });
        }
        if negated {
            Ok(Clause::Neq {
                field: field.to_string(),
                value: value.to_string(),
            })
        } else {
            Ok(Clause::Eq {
                field: field.to_string(),
                value: value.to_string(),
            })
        }
    } else {
        // Substrings can't be negated (matches grammar spec).
        if negated {
            return Err(GrammarError::NegatedSubstring);
        }
        Ok(Clause::Substring { value: token })
    }
}

fn split_field(token: &str) -> Option<(&str, &str)> {
    let idx = token.find(':')?;
    let (f, v) = token.split_at(idx);
    if f.is_empty() {
        return None;
    }
    Some((f, &v[1..]))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &str) -> String {
        v.to_string()
    }

    #[test]
    fn parses_bare_token_as_substring() {
        assert_eq!(
            parse("auth").unwrap(),
            vec![Clause::Substring { value: s("auth") }]
        );
    }

    #[test]
    fn parses_quoted_phrase_as_substring() {
        assert_eq!(
            parse("\"foo bar\"").unwrap(),
            vec![Clause::Substring {
                value: s("foo bar")
            }]
        );
    }

    #[test]
    fn parses_field_value_as_eq() {
        assert_eq!(
            parse("service:payments").unwrap(),
            vec![Clause::Eq {
                field: s("service"),
                value: s("payments")
            }]
        );
    }

    #[test]
    fn parses_negation_as_neq() {
        assert_eq!(
            parse("-severity:INFO").unwrap(),
            vec![Clause::Neq {
                field: s("severity"),
                value: s("INFO")
            }]
        );
    }

    #[test]
    fn parses_numeric_comparisons() {
        assert_eq!(
            parse("status:>500").unwrap(),
            vec![Clause::Cmp {
                field: s("status"),
                op: CmpOp::Gt,
                value: s("500")
            }]
        );
        assert_eq!(
            parse("status:>=500").unwrap(),
            vec![Clause::Cmp {
                field: s("status"),
                op: CmpOp::Ge,
                value: s("500")
            }]
        );
        assert_eq!(
            parse("latency:<10").unwrap(),
            vec![Clause::Cmp {
                field: s("latency"),
                op: CmpOp::Lt,
                value: s("10")
            }]
        );
        assert_eq!(
            parse("latency:<=10").unwrap(),
            vec![Clause::Cmp {
                field: s("latency"),
                op: CmpOp::Le,
                value: s("10")
            }]
        );
    }

    #[test]
    fn parses_exists() {
        assert_eq!(
            parse("trace_id:*").unwrap(),
            vec![Clause::Exists {
                field: s("trace_id")
            }]
        );
    }

    #[test]
    fn parses_quoted_value() {
        assert_eq!(
            parse("message:\"conn refused\"").unwrap(),
            vec![Clause::Eq {
                field: s("message"),
                value: s("conn refused")
            }]
        );
    }

    #[test]
    fn parses_mix() {
        let got = parse("auth service:payments -severity:INFO").unwrap();
        assert_eq!(
            got,
            vec![
                Clause::Substring { value: s("auth") },
                Clause::Eq {
                    field: s("service"),
                    value: s("payments")
                },
                Clause::Neq {
                    field: s("severity"),
                    value: s("INFO")
                },
            ]
        );
    }

    #[test]
    fn empty_input_yields_empty_vec() {
        assert!(parse("").unwrap().is_empty());
        assert!(parse("   ").unwrap().is_empty());
    }

    #[test]
    fn unterminated_quote_is_an_error() {
        assert!(matches!(
            parse("foo:\"bar"),
            Err(GrammarError::UnterminatedQuote)
        ));
    }

    #[test]
    fn dangling_negation_is_an_error() {
        assert!(matches!(parse("-"), Err(GrammarError::DanglingNegation)));
        assert!(matches!(
            parse("- foo"),
            Err(GrammarError::DanglingNegation)
        ));
    }

    #[test]
    fn negated_bare_substring_is_an_error() {
        assert!(matches!(parse("-foo"), Err(GrammarError::NegatedSubstring)));
        assert!(matches!(
            parse("-\"foo bar\""),
            Err(GrammarError::NegatedSubstring)
        ));
    }

    #[test]
    fn multi_colon_takes_first_colon() {
        // `a:b:c` → field "a", value "b:c"
        assert_eq!(
            parse("a:b:c").unwrap(),
            vec![Clause::Eq {
                field: "a".into(),
                value: "b:c".into()
            }]
        );
    }

    #[test]
    fn empty_value_is_eq_to_empty_string() {
        // `field:` → field "field", value "" — explicit equality with empty string.
        assert_eq!(
            parse("field:").unwrap(),
            vec![Clause::Eq {
                field: "field".into(),
                value: "".into()
            }]
        );
    }

    #[test]
    fn cmp_with_empty_value_is_allowed() {
        // `field:>` → Cmp with empty value. Engine will type-check + drop at compile time.
        assert_eq!(
            parse("field:>").unwrap(),
            vec![Clause::Cmp {
                field: "field".into(),
                op: CmpOp::Gt,
                value: "".into()
            }]
        );
    }
}
