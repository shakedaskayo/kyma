<!--
  KymaMultiSourceFlow
  -------------------
  Visual: a SQL query joining kyma's native otel_logs with a federated
  pg_prod.public.users table. Shows two paths through the engine —
  the federation lane (live read) and the sync lane (CDC into kyma extents) —
  meeting at the DataFusion join.
-->
<script setup lang="ts">
defineProps<{
  caption?: string
}>()
</script>

<template>
  <figure class="kyma-msf">
    <svg viewBox="0 0 1040 480" role="img"
         :aria-label="caption ?? 'Multi-source data flow: federation and sync, joined in DataFusion'"
         xmlns="http://www.w3.org/2000/svg">

      <defs>
        <marker id="msf-arrow-fg" viewBox="0 0 10 10" refX="9" refY="5"
                markerWidth="8" markerHeight="8" orient="auto">
          <path d="M0 0 L10 5 L0 10 Z" fill="currentColor" opacity="0.6"/>
        </marker>
        <marker id="msf-arrow-accent" viewBox="0 0 10 10" refX="9" refY="5"
                markerWidth="8" markerHeight="8" orient="auto">
          <path d="M0 0 L10 5 L0 10 Z" class="msf-accent-fill"/>
        </marker>
      </defs>

      <!-- Title strip -->
      <text x="20" y="28" class="msf-eyebrow">// MULTI-SOURCE</text>
      <text x="20" y="50" class="msf-title">SELECT u.email, COUNT(*) FROM pg_prod.public.users u JOIN otel_logs l ON l.user_id = u.id …</text>

      <!-- ───────── External database ───────── -->
      <g transform="translate(40, 100)">
        <rect width="200" height="200" class="msf-source-rect"/>
        <text x="100" y="28" class="msf-node-title" text-anchor="middle">Postgres · prod</text>
        <text x="100" y="46" class="msf-node-sub" text-anchor="middle">public.users · 2.3M rows</text>

        <line x1="20" y1="62" x2="180" y2="62" class="msf-rule"/>

        <!-- Federation lane label -->
        <text x="100" y="84" class="msf-lane-label" text-anchor="middle">FEDERATION</text>
        <text x="100" y="100" class="msf-lane-sub" text-anchor="middle">live · pushdown</text>

        <line x1="40" y1="120" x2="160" y2="120" class="msf-rule msf-rule-soft"/>

        <!-- Sync lane label -->
        <text x="100" y="142" class="msf-lane-label" text-anchor="middle">SYNC</text>
        <text x="100" y="158" class="msf-lane-sub" text-anchor="middle">CDC · exactly-once</text>
        <text x="100" y="178" class="msf-lane-sub-em" text-anchor="middle">replication slot</text>
      </g>

      <!-- Federation pathway: PostgresSource scan -->
      <g transform="translate(290, 130)">
        <rect width="200" height="80" class="msf-pathway-rect"/>
        <text x="100" y="22" class="msf-pathway-title" text-anchor="middle">PostgresSource::scan</text>
        <text x="100" y="40" class="msf-pathway-sub" text-anchor="middle">SQL pushed: WHERE region = $1</text>
        <text x="100" y="56" class="msf-pathway-sub" text-anchor="middle">projection: id, email</text>
        <text x="100" y="72" class="msf-pathway-sub-em" text-anchor="middle">→ Arrow batches</text>
      </g>

      <!-- Sync pathway: CdcConnector -->
      <g transform="translate(290, 230)">
        <rect width="200" height="80" class="msf-pathway-rect"/>
        <text x="100" y="22" class="msf-pathway-title" text-anchor="middle">CdcConnector::stream</text>
        <text x="100" y="40" class="msf-pathway-sub" text-anchor="middle">snapshot · stream events</text>
        <text x="100" y="56" class="msf-pathway-sub" text-anchor="middle">commit + cursor (CAS)</text>
        <text x="100" y="72" class="msf-pathway-sub-em" text-anchor="middle">→ kyma extents</text>
      </g>

      <!-- arrows: source → both pathways -->
      <line x1="240" y1="170" x2="288" y2="170" class="msf-link msf-link-accent" marker-end="url(#msf-arrow-accent)"/>
      <line x1="240" y1="270" x2="288" y2="270" class="msf-link msf-link-accent" marker-end="url(#msf-arrow-accent)"/>

      <!-- ───────── kyma engine ───────── -->
      <g transform="translate(540, 100)">
        <rect width="220" height="280" class="msf-engine-rect"/>
        <text x="110" y="28" class="msf-engine-title" text-anchor="middle">kyma · DataFusion</text>
        <text x="110" y="46" class="msf-engine-sub" text-anchor="middle">SessionContext</text>

        <line x1="20" y1="62" x2="200" y2="62" class="msf-rule"/>

        <!-- Federated catalog -->
        <text x="20" y="86" class="msf-engine-label">▸ pg_prod (catalog)</text>
        <text x="20" y="102" class="msf-engine-detail">FederatedTableProvider</text>
        <text x="20" y="116" class="msf-engine-detail">live · per-query scan</text>

        <line x1="20" y1="135" x2="200" y2="135" class="msf-rule msf-rule-soft"/>

        <!-- Native otel_logs -->
        <text x="20" y="156" class="msf-engine-label">▸ kyma (catalog)</text>
        <text x="20" y="172" class="msf-engine-detail">otel_logs · synced</text>
        <text x="20" y="186" class="msf-engine-detail">14k extents · pruned</text>

        <line x1="20" y1="205" x2="200" y2="205" class="msf-rule msf-rule-soft"/>

        <!-- Plan -->
        <text x="20" y="226" class="msf-engine-label">▸ HashJoin</text>
        <text x="20" y="242" class="msf-engine-detail">build: pg.users (3k)</text>
        <text x="20" y="256" class="msf-engine-detail">probe: otel_logs (1.2M)</text>
      </g>

      <!-- arrows: pathways → engine -->
      <line x1="490" y1="170" x2="538" y2="180" class="msf-link msf-link-accent" marker-end="url(#msf-arrow-accent)"/>
      <line x1="490" y1="270" x2="538" y2="240" class="msf-link msf-link-accent" marker-end="url(#msf-arrow-accent)"/>

      <!-- ───────── Result ───────── -->
      <g transform="translate(810, 180)">
        <rect width="200" height="120" class="msf-result-rect"/>
        <text x="100" y="28" class="msf-result-title" text-anchor="middle">Result</text>
        <text x="100" y="48" class="msf-result-sub" text-anchor="middle">Arrow Flight · NDJSON</text>

        <line x1="20" y1="60" x2="180" y2="60" class="msf-rule"/>

        <text x="100" y="80" class="msf-result-stat" text-anchor="middle">
          <tspan class="msf-num">142 ms</tspan> wall
        </text>
        <text x="100" y="100" class="msf-result-stat" text-anchor="middle">
          <tspan class="msf-num">5</tspan> top customers
        </text>
      </g>

      <!-- arrow: engine → result -->
      <line x1="760" y1="240" x2="808" y2="240" class="msf-link msf-link-accent" marker-end="url(#msf-arrow-accent)"/>

      <!-- Footer note -->
      <text x="20" y="450" class="msf-footer-label">// QUERY-TIME RESOLUTION</text>
      <text x="20" y="466" class="msf-footer">
        <tspan class="msf-mono">SELECT … FROM pg_prod.public.users</tspan> resolves to the synced extents (default).
      </text>
      <text x="20" y="478" class="msf-footer">
        <tspan class="msf-mono">SELECT … FROM live(pg_prod.public.users)</tspan> resolves to the federated path.
      </text>
    </svg>
    <figcaption v-if="caption">{{ caption }}</figcaption>
  </figure>
