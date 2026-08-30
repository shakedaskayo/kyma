<!--
  PensievePruningCascade
  ------------------
  Visual proof of the three-level pruning cascade. Shows data volume
  shrinking dramatically through each filter — from "all extents" to
  "exact rows to decode" — with phosphor-green stage callouts.
-->
<script setup lang="ts">
defineProps<{
  caption?: string
}>()
</script>

<template>
  <figure class="pensieve-cascade-svg">
    <svg viewBox="0 0 1040 520" role="img"
         :aria-label="caption ?? 'Three-level pruning cascade: catalog, extent, block'"
         xmlns="http://www.w3.org/2000/svg">

      <defs>
        <marker id="cascade-arrow" viewBox="0 0 10 10" refX="9" refY="5"
                markerWidth="10" markerHeight="10" orient="auto">
          <path d="M0 0 L10 5 L0 10 Z" class="cascade-accent-fill"/>
        </marker>

        <pattern id="cascade-dots" patternUnits="userSpaceOnUse" width="8" height="8">
          <circle cx="2" cy="2" r="1" fill="currentColor" opacity="0.18"/>
        </pattern>
      </defs>

      <!-- Title -->
      <text x="520" y="32" class="cascade-title" text-anchor="middle">
        a single query, traveling through the cascade
      </text>
      <text x="520" y="50" class="cascade-subtitle" text-anchor="middle">
        otel_logs | where service_name == "auth-svc" | where _timestamp > ago(1h)
      </text>

      <!-- ─────── Stage 0: Input universe ─────── -->
      <g class="stage-0" transform="translate(0, 80)">
        <text x="60" y="20" class="stage-num">/0</text>
        <text x="60" y="40" class="stage-label">UNIVERSE</text>
        <text x="60" y="56" class="stage-meta">14,291 extents · 14.7 GiB</text>

        <!-- Stylized data block -->
        <g transform="translate(220, 0)">
          <rect width="640" height="48" fill="url(#cascade-dots)" />
          <rect width="640" height="48" class="stage-frame" />
          <text x="320" y="29" class="stage-volume" text-anchor="middle">100% — every extent · every byte</text>
        </g>

        <!-- Right percentage -->
        <text x="990" y="30" class="stage-pct" text-anchor="end">100%</text>
      </g>

      <!-- Hairline arrow -->
      <line x1="540" y1="138" x2="540" y2="166" class="cascade-link"
            marker-end="url(#cascade-arrow)" />
      <text x="555" y="156" class="cascade-link-label">catalog query</text>

      <!-- ─────── Stage 1: Catalog pruning ─────── -->
      <g class="stage-1" transform="translate(0, 180)">
        <text x="60" y="20" class="stage-num">/1</text>
        <text x="60" y="40" class="stage-label">CATALOG</text>
        <text x="60" y="56" class="stage-meta">8 extents survive · time + service overlap</text>

        <g transform="translate(370, 0)">
          <rect width="340" height="40" fill="url(#cascade-dots)" />
          <rect width="340" height="40" class="stage-frame stage-frame-active" />
          <text x="170" y="25" class="stage-volume stage-volume-active" text-anchor="middle">
            8 / 14,291 extents
          </text>
        </g>

        <text x="990" y="30" class="stage-pct stage-pct-elim" text-anchor="end">99.94% eliminated</text>
      </g>

      <!-- arrow -->
      <line x1="540" y1="220" x2="540" y2="246" class="cascade-link"
            marker-end="url(#cascade-arrow)" />
      <text x="555" y="237" class="cascade-link-label">range-GET footers</text>

      <!-- ─────── Stage 2: Extent footer pruning ─────── -->
      <g class="stage-2" transform="translate(0, 260)">
        <text x="60" y="20" class="stage-num">/2</text>
        <text x="60" y="40" class="stage-label">EXTENT</text>
        <text x="60" y="56" class="stage-meta">87 blocks survive · bloom + per-block stats</text>

        <g transform="translate(450, 0)">
          <rect width="220" height="32" fill="url(#cascade-dots)" />
          <rect width="220" height="32" class="stage-frame stage-frame-active" />
          <text x="110" y="21" class="stage-volume stage-volume-active" text-anchor="middle">
            87 blocks
          </text>
        </g>

        <text x="990" y="30" class="stage-pct stage-pct-elim" text-anchor="end">+92.4% eliminated</text>
      </g>

      <!-- arrow -->
      <line x1="540" y1="292" x2="540" y2="318" class="cascade-link"
            marker-end="url(#cascade-arrow)" />
      <text x="555" y="309" class="cascade-link-label">posting-list intersect</text>

      <!-- ─────── Stage 3: Block / index pruning ─────── -->
      <g class="stage-3" transform="translate(0, 332)">
        <text x="60" y="20" class="stage-num">/3</text>
        <text x="60" y="40" class="stage-label">BLOCK</text>
        <text x="60" y="56" class="stage-meta">14,083 rows survive · exact rows</text>

        <g transform="translate(500, 0)">
          <rect width="120" height="26" fill="url(#cascade-dots)" />
          <rect width="120" height="26" class="stage-frame stage-frame-active" />
          <text x="60" y="18" class="stage-volume stage-volume-active" text-anchor="middle">
            14,083 rows
          </text>
        </g>

        <text x="990" y="30" class="stage-pct stage-pct-elim" text-anchor="end">+74.1% eliminated</text>
      </g>

      <!-- arrow -->
      <line x1="540" y1="358" x2="540" y2="384" class="cascade-link"
            marker-end="url(#cascade-arrow)" />
      <text x="555" y="375" class="cascade-link-label">decode</text>

      <!-- ─────── Stage 4: Decode + execute ─────── -->
      <g class="stage-4" transform="translate(0, 396)">
        <text x="60" y="20" class="stage-num">/4</text>
        <text x="60" y="40" class="stage-label">DECODE</text>
        <text x="60" y="56" class="stage-meta">Arrow → DataFusion → Flight stream</text>

        <g transform="translate(540, 0)">
          <rect width="40" height="22" class="stage-frame stage-frame-final" />
        </g>
      </g>

      <!-- Bottom summary bar -->
      <g transform="translate(0, 470)">
        <line x1="60" y1="0" x2="980" y2="0" class="cascade-summary-rule"/>
        <text x="60" y="20" class="cascade-summary-label">RESULT</text>
        <text x="200" y="20" class="cascade-summary-value">
          <tspan class="cascade-num">2.1 MiB</tspan> read · <tspan class="cascade-num">14,083 rows</tspan> · <tspan class="cascade-num">84 ms</tspan> wall
        </text>
        <text x="980" y="20" class="cascade-summary-final" text-anchor="end">
          ~ <tspan class="cascade-num">0.0143%</tspan> of bytes touched
        </text>
      </g>
    </svg>
    <figcaption v-if="caption">{{ caption }}</figcaption>
  </figure>
