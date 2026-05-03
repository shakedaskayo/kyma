<!--
  KymaArchitectureDiagram
  -----------------------
  Single-pane view of kyma's architecture: ingest lane (top), shared storage spine
  (middle, the source of truth), and query lane (bottom). Inherits theme colors
  via the kyma-* CSS variables, so it respects dark/light mode.

  Used as the hero diagram on /architecture/ and on /concepts/the-pruning-cascade
  (compact variant via the `compact` prop, which hides the long descriptions).
-->
<script setup lang="ts">
defineProps<{
  caption?: string
}>()
</script>

<template>
  <figure class="kyma-arch">
    <svg viewBox="0 0 1040 540" role="img"
         :aria-label="caption ?? 'kyma architecture: ingest path, shared storage spine, query path'"
         xmlns="http://www.w3.org/2000/svg">
      <defs>
        <!-- Hairline arrowhead, matches link/citation blue -->
        <marker id="arrow-fg" viewBox="0 0 10 10" refX="9" refY="5"
                markerWidth="8" markerHeight="8" orient="auto-start-reverse">
          <path d="M0 0 L10 5 L0 10 Z" fill="currentColor" opacity="0.55"/>
        </marker>
        <marker id="arrow-accent" viewBox="0 0 10 10" refX="9" refY="5"
                markerWidth="8" markerHeight="8" orient="auto-start-reverse">
          <path d="M0 0 L10 5 L0 10 Z" class="arch-accent-fill"/>
        </marker>

        <!-- Subtle hatch fill for the "object storage" plate -->
        <pattern id="hatch" patternUnits="userSpaceOnUse" width="6" height="6"
                 patternTransform="rotate(45)">
          <rect width="6" height="6" fill="transparent"/>
          <line x1="0" y1="0" x2="0" y2="6" stroke="currentColor"
                stroke-width="0.6" opacity="0.18"/>
        </pattern>
      </defs>

      <!-- ─────────────── Ingest lane (top) ─────────────── -->
      <g class="lane lane-ingest">
        <text x="20" y="38" class="lane-label">// INGEST · 01</text>

        <!-- Sources -->
        <g class="node" transform="translate(20, 60)">
          <rect width="120" height="48" />
          <text x="60" y="20" class="node-title">OTLP gRPC</text>
          <text x="60" y="36" class="node-sub">logs · traces · metrics</text>
        </g>
        <g class="node" transform="translate(20, 116)">
          <rect width="120" height="48" />
          <text x="60" y="20" class="node-title">REST / NDJSON</text>
          <text x="60" y="36" class="node-sub">push · idempotent</text>
        </g>
        <g class="node" transform="translate(20, 172)">
          <rect width="120" height="48" />
          <text x="60" y="20" class="node-title">Kafka</text>
          <text x="60" y="36" class="node-sub">exactly-once</text>
        </g>
        <g class="node" transform="translate(20, 228)">
          <rect width="120" height="48" />
          <text x="60" y="20" class="node-title">file-drop</text>
          <text x="60" y="36" class="node-sub">SHA256 idempotency</text>
        </g>

        <!-- Coercion → Staging buffer → Commit coordinator -->
        <g class="node node-mid" transform="translate(190, 60)">
          <rect width="160" height="64" />
          <text x="80" y="22" class="node-title">Frontend</text>
          <text x="80" y="40" class="node-sub">JSON → Arrow</text>
          <text x="80" y="55" class="node-sub">schema coerce</text>
        </g>
        <g class="node node-mid" transform="translate(190, 144)">
          <rect width="160" height="64" />
          <text x="80" y="22" class="node-title">Staging buffer</text>
          <text x="80" y="40" class="node-sub">per-table group commit</text>
          <text x="80" y="55" class="node-sub">batch · debounce</text>
        </g>
        <g class="node node-mid" transform="translate(190, 228)">
          <rect width="160" height="48" />
          <text x="80" y="20" class="node-title">Commit coordinator</text>
          <text x="80" y="36" class="node-sub">CAS snapshot</text>
        </g>

        <!-- Connectors (pull-side) -->
        <g class="node node-pull" transform="translate(390, 60)">
          <rect width="180" height="120" />
          <text x="90" y="22" class="node-title">Connectors</text>
          <text x="90" y="42" class="node-sub">Postgres · MySQL · Mongo</text>
          <text x="90" y="58" class="node-sub">federation · sync</text>
          <text x="90" y="78" class="node-sub">Prometheus · Sentry</text>
          <text x="90" y="94" class="node-sub">scheduled · pull</text>
          <text x="90" y="110" class="node-sub-em">→ shared write-path</text>
        </g>

        <!-- arrows: sources → frontend → staging → commit -->
        <line x1="140" y1="84"  x2="190" y2="92"  class="link" marker-end="url(#arrow-fg)"/>
        <line x1="140" y1="140" x2="190" y2="92"  class="link" marker-end="url(#arrow-fg)"/>
        <line x1="140" y1="196" x2="190" y2="92"  class="link" marker-end="url(#arrow-fg)"/>
        <line x1="140" y1="252" x2="190" y2="92"  class="link" marker-end="url(#arrow-fg)"/>
        <line x1="270" y1="124" x2="270" y2="144" class="link" marker-end="url(#arrow-fg)"/>
        <line x1="270" y1="208" x2="270" y2="228" class="link" marker-end="url(#arrow-fg)"/>
        <line x1="350" y1="252" x2="392" y2="120" class="link" marker-end="url(#arrow-fg)"/>
        <line x1="390" y1="120" x2="350" y2="92"  class="link" marker-end="url(#arrow-fg)"/>
      </g>

      <!-- ─────────────── Storage spine (middle) ─────────────── -->
      <g class="lane lane-storage">
        <text x="20" y="305" class="lane-label">// STORAGE · 02 · the only source of truth</text>

        <!-- Object store (left) -->
        <g class="store-plate" transform="translate(20, 320)">
          <rect width="500" height="80" fill="url(#hatch)"/>
          <rect width="500" height="80" />
          <text x="20" y="26" class="store-title">Object storage · S3 · MinIO</text>
          <text x="20" y="46" class="store-sub">columnar Arrow extents · immutable · time-bounded</text>
          <text x="20" y="64" class="store-sub-em">tlm.v1 format · footers · token indices</text>
        </g>

        <!-- Catalog (right) -->
        <g class="store-plate" transform="translate(540, 320)">
          <rect width="480" height="80" />
          <text x="20" y="26" class="store-title">Postgres catalog</text>
          <text x="20" y="46" class="store-sub">manifests · per-extent stats · CAS commits</text>
          <text x="20" y="64" class="store-sub-em">externalized from byte one</text>
        </g>

        <!-- arrow ingest → storage -->
        <path class="link link-accent"
              d="M270 276 Q 270 300, 270 318"
              fill="none" marker-end="url(#arrow-accent)"/>
        <text x="290" y="295" class="link-label">extent</text>

        <path class="link link-accent"
              d="M390 276 Q 720 290, 770 318"
              fill="none" marker-end="url(#arrow-accent)"/>
        <text x="650" y="298" class="link-label">manifest</text>
      </g>

      <!-- ─────────────── Query lane (bottom) ─────────────── -->
      <g class="lane lane-query">
        <text x="20" y="430" class="lane-label">// QUERY · 03</text>

        <g class="node" transform="translate(20, 448)">
          <rect width="160" height="64" />
          <text x="80" y="22" class="node-title">Frontend</text>
          <text x="80" y="40" class="node-sub">KQL · SQL · PromQL*</text>
          <text x="80" y="55" class="node-sub">→ logical plan</text>
        </g>

        <g class="node" transform="translate(200, 448)">
          <rect width="160" height="64" />
          <text x="80" y="22" class="node-title">Planner</text>
          <text x="80" y="40" class="node-sub">3-level pruning</text>
          <text x="80" y="55" class="node-sub">cascade</text>
        </g>

        <g class="node" transform="translate(380, 448)">
          <rect width="160" height="64" />
          <text x="80" y="22" class="node-title">DataFusion</text>
          <text x="80" y="40" class="node-sub">vectorized exec</text>
          <text x="80" y="55" class="node-sub">over Arrow</text>
        </g>

        <g class="node" transform="translate(560, 448)">
          <rect width="160" height="64" />
          <text x="80" y="22" class="node-title">Arrow Flight</text>
          <text x="80" y="40" class="node-sub">zero-copy stream</text>
          <text x="80" y="55" class="node-sub">to client / agent</text>
        </g>

        <!-- arrows -->
        <line x1="180" y1="480" x2="200" y2="480" class="link" marker-end="url(#arrow-fg)"/>
        <line x1="360" y1="480" x2="380" y2="480" class="link" marker-end="url(#arrow-fg)"/>
        <line x1="540" y1="480" x2="560" y2="480" class="link" marker-end="url(#arrow-fg)"/>

        <!-- arrows up from storage to query (planner reads catalog; exec reads extents) -->
        <path class="link link-accent"
              d="M280 400 Q 280 425, 280 448"
              fill="none" marker-end="url(#arrow-accent)"/>
        <text x="295" y="425" class="link-label">stats</text>
        <path class="link link-accent"
              d="M460 400 Q 460 425, 460 448"
              fill="none" marker-end="url(#arrow-accent)"/>
        <text x="475" y="425" class="link-label">bytes</text>
      </g>
    </svg>
    <figcaption v-if="caption">{{ caption }}</figcaption>
  </figure>
