//! Local-only LLM-judge labeling tool (scaffold).
//!
//! ███ NEVER RUN IN CI. ███
//!
//! CI must stay fully deterministic; this binary exists so a developer can
//! mint golden fixtures locally, freeze them, and commit the JSON. The only
//! deterministic mode is `--dry-run` (keyword-overlap grading), which is
//! what the unit tests exercise — and even that is a placeholder, not a
//! quality signal.
//!
//! Input: JSONL of candidates, one per line:
//!   `{"query": str, "candidate_key": str, "candidate_text": str}`
//! Optionally `{"mode": "data"|"memory"|"graph"}` per line (default "data").
//!
//! Output: one golden fixture JSON per distinct query, written to
//! `--out-dir` (default `eval/golden/`), in the `golden::GoldenFixture`
//! format. Review + spot-check before committing.
//!
//! Calibration: pass `--human-labels=PATH` (JSON map of
//! `"query\u{1}candidate_key" → grade` or a JSONL of
//! `{"query","candidate_key","grade"}`) to print exact-match rate and mean
//! absolute error of the judge against human grades on the overlap.
//!
//! LLM judge: raw HTTP to the Anthropic Messages API (`POST /v1/messages`)
//! — `ANTHROPIC_API_KEY` required, model from `KYMA_JUDGE_MODEL`
//! (default `claude-opus-4-8`). The HTTP call sits behind the [`Judge`]
//! trait so tests never touch the network.

use anyhow::{bail, Context, Result};
use clap::Parser;
use kyma_retrieval_eval::golden::{GoldenFixture, Judged};
use serde::Deserialize;
use std::collections::BTreeMap;
use std::collections::HashSet;
use std::io::BufRead;
use std::path::PathBuf;

#[derive(Debug, Parser)]
#[command(
    name = "judge_calibrate",
    about = "LOCAL-ONLY LLM-judge labeling for golden retrieval fixtures. Never run in CI.",
    after_help = "CI safety: this tool calls an external LLM endpoint unless --dry-run is set.\n\
                  Fixtures it writes must be human spot-checked and frozen before committing."
)]
struct Args {
    /// Input JSONL: `{"query","candidate_key","candidate_text"[,"mode"]}` per line.
    #[arg(long)]
    input: PathBuf,

    /// Directory to write one fixture JSON per query into.
    #[arg(long, default_value = "eval/golden")]
    out_dir: PathBuf,

    /// Deterministic offline mode: grade by keyword overlap instead of an
    /// LLM call. For wiring tests only — not a quality signal.
    #[arg(long, default_value_t = false)]
    dry_run: bool,

    /// Optional human-labeled sample for calibration stats
    /// (JSONL of `{"query","candidate_key","grade"}`).
    #[arg(long)]
    human_labels: Option<PathBuf>,

    /// Judge model id (LLM mode only).
    #[arg(long, env = "KYMA_JUDGE_MODEL", default_value = "claude-opus-4-8")]
    model: String,

    /// Anthropic-compatible Messages endpoint (LLM mode only).
    #[arg(
        long,
        env = "KYMA_JUDGE_ENDPOINT",
        default_value = "https://api.anthropic.com/v1/messages"
    )]
    endpoint: String,
}

#[derive(Debug, Deserialize)]
#[allow(clippy::struct_field_names)] // field names are the JSONL wire format
struct Candidate {
    query: String,
    candidate_key: String,
    candidate_text: String,
    #[serde(default = "default_mode")]
    mode: String,
}

fn default_mode() -> String {
    "data".to_string()
}

#[derive(Debug, Deserialize)]
struct HumanLabel {
    query: String,
    candidate_key: String,
    grade: u8,
}

/// Grading backend. Implementations return a grade in 0..=3.
trait Judge {
    fn grade(&self, query: &str, candidate_text: &str) -> Result<u8>;
}

/// Deterministic offline judge: fraction of query keywords (lowercased,
/// len > 2) present in the candidate text, bucketed to 0..=3. A placeholder
/// that makes the plumbing testable without network — NOT a real judge.
struct KeywordOverlapJudge;

