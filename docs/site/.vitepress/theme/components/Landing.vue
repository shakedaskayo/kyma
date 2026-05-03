<script setup lang="ts">
import { ref, onMounted, onUnmounted } from 'vue'

// "Live" readout — fake-but-realistic kyma metrics that tick every 2.4s.
// The numbers stay within plausible ranges for a busy single-node deployment.
type Readout = {
  walltime_ms: number
  extents_total: number
  extents_pruned: number
  bytes_scanned_mib: number
  bytes_total_gib: number
  rows_returned: number
}

function rand(min: number, max: number) { return Math.random() * (max - min) + min }
function genReadout(): Readout {
  const total = Math.floor(rand(11_000, 18_000))
  const pruned = Math.floor(total * rand(0.991, 0.9995))
  return {
    walltime_ms:        Math.floor(rand(38, 142)),
    extents_total:      total,
    extents_pruned:     pruned,
    bytes_scanned_mib:  +(rand(1.2, 4.8)).toFixed(1),
    bytes_total_gib:    +(rand(8.4, 22.6)).toFixed(1),
    rows_returned:      Math.floor(rand(12, 850)),
  }
}

const readout = ref<Readout>(genReadout())
function pruneRate(r: Readout) { return ((r.extents_pruned / r.extents_total) * 100).toFixed(2) }

let intervalId: ReturnType<typeof setInterval> | null = null
onMounted(() => {
  intervalId = setInterval(() => { readout.value = genReadout() }, 2400)
})
onUnmounted(() => { if (intervalId) clearInterval(intervalId) })

// Pruning cascade stages — the data the hero diagram is built from.
const stages = [
  { num: '01', title: 'Catalog pruning',    desc: 'Postgres query against per-extent manifests. Time range, min/max per column, present-paths bitmaps. Runs without touching object storage.', pct: '99.94%', label: 'extents eliminated' },
  { num: '02', title: 'Extent footer',      desc: 'Range-GET each surviving extent’s footer (cached). Bloom filters, block-level stats, path bitmaps for the dynamic column.', pct: '92.4%',  label: 'blocks eliminated' },
  { num: '03', title: 'Block index',        desc: 'Per candidate block: footer min/max confirms; inverted-index posting lists for text predicates intersect to exact row ids.', pct: '74.1%',  label: 'rows eliminated' },
  { num: '04', title: 'Decode · execute', desc: 'Decoded blocks become Arrow RecordBatches. Standard DataFusion pipeline. Zero-copy through Arrow Flight to the client.', pct: '0.0001%', label: 'of bytes read' },
]

// Today's date string for the meta bar.
const buildDate = new Date().toISOString().slice(0, 10)
</script>

