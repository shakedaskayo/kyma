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
#[serde(tag = "type")]
pub enum Clause {
    Substring { value: String },
    Eq { field: String, value: String },
    Neq { field: String, value: String },
    Cmp { field: String, op: CmpOp, value: String },
    Exists { field: String },
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
}

pub fn parse(input: &str) -> Result<Vec<Clause>, GrammarError> {
    todo!("Task 2")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_bare_token_as_substring() {
        assert_eq!(
            parse("auth").unwrap(),
            vec![Clause::Substring { value: "auth".to_string() }],
        );
    }
}