impl Judge for KeywordOverlapJudge {
    fn grade(&self, query: &str, candidate_text: &str) -> Result<u8> {
        let text = candidate_text.to_lowercase();
        let words: Vec<String> = query
            .to_lowercase()
            .split(|c: char| !c.is_alphanumeric())
            .filter(|w| w.len() > 2)
            .map(str::to_string)
            .collect();
        if words.is_empty() {
            return Ok(0);
        }
        let hits = words.iter().filter(|w| text.contains(w.as_str())).count();
        #[allow(clippy::cast_precision_loss)]
        let overlap = hits as f64 / words.len() as f64;
        Ok(match overlap {
            o if o >= 0.75 => 3,
            o if o >= 0.5 => 2,
            o if o >= 0.25 => 1,
            _ => 0,
        })
    }
}

/// LLM judge over the Anthropic Messages API (raw HTTP via reqwest —
/// there is no official Rust SDK). Requires `ANTHROPIC_API_KEY`.
struct AnthropicJudge {
    endpoint: String,
    model: String,
    api_key: String,
    http: reqwest::blocking::Client,
}

impl AnthropicJudge {
    fn from_env(endpoint: &str, model: &str) -> Result<Self> {
        let api_key = std::env::var("ANTHROPIC_API_KEY")
            .context("ANTHROPIC_API_KEY is required (or use --dry-run)")?;
        Ok(Self {
            endpoint: endpoint.to_string(),
            model: model.to_string(),
            api_key,
            http: reqwest::blocking::Client::builder()
                .timeout(std::time::Duration::from_mins(2))
                .build()?,
        })
    }
}

const JUDGE_PROMPT: &str = "You are grading search-result relevance for a retrieval system.\n\
Grade how well the CANDIDATE answers the QUERY on this scale:\n\
3 = perfect/direct answer, 2 = highly relevant, 1 = marginally related, 0 = irrelevant.\n\
Respond with ONLY the single digit.";

impl Judge for AnthropicJudge {
    fn grade(&self, query: &str, candidate_text: &str) -> Result<u8> {
        let body = serde_json::json!({
            "model": self.model,
            "max_tokens": 16,
            "system": JUDGE_PROMPT,
            "messages": [{
                "role": "user",
                "content": format!("QUERY:\n{query}\n\nCANDIDATE:\n{candidate_text}"),
            }],
        });
        let resp = self
            .http
            .post(&self.endpoint)
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .json(&body)
            .send()
            .context("calling judge endpoint")?;
        let status = resp.status();
        let value: serde_json::Value = resp.json().context("parsing judge response")?;
        if !status.is_success() {
            bail!("judge endpoint HTTP {status}: {value}");
        }
        if value.get("stop_reason").and_then(serde_json::Value::as_str) == Some("refusal") {
            bail!("judge refused the request");
        }
        let text = value["content"]
            .as_array()
            .and_then(|blocks| {
                blocks
                    .iter()
                    .find(|b| b["type"] == "text")
                    .and_then(|b| b["text"].as_str())
            })
            .with_context(|| format!("no text block in judge response: {value}"))?;
        parse_grade(text)
    }
}

/// Extract a 0..=3 grade from judge output (first digit wins; tolerates
/// whitespace/prose around it).
fn parse_grade(text: &str) -> Result<u8> {
    for c in text.chars() {
        if let Some(d) = c.to_digit(10) {
            if d <= 3 {
                #[allow(clippy::cast_possible_truncation)]
                return Ok(d as u8);
            }
            bail!("judge returned out-of-range grade {d} in {text:?}");
        }
    }
    bail!("no grade digit in judge output {text:?}")
}

fn read_candidates(path: &PathBuf) -> Result<Vec<Candidate>> {
    let file = std::fs::File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let mut out = Vec::new();
    for (lineno, line) in std::io::BufReader::new(file).lines().enumerate() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let c: Candidate = serde_json::from_str(&line)
            .with_context(|| format!("{}:{}: bad candidate", path.display(), lineno + 1))?;
        out.push(c);
    }
    Ok(out)
}

fn read_human_labels(path: &PathBuf) -> Result<BTreeMap<(String, String), u8>> {
    let file = std::fs::File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let mut out = BTreeMap::new();
    for (lineno, line) in std::io::BufReader::new(file).lines().enumerate() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let l: HumanLabel = serde_json::from_str(&line)
            .with_context(|| format!("{}:{}: bad human label", path.display(), lineno + 1))?;
        out.insert((l.query, l.candidate_key), l.grade);
    }
    Ok(out)
}

/// File-safe slug for a fixture name derived from the query.
fn slug(query: &str) -> String {
    let s: String = query
        .to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect();
    let mut compact = String::with_capacity(s.len());
    for part in s.split('-').filter(|p| !p.is_empty()) {
        if !compact.is_empty() {
            compact.push('-');
        }
        compact.push_str(part);
    }
    compact.truncate(60);
    if compact.is_empty() {
        "query".to_string()
    } else {
        compact
    }
}

