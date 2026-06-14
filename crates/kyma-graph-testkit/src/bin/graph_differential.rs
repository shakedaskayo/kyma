//! End-to-end differential gate: synth → ingest → existing Cypher subset
//! through a live kyma vs the petgraph oracle.
//!
//! Run by `scripts/test-graph-differential.sh` against a freshly reset stack.
//! Exits nonzero on any oracle/engine divergence (or engine error).
//!
//! Env: `KYMA_BASE_URL`, `KYMA_DATABASE`, `KYMA_TOKEN`, `KYMA_CLI_BIN` — see
//! `kyma_graph_testkit::ingest::EngineConfig::from_env`.

use kyma_graph_testkit::harness::{run_differential, HttpKymaEngine};
use kyma_graph_testkit::ingest::{load_graph, EngineConfig};
use kyma_graph_testkit::model::TestGraph;
use kyma_graph_testkit::oracle::ChainPattern;
use kyma_graph_testkit::synth;

/// ~10 forward-chain patterns over the `N`/`Hub` + `LINKS`/`CALLS`
/// conventions every generator follows: 1–3 hops, typed/untyped, labeled on
/// head/middle/tail.
fn patterns() -> Vec<ChainPattern> {
    vec![
        // 1-hop
        ChainPattern::start(None).hop(None, None),
        ChainPattern::start(None).hop(Some("LINKS"), None),
        ChainPattern::start(None).hop(Some("CALLS"), None),
        ChainPattern::start(Some("N")).hop(None, None),
        ChainPattern::start(Some("Hub")).hop(None, None),
        ChainPattern::start(None).hop(Some("CALLS"), Some("N")),
        // 2-hop
        ChainPattern::start(None).hop(None, None).hop(None, None),
        ChainPattern::start(None)
            .hop(Some("LINKS"), None)
            .hop(Some("LINKS"), None),
        ChainPattern::start(None)
            .hop(Some("LINKS"), Some("N"))
            .hop(Some("CALLS"), None),
        ChainPattern::start(Some("Hub")).hop(None, None).hop(None, None),
        // 3-hop
        ChainPattern::start(None)
            .hop(None, None)
            .hop(None, None)
            .hop(None, None),
        ChainPattern::start(None)
            .hop(Some("LINKS"), None)
            .hop(None, Some("N"))
            .hop(Some("LINKS"), None),
    ]
}

#[tokio::main]
async fn main() -> std::process::ExitCode {
    let cfg = EngineConfig::from_env();
    let topologies: Vec<(&str, TestGraph)> = vec![
        ("gd_power", synth::power_law(2000, 4, 42)),
        ("gd_grid", synth::grid(20, 20)),
        ("gd_cyclic", synth::cyclic(&[50, 80, 120], 60, 7)),
    ];

    let patterns = patterns();
    let mut failed = false;

    for (name, graph) in &topologies {
        println!(
            "\n=== {name}: {} nodes, {} edges ===",
            graph.node_count(),
            graph.edge_count()
        );
        if let Err(e) = load_graph(&cfg, graph, name).await {
            eprintln!("FATAL: failed to load {name} into the engine: {e:#}");
            return std::process::ExitCode::from(2);
        }
        println!("ingested + registered stored graph `{}/{name}`", cfg.database);

        let engine = HttpKymaEngine::new(
            cfg.base_url.clone(),
            cfg.database.clone(),
            (*name).to_string(),
            cfg.token.clone(),
        );
        let report = run_differential(&engine, graph, &patterns).await;
        print!("{}", report.render());
        if !report.is_clean() {
            failed = true;
        }
    }

    if failed {
        println!("\nRESULT: FAIL — oracle/engine divergence on the existing Cypher subset");
        std::process::ExitCode::from(1)
    } else {
        println!("\nRESULT: PASS — engine matches the oracle on all topologies/patterns");
        std::process::ExitCode::SUCCESS
    }
}
