//! Maximal Marginal Relevance (MMR) diversity re-ranking (M8.3a) — pure, no
//! I/O. Standard greedy MMR: at each step, pick the candidate maximizing
//! `lambda * relevance - (1 - lambda) * max_similarity_to_already_selected`,
//! so near-duplicate top results don't crowd out diverse ones.
//!
//! Deliberately duplicates a small `cosine_sim` here rather than reusing the
//! DataFusion vector UDFs in `pensieve-exec`: those operate on Arrow arrays
//! inside a `SessionContext`, not plain `Vec<f32>` — a 5-line duplicate is
//! cheaper than a cross-crate dependency for this.

fn cosine_sim(a: &[f32], b: &[f32]) -> f64 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let (mut dot, mut na, mut nb) = (0f64, 0f64, 0f64);
    for i in 0..a.len() {
        let (x, y) = (a[i] as f64, b[i] as f64);
        dot += x * y;
        na += x * x;
        nb += y * y;
    }
    if na == 0.0 || nb == 0.0 {
        return 0.0;
    }
    dot / (na.sqrt() * nb.sqrt())
}

/// Greedily re-rank `items` (id, embedding, relevance — already sorted by
/// relevance descending doesn't matter, this re-derives the order) by MMR,
/// returning up to `k` ids. `lambda` in `[0, 1]`: 1.0 = pure relevance (no
/// diversity effect), lower values favor spreading out near-duplicates.
/// Items with no embedding (`None`) are treated as maximally diverse from
/// everything else (similarity 0) — never penalized, since there's nothing to
/// compare, but also never contribute to future similarity comparisons.
pub fn mmr_select(items: &[(String, Option<Vec<f32>>, f64)], lambda: f64, k: usize) -> Vec<String> {
    if items.len() <= k {
        return items.iter().map(|(id, _, _)| id.clone()).collect();
    }
    let lambda = lambda.clamp(0.0, 1.0);
    let mut remaining: Vec<usize> = (0..items.len()).collect();
    let mut selected: Vec<usize> = Vec::with_capacity(k);

    while selected.len() < k && !remaining.is_empty() {
        let (best_pos, _) = remaining
            .iter()
            .enumerate()
            .map(|(pos, &i)| {
                let (_, emb_i, rel_i) = &items[i];
                let max_sim = selected
                    .iter()
                    .map(|&j| match (emb_i, &items[j].1) {
                        (Some(a), Some(b)) => cosine_sim(a, b),
                        _ => 0.0,
                    })
                    .fold(0.0_f64, f64::max);
                let mmr_score = lambda * rel_i - (1.0 - lambda) * max_sim;
                (pos, mmr_score)
            })
            .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
            .expect("remaining is non-empty");
        selected.push(remaining.remove(best_pos));
    }
    selected.into_iter().map(|i| items[i].0.clone()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returns_all_when_pool_not_larger_than_k() {
        let items = vec![
            ("a".to_string(), Some(vec![1.0, 0.0]), 0.9),
            ("b".to_string(), Some(vec![0.0, 1.0]), 0.5),
        ];
        let out = mmr_select(&items, 0.7, 5);
        assert_eq!(out, vec!["a", "b"]);
    }

    #[test]
    fn diversifies_near_duplicates_when_lambda_low() {
        // a and b are near-identical (high similarity); c is orthogonal.
        // With lambda favoring diversity, c should be picked over the
        // slightly-lower-relevance-but-redundant b.
        let items = vec![
            ("a".to_string(), Some(vec![1.0, 0.0]), 0.95),
            ("b".to_string(), Some(vec![0.99, 0.01]), 0.90),
            ("c".to_string(), Some(vec![0.0, 1.0]), 0.80),
        ];
        let out = mmr_select(&items, 0.3, 2);
        assert_eq!(out[0], "a", "highest relevance always picked first");
        assert_eq!(out[1], "c", "diverse candidate beats a near-duplicate");
    }

    #[test]
    fn pure_relevance_when_lambda_is_one() {
        let items = vec![
            ("a".to_string(), Some(vec![1.0, 0.0]), 0.95),
            ("b".to_string(), Some(vec![0.99, 0.01]), 0.90),
            ("c".to_string(), Some(vec![0.0, 1.0]), 0.80),
        ];
        let out = mmr_select(&items, 1.0, 2);
        assert_eq!(
            out,
            vec!["a", "b"],
            "lambda=1 ⇒ diversity term never matters"
        );
    }

    #[test]
    fn missing_embeddings_never_penalized_or_compared() {
        let items = vec![
            ("a".to_string(), None, 0.9),
            ("b".to_string(), None, 0.8),
            ("c".to_string(), Some(vec![1.0, 0.0]), 0.7),
        ];
        let out = mmr_select(&items, 0.3, 2);
        assert_eq!(out, vec!["a", "b"]);
    }

    #[test]
    fn empty_input_returns_empty() {
        assert!(mmr_select(&[], 0.7, 5).is_empty());
    }
}