/// Run the labeling pass: judge every candidate, group fixtures per query.
fn label(candidates: &[Candidate], judge: &dyn Judge) -> Result<Vec<GoldenFixture>> {
    // Preserve first-seen query order.
    let mut order: Vec<String> = Vec::new();
    let mut by_query: BTreeMap<String, GoldenFixture> = BTreeMap::new();
    let mut seen: HashSet<(String, String)> = HashSet::new();
    for c in candidates {
        if !seen.insert((c.query.clone(), c.candidate_key.clone())) {
            continue; // duplicate (query, key) — keep the first judgment
        }
        let grade = judge.grade(&c.query, &c.candidate_text)?;
        let entry = by_query.entry(c.query.clone()).or_insert_with(|| {
            order.push(c.query.clone());
            GoldenFixture {
                query: c.query.clone(),
                mode: c.mode.clone(),
                judged: Vec::new(),
                notes: "Labeled by judge_calibrate; spot-check before freezing. NEVER re-judge in CI.".to_string(),
            }
        });
        entry.judged.push(Judged { key: c.candidate_key.clone(), grade });
    }
    Ok(order
        .into_iter()
        .filter_map(|q| by_query.remove(&q))
        .collect())
}

struct Calibration {
    overlap: usize,
    exact_match: f64,
    mae: f64,
}

fn calibrate(
    fixtures: &[GoldenFixture],
    human: &BTreeMap<(String, String), u8>,
) -> Calibration {
    let mut n = 0usize;
    let mut exact = 0usize;
    let mut abs_err = 0.0f64;
    for f in fixtures {
        for j in &f.judged {
            if let Some(h) = human.get(&(f.query.clone(), j.key.clone())) {
                n += 1;
                if *h == j.grade {
                    exact += 1;
                }
                abs_err += (f64::from(j.grade) - f64::from(*h)).abs();
            }
        }
    }
    #[allow(clippy::cast_precision_loss)]
    Calibration {
        overlap: n,
        exact_match: if n == 0 { 0.0 } else { exact as f64 / n as f64 },
        mae: if n == 0 { 0.0 } else { abs_err / n as f64 },
    }
}

fn main() -> Result<()> {
    let args = Args::parse();
    eprintln!("[judge] LOCAL-ONLY tool — never run in CI (dry_run={})", args.dry_run);

    let candidates = read_candidates(&args.input)?;
    if candidates.is_empty() {
        bail!("no candidates in {}", args.input.display());
    }

    let fixtures = if args.dry_run {
        label(&candidates, &KeywordOverlapJudge)?
    } else {
        let judge = AnthropicJudge::from_env(&args.endpoint, &args.model)?;
        eprintln!("[judge] grading {} candidates with {}", candidates.len(), args.model);
        label(&candidates, &judge)?
    };

    std::fs::create_dir_all(&args.out_dir)
        .with_context(|| format!("creating {}", args.out_dir.display()))?;
    for f in &fixtures {
        let path = args.out_dir.join(format!("{}.json", slug(&f.query)));
        std::fs::write(&path, format!("{}\n", serde_json::to_string_pretty(f)?))
            .with_context(|| format!("writing {}", path.display()))?;
        eprintln!("[judge] wrote {} ({} judgments)", path.display(), f.judged.len());
    }

    if let Some(human_path) = &args.human_labels {
        let human = read_human_labels(human_path)?;
        let cal = calibrate(&fixtures, &human);
        println!(
            "calibration: overlap={} exact_match={:.3} mae={:.3}",
            cal.overlap, cal.exact_match, cal.mae
        );
        if cal.overlap == 0 {
            eprintln!("[judge] WARNING: no overlap between judge output and human labels");
        }
    }

    Ok(())
}

#[cfg(test)]
#[allow(clippy::float_cmp)]
mod tests {
    use super::*;

    #[test]
    fn args_parse_dry_run() {
        let args = Args::try_parse_from([
            "judge_calibrate",
            "--input=/tmp/cands.jsonl",
            "--dry-run",
            "--out-dir=/tmp/out",
        ])
        .unwrap();
        assert!(args.dry_run);
        assert_eq!(args.model, "claude-opus-4-8");
        assert_eq!(args.out_dir.to_str().unwrap(), "/tmp/out");
    }

