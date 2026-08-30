//! Oracle baselines: k-hop + shortest-path on power-law graphs, and synth
//! generation throughput. See `kyma-graph-bench`'s crate docs for how S3
//! engine benches line up against these.

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use kyma_graph_testkit::model::TestGraph;
use kyma_graph_testkit::oracle::{Direction, Oracle};
use kyma_graph_testkit::synth;

/// (label, n_nodes) — avg_degree 4 ⇒ ~2 edges per node, so edge counts land
/// near the label. The 1M case only runs with `KYMA_BENCH_LARGE=1`.
fn sizes() -> Vec<(&'static str, usize)> {
    let mut v = vec![("10k_edges", 5_000), ("100k_edges", 50_000)];
    if std::env::var("KYMA_BENCH_LARGE").as_deref() == Ok("1") {
        v.push(("1M_edges", 500_000));
    }
    v
}

fn gen_graph(n_nodes: usize) -> TestGraph {
    synth::power_law(n_nodes, 4, 42)
}

fn bench_synth(c: &mut Criterion) {
    let mut group = c.benchmark_group("synth/power_law");
    for (label, n) in sizes() {
        // Throughput in edges generated per second.
        let edges = gen_graph(n).edge_count() as u64;
        group.throughput(Throughput::Elements(edges));
        group.sample_size(10);
        group.bench_with_input(BenchmarkId::from_parameter(label), &n, |b, &n| {
            b.iter(|| synth::power_law(n, 4, 42));
        });
    }
    group.finish();
}

fn bench_k_hop(c: &mut Criterion) {
    let mut group = c.benchmark_group("oracle/k_hop_k3");
    for (label, n) in sizes() {
        let graph = gen_graph(n);
        let oracle = Oracle::new(&graph);
        // Hubs (seed nodes) as sources — worst-case fan-in for Back/Both.
        let sources = ["n0", "n1", "n2", "n100"];
        group.sample_size(10);
        group.bench_function(BenchmarkId::new("fwd", label), |b| {
            b.iter(|| oracle.k_hop(&sources, 3, Direction::Fwd, None));
        });
        group.bench_function(BenchmarkId::new("back", label), |b| {
            b.iter(|| oracle.k_hop(&sources, 3, Direction::Back, None));
        });
    }
    group.finish();
}

fn bench_shortest_path(c: &mut Criterion) {
    let mut group = c.benchmark_group("oracle/shortest_path");
    for (label, n) in sizes() {
        let graph = gen_graph(n);
        let oracle = Oracle::new(&graph);
        // Last node → seed hub: longest expected directed path in BA graphs.
        let src = format!("n{}", n - 1);
        group.sample_size(10);
        group.bench_function(BenchmarkId::from_parameter(label), |b| {
            b.iter(|| oracle.shortest_path_len(&src, "n0", Direction::Fwd, None));
        });
    }
    group.finish();
}

criterion_group!(benches, bench_synth, bench_k_hop, bench_shortest_path);
criterion_main!(benches);
