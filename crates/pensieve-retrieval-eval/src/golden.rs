//! Golden-set fixtures + regression runner.
//!
//! Fixtures live in `eval/golden/*.json` (see that directory's README for
//! how queries are mined and judged). Each fixture is one query with graded
//! relevance judgments. The runner takes ranked result keys produced by any
//! engine call, computes NDCG@10 (exponential gain) + MRR per query and in
//! aggregate, and compares against the committed `eval/golden/baseline.json`
//! within a tolerance — exit nonzero on regression is the caller's job
//! (see [`check_regression`]).

use crate::metrics::{mean, ndcg_at_k, reciprocal_rank, Gain};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};
use std::path::Path;

/// NDCG cutoff used throughout the golden runner.
pub const NDCG_K: usize = 10;

/// One judged result for a query.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Judged {
    /// Stable result key (e.g. memory id, `db/table/row` key, graph node id).
    pub key: String,
    /// Graded relevance: 0 = irrelevant … 3 = perfect.
    pub grade: u8,
}

/// One golden fixture: a query plus its judgments.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GoldenFixture {
    pub query: String,
    /// Search surface this query targets: `"data"`, `"memory"`, or `"graph"`.
    pub mode: String,
    pub judged: Vec<Judged>,
    #[serde(default)]
    pub notes: String,
}

/// Per-query evaluation result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryEval {
    pub query: String,
    pub mode: String,
    pub ndcg_at_10: f64,
    pub mrr: f64,
    /// Number of returned results that had a judgment (coverage signal).
    pub judged_returned: usize,
}

/// Aggregate report over all fixtures.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GoldenReport {
    pub per_query: Vec<QueryEval>,
    pub mean_ndcg_at_10: f64,
    pub mean_mrr: f64,
}

/// Committed baseline (`eval/golden/baseline.json`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GoldenBaseline {
    pub mean_ndcg_at_10: f64,
    pub mean_mrr: f64,
    /// Allowed absolute drop below baseline before a run counts as a
    /// regression. Improvements never fail.
    pub tolerance: f64,
    #[serde(default)]
    pub notes: String,
}

/// Load all fixtures from a directory: every `*.json` except
/// `baseline.json`, sorted by file name for deterministic ordering.
/// Returns `(file_stem, fixture)` pairs.
pub fn load_fixtures(dir: &Path) -> Result<Vec<(String, GoldenFixture)>> {
    let mut entries: Vec<_> = std::fs::read_dir(dir)
        .with_context(|| format!("reading golden dir {}", dir.display()))?
        .filter_map(std::result::Result::ok)
        .map(|e| e.path())
        .filter(|p| {
            p.extension().is_some_and(|e| e == "json")
                && p.file_name().is_some_and(|n| n != "baseline.json")
        })
        .collect();
    entries.sort();
    let mut out = Vec::with_capacity(entries.len());
    for path in entries {
        let raw = std::fs::read_to_string(&path)
            .with_context(|| format!("reading {}", path.display()))?;
        let fixture: GoldenFixture = serde_json::from_str(&raw)
            .with_context(|| format!("parsing {}", path.display()))?;
        let stem = path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        out.push((stem, fixture));
    }
    Ok(out)
}

/// Load the committed baseline file.
pub fn load_baseline(path: &Path) -> Result<GoldenBaseline> {
    let raw = std::fs::read_to_string(path)
        .with_context(|| format!("reading baseline {}", path.display()))?;
    serde_json::from_str(&raw).with_context(|| format!("parsing baseline {}", path.display()))
}

/// Evaluate engine results against the fixtures.
///
/// `results` maps each fixture's `query` string to the ranked result keys
/// the engine returned (best first). Queries missing from `results` score 0
/// — a silently-empty engine response is a regression, not a skip.
pub fn evaluate(
    fixtures: &[(String, GoldenFixture)],
    results: &BTreeMap<String, Vec<String>>,
) -> GoldenReport {
    static EMPTY: Vec<String> = Vec::new();
    let mut per_query = Vec::with_capacity(fixtures.len());
    for (_, fixture) in fixtures {
        let ranked = results.get(&fixture.query).unwrap_or(&EMPTY);
        let grade_of = |key: &String| -> f64 {
            fixture
                .judged
                .iter()
                .find(|j| &j.key == key)
                .map_or(0.0, |j| f64::from(j.grade))
        };
        let ranked_grades: Vec<f64> = ranked.iter().map(grade_of).collect();
        let all_grades: Vec<f64> = fixture.judged.iter().map(|j| f64::from(j.grade)).collect();
        let relevant: HashSet<String> = fixture
            .judged
            .iter()
            .filter(|j| j.grade > 0)
            .map(|j| j.key.clone())
            .collect();
        let judged_returned = ranked
            .iter()
            .filter(|k| fixture.judged.iter().any(|j| &j.key == *k))
            .count();
        per_query.push(QueryEval {
            query: fixture.query.clone(),
            mode: fixture.mode.clone(),
            ndcg_at_10: ndcg_at_k(&ranked_grades, &all_grades, NDCG_K, Gain::Exponential),
            mrr: reciprocal_rank(ranked, &relevant),
            judged_returned,
        });
    }
    let ndcgs: Vec<f64> = per_query.iter().map(|q| q.ndcg_at_10).collect();
    let mrrs: Vec<f64> = per_query.iter().map(|q| q.mrr).collect();
    GoldenReport {
        per_query,
        mean_ndcg_at_10: mean(&ndcgs),
        mean_mrr: mean(&mrrs),
    }
}