</template>

<style scoped>
.kyma-msf {
  margin: 1.5rem 0 2rem;
  color: var(--kyma-fg);
}
.kyma-msf svg {
  width: 100%;
  height: auto;
  display: block;
  background: var(--kyma-bg-soft);
  border: 1px solid var(--kyma-rule-soft);
}
.kyma-msf figcaption {
  margin-top: 0.75rem;
  font-size: 0.78rem;
  color: var(--kyma-muted);
  text-align: center;
  font-family: var(--kyma-font-mono);
}

.msf-eyebrow {
  font-family: var(--kyma-font-mono);
  font-size: 11px;
  fill: var(--kyma-muted);
  letter-spacing: 0.18em;
}
.msf-title {
  font-family: var(--kyma-font-mono);
  font-size: 12px;
  fill: var(--kyma-fg);
  letter-spacing: -0.005em;
}

.msf-source-rect {
  fill: var(--kyma-bg);
  stroke: var(--kyma-rule);
  stroke-width: 1;
}
.msf-pathway-rect {
  fill: transparent;
  stroke: var(--kyma-accent);
  stroke-width: 1;
  stroke-dasharray: 3 3;
}
.msf-engine-rect {
  fill: var(--kyma-bg);
  stroke: var(--kyma-fg);
  stroke-width: 1.4;
}
.msf-result-rect {
  fill: var(--kyma-bg);
  stroke: var(--kyma-rule);
  stroke-width: 1;
}