</template>

<style scoped>
.pensieve-cascade-svg {
  margin: 1.5rem 0 2rem;
  color: var(--pensieve-fg);
}
.pensieve-cascade-svg svg {
  width: 100%;
  height: auto;
  display: block;
  background: var(--pensieve-bg-soft);
  border: 1px solid var(--pensieve-rule-soft);
}
.pensieve-cascade-svg figcaption {
  margin-top: 0.75rem;
  font-size: 0.78rem;
  letter-spacing: 0.04em;
  color: var(--pensieve-muted);
  text-align: center;
  font-family: var(--pensieve-font-mono);
}

.cascade-title {
  font-family: var(--pensieve-font-mono);
  font-size: 13px;
  fill: var(--pensieve-fg);
  font-weight: 600;
}
.cascade-subtitle {
  font-family: var(--pensieve-font-mono);
  font-size: 10.5px;
  fill: var(--pensieve-muted);
  letter-spacing: -0.005em;
}

.stage-num {
  font-family: var(--pensieve-font-mono);
  font-size: 10px;
  fill: var(--pensieve-muted);
  letter-spacing: 0.1em;
}
.stage-label {
  font-family: var(--pensieve-font-mono);
  font-size: 13px;
  fill: var(--pensieve-fg);
  font-weight: 600;
  letter-spacing: 0.04em;
}
.stage-meta {
  font-family: var(--pensieve-font-body);
  font-size: 10px;
  fill: var(--pensieve-fg-soft);
}

.stage-frame {
  fill: transparent;
  stroke: var(--pensieve-rule);
  stroke-width: 1;
}
.stage-frame-active {
  stroke: var(--pensieve-accent);
  stroke-width: 1.2;
}
.stage-frame-final {
  fill: var(--pensieve-accent);
  stroke: var(--pensieve-accent);
}

.stage-volume {
  font-family: var(--pensieve-font-mono);
  font-size: 10.5px;
  fill: var(--pensieve-fg-soft);
  font-weight: 500;
}
.stage-volume-active {
  fill: var(--pensieve-fg);
}

.stage-pct {
  font-family: var(--pensieve-font-mono);
  font-size: 14px;
  fill: var(--pensieve-muted);
  font-weight: 600;
  letter-spacing: -0.01em;
}
.stage-pct-elim {
  fill: var(--pensieve-accent);
}

.cascade-link {
  stroke: var(--pensieve-accent);
  stroke-width: 1.2;
  opacity: 0.85;
}
.cascade-link-label {
  font-family: var(--pensieve-font-mono);
  font-size: 9px;
  fill: var(--pensieve-accent);
  letter-spacing: 0.04em;
}
.cascade-accent-fill {
  fill: var(--pensieve-accent);
}

.cascade-summary-rule {
  stroke: var(--pensieve-rule-soft);
  stroke-width: 1;
}
.cascade-summary-label {
  font-family: var(--pensieve-font-mono);
  font-size: 11px;
  fill: var(--pensieve-muted);
  letter-spacing: 0.14em;
}
.cascade-summary-value {
  font-family: var(--pensieve-font-mono);
  font-size: 11px;
  fill: var(--pensieve-fg);
}
.cascade-summary-final {
  font-family: var(--pensieve-font-mono);
  font-size: 11px;
  fill: var(--pensieve-fg);
}
.cascade-num {
  fill: var(--pensieve-accent);
  font-weight: 600;
}
</style>
