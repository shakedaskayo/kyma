<script setup lang="ts">
// Quiet landing — just like every other good docs site (React, Vue, Vitest,
// Tailwind). Title, tagline, two CTAs, and a clean grid of links into the
// actual sections. Nothing that "screams."
</script>

<template>
  <div class="kyma-landing">
    <section class="kyma-hero">
      <h1>kyma</h1>
      <p class="kyma-tagline">Production knowledge, as a query.</p>
      <p class="kyma-lede">
        A unified columnar query engine for every signal your stack emits —
        logs, traces, metrics, plus first-class federation to Postgres,
        MySQL, and MongoDB. Sub-second latency over a decade of history.
      </p>
      <div class="kyma-cta-row">
        <a href="/quickstart/five-minute-start" class="kyma-cta kyma-cta--primary">
          Quickstart
        </a>
        <a href="/concepts/what-is-kyma" class="kyma-cta">
          Concepts
        </a>
      </div>
    </section>

    <section class="kyma-sections">
      <a class="kyma-section-card" href="/quickstart/">
        <h3>Quickstart</h3>
        <p>Five minutes from <code>docker compose up</code> to your first KQL query.</p>
      </a>
      <a class="kyma-section-card" href="/concepts/">
        <h3>Concepts</h3>
        <p>How the engine thinks: invariants, the pruning cascade, schema, agents.</p>
      </a>
      <a class="kyma-section-card" href="/ingest/">
        <h3>Ingest</h3>
        <p>OTLP gRPC, REST/NDJSON, Kafka, file-drop. One write path on the far side.</p>
      </a>
      <a class="kyma-section-card" href="/query/">
        <h3>Query</h3>
        <p>KQL, SQL, and the agent endpoint. Arrow Flight gRPC for zero-copy streams.</p>
      </a>
      <a class="kyma-section-card" href="/connectors/">
        <h3>Connectors</h3>
        <p>Postgres, MySQL, MongoDB. Federate live, or sync via CDC.</p>
      </a>
      <a class="kyma-section-card" href="/architecture/architecture">
        <h3>Architecture</h3>
        <p>The five invariants and how they hold the engine together.</p>
      </a>
    </section>
  </div>
</template>