.msf-node-title, .msf-pathway-title, .msf-engine-title, .msf-result-title {
  font-family: var(--kyma-font-mono);
  font-weight: 600;
  fill: var(--kyma-fg);
  letter-spacing: -0.005em;
}
.msf-node-title { font-size: 13px; }
.msf-pathway-title { font-size: 12px; }
.msf-engine-title { font-size: 13px; }
.msf-result-title { font-size: 12px; }

.msf-node-sub, .msf-pathway-sub, .msf-engine-sub, .msf-result-sub {
  font-family: var(--kyma-font-body);
  font-size: 10px;
  fill: var(--kyma-fg-soft);
}
.msf-pathway-sub-em {
  font-family: var(--kyma-font-mono);
  font-size: 10px;
  fill: var(--kyma-accent);
  letter-spacing: 0.04em;
}

.msf-lane-label {
  font-family: var(--kyma-font-mono);
  font-size: 10px;
  fill: var(--kyma-fg);
  font-weight: 600;
  letter-spacing: 0.16em;
}
.msf-lane-sub {
  font-family: var(--kyma-font-body);
  font-size: 9px;
  fill: var(--kyma-fg-soft);
}
.msf-lane-sub-em {
  font-family: var(--kyma-font-mono);
  font-size: 9px;
  fill: var(--kyma-accent);
  letter-spacing: 0.04em;
}

.msf-engine-label {
  font-family: var(--kyma-font-mono);
  font-size: 10.5px;
  fill: var(--kyma-fg);
  font-weight: 500;
}
.msf-engine-detail {
  font-family: var(--kyma-font-body);
  font-size: 9.5px;
  fill: var(--kyma-fg-soft);
}

.msf-result-stat {
  font-family: var(--kyma-font-mono);
  font-size: 11px;
  fill: var(--kyma-fg);
}
.msf-num {
  fill: var(--kyma-accent);
  font-weight: 600;
}

.msf-link {
  stroke: var(--kyma-fg);
  stroke-width: 1;
  opacity: 0.55;
  fill: none;
}
.msf-link-accent {
  stroke: var(--kyma-accent);
  stroke-width: 1.4;
  opacity: 0.95;
}
.msf-accent-fill { fill: var(--kyma-accent); }

.msf-rule {
  stroke: var(--kyma-rule);
  stroke-width: 1;
}
.msf-rule-soft {
  stroke: var(--kyma-rule-soft);
  stroke-width: 1;
}

.msf-footer-label {
  font-family: var(--kyma-font-mono);
  font-size: 10px;
  fill: var(--kyma-muted);
  letter-spacing: 0.18em;
}
.msf-footer {
  font-family: var(--kyma-font-body);
  font-size: 10px;
  fill: var(--kyma-fg-soft);
}
.msf-mono {
  font-family: var(--kyma-font-mono);
  fill: var(--kyma-accent);
  font-weight: 500;
}
</style>