/// Compare a report against the baseline. Returns the list of violations
/// (empty = pass). Callers exit nonzero when non-empty.
pub fn check_regression(current: &GoldenReport, baseline: &GoldenBaseline) -> Vec<String> {
    let mut violations = Vec::new();
    let floor_ndcg = baseline.mean_ndcg_at_10 - baseline.tolerance;
    if current.mean_ndcg_at_10 < floor_ndcg {
        violations.push(format!(
            "mean NDCG@10 regressed: {:.4} < baseline {:.4} - tolerance {:.4}",
            current.mean_ndcg_at_10, baseline.mean_ndcg_at_10, baseline.tolerance
        ));
    }
    let floor_mrr = baseline.mean_mrr - baseline.tolerance;
    if current.mean_mrr < floor_mrr {
        violations.push(format!(
            "mean MRR regressed: {:.4} < baseline {:.4} - tolerance {:.4}",
            current.mean_mrr, baseline.mean_mrr, baseline.tolerance
        ));
    }
    violations
}

#[cfg(test)]
#[allow(clippy::float_cmp)]
mod tests {
    use super::*;

    fn fixture(query: &str, judged: &[(&str, u8)]) -> GoldenFixture {
        GoldenFixture {
            query: query.to_string(),
            mode: "data".to_string(),
            judged: judged
                .iter()
                .map(|(k, g)| Judged { key: (*k).to_string(), grade: *g })
                .collect(),
            notes: String::new(),
        }
    }

    fn results(pairs: &[(&str, &[&str])]) -> BTreeMap<String, Vec<String>> {
        pairs
            .iter()
            .map(|(q, keys)| {
                ((*q).to_string(), keys.iter().map(|k| (*k).to_string()).collect())
            })
            .collect()
    }

    #[test]
    fn perfect_ranking_scores_one() {
        let fixtures = vec![("f1".into(), fixture("q1", &[("a", 3), ("b", 1), ("c", 0)]))];
        let report = evaluate(&fixtures, &results(&[("q1", &["a", "b", "c"])]));
        assert_eq!(report.per_query.len(), 1);
        assert!((report.per_query[0].ndcg_at_10 - 1.0).abs() < 1e-12);
        assert_eq!(report.per_query[0].mrr, 1.0);
        assert_eq!(report.per_query[0].judged_returned, 3);
        assert!((report.mean_ndcg_at_10 - 1.0).abs() < 1e-12);
    }

    #[test]
    fn inverted_ranking_scores_below_one() {
        let fixtures = vec![("f1".into(), fixture("q1", &[("a", 3), ("b", 0)]))];
        let report = evaluate(&fixtures, &results(&[("q1", &["b", "a"])]));
        let q = &report.per_query[0];
        // DCG = 0 + 7/log2(3); IDCG = 7 → NDCG = 1/log2(3)
        assert!((q.ndcg_at_10 - 1.0 / 3.0f64.log2()).abs() < 1e-12);
        assert_eq!(q.mrr, 0.5);
    }

    #[test]
    fn missing_query_scores_zero() {
        let fixtures = vec![("f1".into(), fixture("q1", &[("a", 3)]))];
        let report = evaluate(&fixtures, &BTreeMap::new());
        assert_eq!(report.per_query[0].ndcg_at_10, 0.0);
        assert_eq!(report.per_query[0].mrr, 0.0);
        assert_eq!(report.mean_mrr, 0.0);
    }