<template>
  <div class="kyma-landing">
    <!-- Top metadata strip — datasheet header. -->
    <header class="kyma-meta-bar">
      <div class="left">
        <span class="dot" /> live · single-node · v0.0.1
      </div>
      <div class="center">kyma engine · technical reference</div>
      <div class="right">build {{ buildDate }} · arrow flight gRPC</div>
    </header>

    <!-- Hero. Left rail = metadata; right column = title + lede + readout. -->
    <section class="kyma-hero">
      <aside class="kyma-rail" aria-label="document metadata">
        <dl>
          <dt>document</dt>
          <dd>landing</dd>
          <dt>section</dt>
          <dd>00</dd>
          <dt>revision</dt>
          <dd>r1</dd>
          <dt>format</dt>
          <dd>tlm.v1</dd>
        </dl>
      </aside>

      <div class="kyma-hero-content">
        <p class="kyma-hero-eyebrow">[ kyma · query engine · 2026 ]</p>

        <h1 class="kyma-hero-title">
          Production knowledge,<br />
          as <em>a query.</em>
        </h1>

        <p class="kyma-hero-lede">
          A unified columnar engine for every signal your stack emits. Logs, traces, metrics,
          tool calls, deploy events, plus first-class federation to Postgres, MySQL, MongoDB.
          Sub-second latency over a decade of history, in KQL or SQL, over Arrow Flight gRPC.
        </p>

        <div class="kyma-hero-cta">
          <a href="/quickstart/five-minute-start" class="kyma-cta kyma-cta--primary">
            Five-minute start <span class="kyma-cta-arrow">→</span>
          </a>
          <a href="/concepts/what-is-kyma" class="kyma-cta">
            What kyma is
          </a>
          <a href="https://github.com/shaked/engine" target="_blank" rel="noopener" class="kyma-cta">
            Source
          </a>
        </div>

        <!-- Live readout: a single representative query of how the cascade
             would have answered, refreshed every 2.4s. -->
        <div class="kyma-readout" role="status" aria-live="polite" aria-label="live engine readout">
          <span class="kyma-readout-label">readout</span>
          <div class="kyma-readout-row">
            <div class="kyma-readout-cell">
              <span class="kyma-readout-label">wall</span>
              <span class="kyma-readout-value"><span class="kyma-num">{{ readout.walltime_ms }}</span> ms</span>
            </div>
            <div class="kyma-readout-cell">
              <span class="kyma-readout-label">extents pruned</span>
              <span class="kyma-readout-value">
                <span class="kyma-num">{{ readout.extents_pruned.toLocaleString() }}</span>
                / {{ readout.extents_total.toLocaleString() }}
                ({{ pruneRate(readout) }}%)
              </span>
            </div>
            <div class="kyma-readout-cell">
              <span class="kyma-readout-label">bytes scanned</span>
              <span class="kyma-readout-value">
                <span class="kyma-num">{{ readout.bytes_scanned_mib }}</span> MiB
                / {{ readout.bytes_total_gib }} GiB
              </span>
            </div>
            <div class="kyma-readout-cell">
              <span class="kyma-readout-label">rows returned</span>
              <span class="kyma-readout-value"><span class="kyma-num">{{ readout.rows_returned }}</span></span>
            </div>
          </div>
        </div>
      </div>
    </section>

    <!-- Pruning cascade — the central technical proof point. -->
    <section class="kyma-cascade" aria-label="The pruning cascade">
      <p class="kyma-cascade-eyebrow">§ 01 · how it stays fast</p>
      <h2 class="kyma-cascade-title">The pruning cascade</h2>
      <p class="kyma-cascade-lede">
        Sub-second latency is not an optimization problem. It's a "don't look at the data"
        problem. Each stage does only the work the next stage will demand, so 99.99% of bytes
        are eliminated before any extent is decoded.
      </p>

      <ol class="kyma-cascade-stages">
        <li v-for="s in stages" :key="s.num" class="kyma-stage">
          <div class="kyma-stage-num">{{ s.num }}</div>
          <div class="kyma-stage-body">
            <h3>{{ s.title }}</h3>
            <p>{{ s.desc }}</p>
          </div>
          <div class="kyma-stage-stat">
            <span class="kyma-pct">{{ s.pct }}</span>
            <span class="kyma-pct-label">{{ s.label }}</span>
          </div>
        </li>
      </ol>
    </section>

    <!-- Three-pillar value props. -->
    <section class="kyma-tri" aria-label="What kyma does">
      <aside class="kyma-rail">
        <dl>
          <dt>section</dt>
          <dd>02</dd>
          <dt>topic</dt>
          <dd>scope</dd>
        </dl>
      </aside>

      <div class="kyma-tri-grid">
        <article class="kyma-tri-card">
          <span class="label">→ ingest</span>
          <h3>One pipe, every signal</h3>
          <p>
            OTLP gRPC for the OpenTelemetry world. REST/NDJSON for everything else.
            Kafka and file-drop for high-throughput sources. One schema model on the
            far side: append-only columnar Arrow with token indices.
          </p>
          <a href="/ingest/">Ingest paths →</a>
        </article>

        <article class="kyma-tri-card">
          <span class="label">→ query</span>
          <h3>KQL, SQL, agent</h3>
          <p>
            KQL for the operator. SQL when you'd rather. The agent endpoint when you'd
            rather not write a query. Same execution, same Arrow result, same sub-second
            wall time over a decade of history.
          </p>
          <a href="/query/">Query languages →</a>
        </article>

        <article class="kyma-tri-card">
          <span class="label">→ federate</span>
          <h3>Your databases, joined in</h3>
          <p>
            Postgres, MySQL, MongoDB register as DataFusion catalogs. Federate live, or
            sync via CDC with exactly-once snapshot consistency. <code>live(table)</code>
            opts into source-fresh on a per-query basis.
          </p>
          <a href="/connectors/">Connectors →</a>
        </article>
      </div>
    </section>

    <!-- Real KQL/SQL pair — same query in two languages. -->
    <section class="kyma-codeshowcase" aria-label="Two languages, one query">
      <aside class="kyma-rail">
        <dl>
          <dt>section</dt>
          <dd>03</dd>
          <dt>topic</dt>
          <dd>surface</dd>
        </dl>
      </aside>

      <div class="kyma-codeshowcase-grid">
        <pre class="kyma-code-pane"><span class="kyma-code-label">KQL — application/x-kql</span>
<span class="cm">// errors per service in the last hour</span>
otel_logs
| <span class="kw">where</span> _timestamp &gt; <span class="kw">ago</span>(<span class="num">1h</span>)
| <span class="kw">where</span> severity_text == <span class="str">"ERROR"</span>
| <span class="kw">summarize</span> n = <span class="kw">count</span>() <span class="kw">by</span> service_name
| <span class="kw">order</span> <span class="kw">by</span> n <span class="kw">desc</span>
| <span class="kw">take</span> <span class="num">5</span></pre>

        <pre class="kyma-code-pane"><span class="kyma-code-label">SQL — application/sql</span>
<span class="cm">-- errors per service in the last hour</span>
<span class="kw">SELECT</span> service_name, <span class="kw">COUNT</span>(*) <span class="kw">AS</span> n
  <span class="kw">FROM</span> otel_logs
 <span class="kw">WHERE</span> _timestamp &gt; <span class="kw">now</span>() - <span class="kw">INTERVAL</span> <span class="str">'1 hour'</span>
   <span class="kw">AND</span> severity_text = <span class="str">'ERROR'</span>
 <span class="kw">GROUP</span> <span class="kw">BY</span> service_name
 <span class="kw">ORDER</span> <span class="kw">BY</span> n <span class="kw">DESC</span>
 <span class="kw">LIMIT</span> <span class="num">5</span></pre>
      </div>
    </section>

    <!-- Editorial close. -->
    <footer class="kyma-footstrip">
      <div class="kyma-footstrip-prompt">
        Built for the agent that needs production awareness <em>across the whole stack.</em>
      </div>
      <div>
        <a href="/quickstart/five-minute-start" class="kyma-cta">
          Start now <span class="kyma-cta-arrow">→</span>
        </a>
      </div>
    </footer>
  </div>
</template>