    #[test]
    fn args_require_input() {
        assert!(Args::try_parse_from(["judge_calibrate"]).is_err());
    }

    #[test]
    fn keyword_judge_is_deterministic_and_bucketed() {
        let j = KeywordOverlapJudge;
        assert_eq!(j.grade("rotate credentials", "how to rotate credentials safely").unwrap(), 3);
        assert_eq!(j.grade("rotate credentials", "rotate the logs daily").unwrap(), 2);
        assert_eq!(j.grade("rotate credentials now please", "credentials").unwrap(), 1);
        assert_eq!(j.grade("rotate credentials", "unrelated text").unwrap(), 0);
        // Same input → same output.
        assert_eq!(
            j.grade("a query here", "a candidate here").unwrap(),
            j.grade("a query here", "a candidate here").unwrap()
        );
    }

    #[test]
    fn parse_grade_variants() {
        assert_eq!(parse_grade("3").unwrap(), 3);
        assert_eq!(parse_grade("  2\n").unwrap(), 2);
        assert_eq!(parse_grade("Grade: 1").unwrap(), 1);
        assert!(parse_grade("7").is_err());
        assert!(parse_grade("no digits").is_err());
    }

    #[test]
    fn label_groups_by_query_and_dedups() {
        let candidates = vec![
            Candidate { query: "q1".into(), candidate_key: "a".into(), candidate_text: "q1".into(), mode: "memory".into() },
            Candidate { query: "q2".into(), candidate_key: "b".into(), candidate_text: "zzz".into(), mode: "data".into() },
            Candidate { query: "q1".into(), candidate_key: "c".into(), candidate_text: "zzz".into(), mode: "memory".into() },
            // duplicate (q1, a) — must be ignored
            Candidate { query: "q1".into(), candidate_key: "a".into(), candidate_text: "different".into(), mode: "memory".into() },
        ];
        let fixtures = label(&candidates, &KeywordOverlapJudge).unwrap();
        assert_eq!(fixtures.len(), 2);
        assert_eq!(fixtures[0].query, "q1");
        assert_eq!(fixtures[0].mode, "memory");
        assert_eq!(fixtures[0].judged.len(), 2);
        assert_eq!(fixtures[1].query, "q2");
    }

    #[test]
    fn end_to_end_dry_run_writes_fixture_files() {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("cands.jsonl");
        std::fs::write(
            &input,
            concat!(
                r#"{"query":"rotate credentials","candidate_key":"mem:1","candidate_text":"rotate credentials guide"}"#, "\n",
                r#"{"query":"rotate credentials","candidate_key":"mem:2","candidate_text":"unrelated"}"#, "\n",
            ),
        )
        .unwrap();
        let candidates = read_candidates(&input).unwrap();
        let fixtures = label(&candidates, &KeywordOverlapJudge).unwrap();
        let out = dir.path().join(format!("{}.json", slug(&fixtures[0].query)));
        std::fs::write(&out, serde_json::to_string_pretty(&fixtures[0]).unwrap()).unwrap();
        // Round-trip through the golden loader.
        let loaded = kyma_retrieval_eval::golden::load_fixtures(dir.path()).unwrap();
        // cands.jsonl isn't .json, so only the fixture is picked up.
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].1.judged.len(), 2);
        assert_eq!(loaded[0].1.judged[0].grade, 3);
        assert_eq!(loaded[0].1.judged[1].grade, 0);
    }

    #[test]
    fn calibration_stats() {
        let fixtures = vec![GoldenFixture {
            query: "q".into(),
            mode: "data".into(),
            judged: vec![
                Judged { key: "a".into(), grade: 3 },
                Judged { key: "b".into(), grade: 1 },
                Judged { key: "c".into(), grade: 2 }, // no human label
            ],
            notes: String::new(),
        }];
        let mut human = BTreeMap::new();
        human.insert(("q".to_string(), "a".to_string()), 3u8);
        human.insert(("q".to_string(), "b".to_string()), 3u8);
        let cal = calibrate(&fixtures, &human);
        assert_eq!(cal.overlap, 2);
        assert_eq!(cal.exact_match, 0.5);
        assert_eq!(cal.mae, 1.0); // |3-3|=0, |1-3|=2 → mean 1.0
    }

    #[test]
    fn slug_is_filesystem_safe() {
        assert_eq!(slug("How do I rotate credentials?"), "how-do-i-rotate-credentials");
        assert_eq!(slug("***"), "query");
    }
}