    #[test]
    fn unjudged_results_count_as_grade_zero() {
        let fixtures = vec![("f1".into(), fixture("q1", &[("a", 2)]))];
        let report = evaluate(&fixtures, &results(&[("q1", &["zzz", "a"])]));
        let q = &report.per_query[0];
        // ranked grades [0, 2]: DCG = 3/log2(3); IDCG = 3.
        assert!((q.ndcg_at_10 - 1.0 / 3.0f64.log2()).abs() < 1e-12);
        assert_eq!(q.mrr, 0.5);
        assert_eq!(q.judged_returned, 1);
    }

    #[test]
    fn aggregates_are_means() {
        let fixtures = vec![
            ("f1".into(), fixture("q1", &[("a", 3)])),
            ("f2".into(), fixture("q2", &[("b", 3)])),
        ];
        // q1 perfect, q2 missing → means are the average of 1.0 and 0.0.
        let report = evaluate(&fixtures, &results(&[("q1", &["a"])]));
        assert_eq!(report.mean_ndcg_at_10, 0.5);
        assert_eq!(report.mean_mrr, 0.5);
    }

    #[test]
    fn regression_check_passes_within_tolerance() {
        let baseline = GoldenBaseline {
            mean_ndcg_at_10: 0.9,
            mean_mrr: 0.8,
            tolerance: 0.05,
            notes: String::new(),
        };
        let report = GoldenReport {
            per_query: vec![],
            mean_ndcg_at_10: 0.86,
            mean_mrr: 0.78,
        };
        assert!(check_regression(&report, &baseline).is_empty());
    }

    #[test]
    fn regression_check_fails_below_tolerance() {
        let baseline = GoldenBaseline {
            mean_ndcg_at_10: 0.9,
            mean_mrr: 0.8,
            tolerance: 0.02,
            notes: String::new(),
        };
        let report = GoldenReport {
            per_query: vec![],
            mean_ndcg_at_10: 0.85,
            mean_mrr: 0.5,
        };
        let violations = check_regression(&report, &baseline);
        assert_eq!(violations.len(), 2);
        assert!(violations[0].contains("NDCG@10"));
        assert!(violations[1].contains("MRR"));
    }

    #[test]
    fn improvement_never_fails() {
        let baseline = GoldenBaseline {
            mean_ndcg_at_10: 0.5,
            mean_mrr: 0.5,
            tolerance: 0.0,
            notes: String::new(),
        };
        let report = GoldenReport {
            per_query: vec![],
            mean_ndcg_at_10: 0.9,
            mean_mrr: 0.9,
        };
        assert!(check_regression(&report, &baseline).is_empty());
    }

    #[test]
    fn fixture_roundtrip_and_dir_loading() {
        let dir = tempfile::tempdir().unwrap();
        let f = fixture("how do I rotate credentials", &[("mem:1", 3), ("mem:2", 1)]);
        std::fs::write(
            dir.path().join("example-creds.json"),
            serde_json::to_string_pretty(&f).unwrap(),
        )
        .unwrap();
        // baseline.json must be skipped by the loader.
        std::fs::write(
            dir.path().join("baseline.json"),
            r#"{"mean_ndcg_at_10":0.9,"mean_mrr":0.9,"tolerance":0.02}"#,
        )
        .unwrap();
        let fixtures = load_fixtures(dir.path()).unwrap();
        assert_eq!(fixtures.len(), 1);
        assert_eq!(fixtures[0].0, "example-creds");
        assert_eq!(fixtures[0].1.judged.len(), 2);
        let baseline = load_baseline(&dir.path().join("baseline.json")).unwrap();
        assert_eq!(baseline.tolerance, 0.02);
    }

    /// The committed example fixtures + baseline in eval/golden/ must parse
    /// and wire through the runner end-to-end.
    #[test]
    fn committed_example_fixtures_load_and_run() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../eval/golden");
        let fixtures = load_fixtures(&dir).unwrap();
        assert!(fixtures.len() >= 3, "expected >= 3 example fixtures");
        let baseline = load_baseline(&dir.join("baseline.json")).unwrap();
        // A perfect run (engine returns judged keys in grade order) must
        // pass the committed baseline.
        let mut results = BTreeMap::new();
        for (_, f) in &fixtures {
            let mut keys: Vec<(u8, String)> =
                f.judged.iter().map(|j| (j.grade, j.key.clone())).collect();
            keys.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
            results.insert(f.query.clone(), keys.into_iter().map(|(_, k)| k).collect());
        }
        let report = evaluate(&fixtures, &results);
        assert!(check_regression(&report, &baseline).is_empty());
        // An empty run must trip the baseline.
        let empty = evaluate(&fixtures, &BTreeMap::new());
        assert!(!check_regression(&empty, &baseline).is_empty());
    }
}