</template>

<style scoped>
.kyma-arch {
  margin: 1.5rem 0 2rem;
  font-family: var(--kyma-font-mono);
  color: var(--kyma-fg);
}
.kyma-arch svg {
  width: 100%;
  height: auto;
  display: block;
  background: var(--kyma-bg-soft);
  border: 1px solid var(--kyma-rule-soft);
}
.kyma-arch figcaption {
  margin-top: 0.75rem;
  font-size: 0.78rem;
  letter-spacing: 0.04em;
  color: var(--kyma-muted);
  text-align: center;
}

/* ---- Lane labels ---- */
.lane-label {
  font-family: var(--kyma-font-mono);
  font-size: 11px;
  letter-spacing: 0.18em;
  fill: var(--kyma-muted);
  text-transform: uppercase;
}

/* ---- Generic node boxes ---- */
.node rect {
  fill: var(--kyma-bg);
  stroke: var(--kyma-rule);
  stroke-width: 1;
}
.node-mid rect {
  fill: var(--kyma-bg-soft);
  stroke: var(--kyma-rule-soft);
}
.node-pull rect {
  fill: transparent;
  stroke: var(--kyma-accent);
  stroke-width: 1;
  stroke-dasharray: 3 3;
}

.node-title {
  font-family: var(--kyma-font-mono);
  font-size: 11px;
  fill: var(--kyma-fg);
  text-anchor: middle;
  font-weight: 600;
  letter-spacing: -0.005em;
}
.node-sub {
  font-family: var(--kyma-font-body);
  font-size: 9.5px;
  fill: var(--kyma-fg-soft);
  text-anchor: middle;
}
.node-sub-em {
  font-family: var(--kyma-font-mono);
  font-size: 9.5px;
  fill: var(--kyma-accent);
  text-anchor: middle;
  letter-spacing: 0.04em;
}

/* ---- Storage plate ---- */
.store-plate rect {
  fill: var(--kyma-bg-soft);
  stroke: var(--kyma-rule);
  stroke-width: 1;
}
.store-title {
  font-family: var(--kyma-font-mono);
  font-size: 12.5px;
  fill: var(--kyma-fg);
  font-weight: 600;
  letter-spacing: -0.005em;
}
.store-sub {
  font-family: var(--kyma-font-body);
  font-size: 10.5px;
  fill: var(--kyma-fg-soft);
}
.store-sub-em {
  font-family: var(--kyma-font-mono);
  font-size: 10px;
  fill: var(--kyma-accent);
  letter-spacing: 0.04em;
}

/* ---- Connector lines ---- */
.link {
  stroke: var(--kyma-fg);
  stroke-width: 1;
  opacity: 0.55;
  fill: none;
}
.link-accent {
  stroke: var(--kyma-accent);
  stroke-width: 1.4;
  opacity: 0.95;
}
.link-label {
  font-family: var(--kyma-font-mono);
  font-size: 9px;
  fill: var(--kyma-accent);
  letter-spacing: 0.06em;
}
.arch-accent-fill {
  fill: var(--kyma-accent);
}
</style>
