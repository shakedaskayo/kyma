<script setup lang="ts">
import { ref, onMounted, onUnmounted, computed } from 'vue'
import { withBase } from 'vitepress'

// ─────────────────────────────────────────────────────────────────────────
// Hero orbit — proper polar layout. Sources sit on the LEFT half-circle
// (angles 110°-250°, expressed CCW from 3 o'clock), consumers on the RIGHT
// half-circle (angles -70°…+70°, i.e. spanning the right half).
//
// Each node has:
//   - slug    real brand icon slug
//   - label   visible text on the pill
//   - role    'source' | 'consumer' (defines which half of the orbit)
//   - ring    'outer' | 'inner'  (radius from center)
//
// Angles are computed at render time so we don't drift between geometry and
// CSS. The viewBox is 1200x720; center at (600, 360).
// ─────────────────────────────────────────────────────────────────────────

const VIEW_W = 1200
const VIEW_H = 720
const CX = VIEW_W / 2
const CY = VIEW_H / 2
const R_INNER = 200   // inner ring radius
const R_OUTER = 300   // outer ring radius

type OrbitNode = {
  slug: string
  label: string
  role: 'source' | 'consumer'
  ring: 'outer' | 'inner'
}

const sources: OrbitNode[] = [
  // outer ring (5) — the heavy hitters
  { slug: 'kubernetes',  label: 'Kubernetes', role: 'source', ring: 'outer' },
  { slug: 'aws',         label: 'AWS',        role: 'source', ring: 'outer' },
  { slug: 'gcp',         label: 'GCP',        role: 'source', ring: 'outer' },
  { slug: 'github',      label: 'GitHub',     role: 'source', ring: 'outer' },
  { slug: 'jenkins',     label: 'Jenkins',    role: 'source', ring: 'outer' },
  // inner ring (4) — databases + telemetry
  { slug: 'postgresql',  label: 'Postgres',   role: 'source', ring: 'inner' },
  { slug: 'mongodb',     label: 'MongoDB',    role: 'source', ring: 'inner' },
  { slug: 'kafka',       label: 'Kafka',      role: 'source', ring: 'inner' },
  { slug: 'otel',        label: 'OTLP',       role: 'source', ring: 'inner' },
]

const consumers: OrbitNode[] = [
  // outer (4) — model providers
  { slug: 'anthropic',   label: 'Claude',     role: 'consumer', ring: 'outer' },
  { slug: 'openai',      label: 'OpenAI',     role: 'consumer', ring: 'outer' },
  { slug: 'ollama',      label: 'Ollama',     role: 'consumer', ring: 'outer' },
  { slug: 'mcp',         label: 'MCP',        role: 'consumer', ring: 'outer' },
  // inner (3) — humans / dashboards / pager
  { slug: 'grafana',     label: 'Grafana',    role: 'consumer', ring: 'inner' },
  { slug: 'slack',       label: 'Slack',      role: 'consumer', ring: 'inner' },
  { slug: 'pagerduty',   label: 'PagerDuty',  role: 'consumer', ring: 'inner' },
]

// Place each list of nodes evenly across an arc.
// `arcStart`, `arcEnd` are degrees (CCW from 3 o'clock).
// For sources we want the LEFT half of the circle: from 110° (top-left) down
// through 180° to 250° (bottom-left).
// For consumers we want the RIGHT half: -70° (bottom-right) up through 0° to 70° (top-right).
function place(nodes: OrbitNode[], arcStart: number, arcEnd: number, radius: number) {
  const n = nodes.length
  if (n === 0) return []
  // Spread evenly between start and end inclusive, with a small inset so the
  // first/last items don't hug the edge.
  return nodes.map((node, i) => {
    const t = n === 1 ? 0.5 : i / (n - 1)
    const angle = arcStart + t * (arcEnd - arcStart)
    const rad = (angle * Math.PI) / 180
    const x = CX + radius * Math.cos(rad)
    const y = CY + radius * Math.sin(rad)
    return { ...node, x, y, angle }
  })
}

// Sources: left arc 110° → 250° (rotated so up-left is start).
const sourcesOuter = computed(() => place(sources.filter(s => s.ring === 'outer'), 130, 230, R_OUTER))
const sourcesInner = computed(() => place(sources.filter(s => s.ring === 'inner'), 145, 215, R_INNER))
// Consumers: right arc -50° → +50°.
const consumersOuter = computed(() => place(consumers.filter(c => c.ring === 'outer'), -50, 50, R_OUTER))
const consumersInner = computed(() => place(consumers.filter(c => c.ring === 'inner'), -35, 35, R_INNER))

const allNodes = computed(() => [
  ...sourcesOuter.value, ...sourcesInner.value,
  ...consumersOuter.value, ...consumersInner.value,
])

// Build a curved path from a node to the center. The curve bows slightly
// outward (away from horizontal) so adjacent links don't collide.
function pathTo(node: { x: number; y: number; role: string }) {
  const sx = node.x
  const sy = node.y
  const ex = CX
  const ey = CY
  // Control point: midpoint, pulled vertically by ~10% of the radius
  const mx = (sx + ex) / 2
  const my = (sy + ey) / 2
  // Direction-aware bow: sources curve up, consumers curve down (or vice
  // versa) — but really we just want a gentle arc, so use a fixed offset
  // away from the connecting line's normal.
  const dx = ex - sx
  const dy = ey - sy
  const len = Math.hypot(dx, dy)
  // Perpendicular unit vector
  const nx = -dy / len
  const ny = dx / len
  // Bow away from the horizontal axis (so top icons curve up, bottom curve down)
  const bow = 60 * (sy < CY ? 1 : -1)
  const cx = mx + nx * bow
  const cy = my + ny * bow
  return `M ${sx.toFixed(1)} ${sy.toFixed(1)} Q ${cx.toFixed(1)} ${cy.toFixed(1)} ${ex.toFixed(1)} ${ey.toFixed(1)}`
}

// ─────────────────────────────────────────────────────────────────────────
// Live readout — fakes a representative kyma query result, ticks every 3s.
// ─────────────────────────────────────────────────────────────────────────
type Readout = {
  walltime_ms: number
  extents_pruned_pct: string
  bytes_scanned_mib: number
  rows_returned: number
  question: string
}
const questions = [
  'errors per service in the last hour',
  'users who hit checkout 5xx today',
  'AWS spend by team this week',
  'p99 latency on /api/checkout',
  'top customers by support tickets',
  'CI flake rate per workflow',
  'MRR by region this quarter',
]
function rand(min: number, max: number) { return Math.random() * (max - min) + min }
function genReadout(): Readout {
  const total = Math.floor(rand(11_000, 18_000))
  const pruned = Math.floor(total * rand(0.992, 0.9996))
  return {
    walltime_ms: Math.floor(rand(38, 142)),
    extents_pruned_pct: ((pruned / total) * 100).toFixed(2),
    bytes_scanned_mib: +(rand(1.2, 4.8)).toFixed(1),
    rows_returned: Math.floor(rand(12, 850)),
    question: questions[Math.floor(rand(0, questions.length))],
  }
}
const readout = ref<Readout>(genReadout())
let intervalId: ReturnType<typeof setInterval> | null = null
onMounted(() => { intervalId = setInterval(() => { readout.value = genReadout() }, 3000) })
onUnmounted(() => { if (intervalId) clearInterval(intervalId) })

// ─────────────────────────────────────────────────────────────────────────
// Use cases — what kyma replaces.
// ─────────────────────────────────────────────────────────────────────────
const useCases = [
  {
    label: 'SRE',
    title: 'Replace your observability stack',
    body: 'OTLP logs, traces, metrics in one engine — sub-second queries over years of history. Agents triage incidents from the same data your dashboards render.',
    bullets: ['Logs · traces · metrics over OTLP', 'Sub-second pruning over a decade', 'Agent-driven triage on the same data'],
    accent: '#7ed957',
  },
  {
    label: 'BI',
    title: 'Replace dashboard sprawl',
    body: 'Federate Postgres, MySQL, Mongo. Join customer rows with product events and revenue. Skip the warehouse rebuild — query the data where it lives.',
    bullets: ['Federate to operational DBs', 'CDC sync for hot tables', 'Cross-source joins in DataFusion'],
    accent: '#7eaae6',
  },
  {
    label: 'FinOps',
    title: 'Cloud spend, queryable',
    body: 'Pull AWS, GCP, Azure billing exports through the same ingest path. Group by service, team, tag. Alert on anomalies. The agent answers "why is spend up?" without a separate tool.',
    bullets: ['CUR / billing exports as kyma tables', 'Group by tag, team, env', 'Anomaly queries in KQL or SQL'],
    accent: '#f0a060',
  },
  {
    label: 'Security',
    title: 'Audit logs that survive a decade',
    body: 'Auth events, IAM changes, deploys, secret rotations — one immutable, time-partitioned store. Forensic queries that would take hours in a SIEM, in seconds.',
    bullets: ['Immutable append-only extents', 'Token index on every string column', 'KQL pivot in seconds'],
    accent: '#a78bfa',
  },
  {
    label: 'Graph',
    title: 'A native graph layer at million-node scale',
    body: 'Beyond rows and columns: kyma stores entities and relationships as a first-class graph. Trace from a Stripe customer to their auth events to the deploy that broke their checkout — in one query, in milliseconds, at organizational scale.',
    bullets: ['Entities and edges as native types', 'Cypher-style traversals over Arrow', 'Same pruning cascade applies'],
    accent: '#7ed957',
  },
  {
    label: 'Anything',
    title: 'Whatever your org runs on',
    body: 'Stripe events. GitHub Actions outcomes. Customer support tickets. Webhook payloads. If it has a timestamp and a JSON body, it goes through one POST and becomes queryable.',
    bullets: ['POST /v1/ingest takes NDJSON', 'Auto-create + schema-evolve', 'Idempotent by design'],
    accent: '#fbb6ce',
  },
]

const buildDate = new Date().toISOString().slice(0, 10)
</script>

<template>
  <div class="kyma-landing">

    <!-- ════════════════════════════════════ HERO ════════════════════════════════════ -->
    <section class="kyma-hero kyma-hero--orbit">
      <h1 class="kyma-hero-title">kyma</h1>
      <p class="kyma-tagline">The context engine for coding agents.</p>
      <p class="kyma-lede">
        Not just memory — one place your agent recalls durable, graph-aware <strong>memory</strong>,
        queries <strong>live data</strong> (logs, traces, code, every signal your org emits) in KQL/SQL,
        and walks the <strong>graph</strong> that links them. Connect it via a plugin, a CLI, or MCP —
        a local single binary that syncs to your control plane.
      </p>

      <div class="kyma-cta-row">
        <a :href="withBase('/quickstart/give-your-agent-memory')" class="kyma-cta kyma-cta--primary">Give your agent memory</a>
        <a :href="withBase('/concepts/how-kyma-works')" class="kyma-cta">How it works</a>
        <a :href="withBase('/agent/memory')" class="kyma-cta">Memory</a>
        <a href="https://github.com/shakedaskayo/kyma" target="_blank" rel="noopener" class="kyma-cta">GitHub</a>
      </div>

      <!-- The orbit — one SVG containing geometry, beams, rings; nodes overlay
           via absolute-positioned HTML for crisp pill rendering. -->
      <div class="kyma-orbit" aria-hidden="true">
        <svg
          class="kyma-orbit-svg"
          :viewBox="`0 0 ${VIEW_W} ${VIEW_H}`"
          preserveAspectRatio="xMidYMid meet"
          role="img"
        >
          <defs>
            <radialGradient id="kyma-orbit-fade" cx="50%" cy="50%" r="50%">
              <stop offset="0%"  stop-color="#7ed957" stop-opacity="0.15"/>
              <stop offset="55%" stop-color="#7ed957" stop-opacity="0.04"/>
              <stop offset="100%" stop-color="#7ed957" stop-opacity="0"/>
            </radialGradient>
            <linearGradient id="kyma-beam-grad-in" x1="0%" y1="0%" x2="100%" y2="0%">
              <stop offset="0%"   stop-color="#7ed957" stop-opacity="0.0"/>
              <stop offset="50%"  stop-color="#7ed957" stop-opacity="0.55"/>
              <stop offset="100%" stop-color="#7ed957" stop-opacity="0.85"/>
            </linearGradient>
            <linearGradient id="kyma-beam-grad-out" x1="0%" y1="0%" x2="100%" y2="0%">
              <stop offset="0%"   stop-color="#7ed957" stop-opacity="0.85"/>
              <stop offset="50%"  stop-color="#7ed957" stop-opacity="0.55"/>
              <stop offset="100%" stop-color="#7ed957" stop-opacity="0.0"/>
            </linearGradient>
            <filter id="kyma-orbit-glow">
              <feGaussianBlur stdDeviation="3" />
            </filter>
          </defs>

          <!-- Soft phosphor halo behind the center -->
          <circle :cx="CX" :cy="CY" r="220" fill="url(#kyma-orbit-fade)"/>

          <!-- Concentric rings (decorative) -->
          <circle :cx="CX" :cy="CY" :r="R_INNER" class="kyma-orbit-ring-svg" />
          <circle :cx="CX" :cy="CY" :r="R_OUTER" class="kyma-orbit-ring-svg" />

          <!-- Connection paths — one per node. Static stroke + animated
               flowing dash overlay for the data-flow sense. -->
          <g class="kyma-orbit-paths">
            <path
              v-for="(node, i) in allNodes"
              :key="`path-${node.slug}`"
              :d="pathTo(node)"
              class="kyma-orbit-path"
              :class="`kyma-orbit-path--${node.role}`"
              :style="{ '--i': i }"
            />
          </g>
          <!-- Animated flow particles riding the same paths -->
          <g class="kyma-orbit-flows">
            <path
              v-for="(node, i) in allNodes"
              :key="`flow-${node.slug}`"
              :d="pathTo(node)"
              class="kyma-orbit-flow"
              :class="`kyma-orbit-flow--${node.role}`"
              :style="{ '--i': i, '--delay': `${i * 0.15}s` }"
            />
          </g>
        </svg>

        <!-- Center mark: positioned by % of orbit container. -->
        <div class="kyma-orbit-center">
          <div class="kyma-orbit-mark">
            <img :src="withBase('/icons/kyma-mark.svg')" alt="kyma" />
            <div class="kyma-orbit-mark-pulse"></div>
          </div>
        </div>

        <!-- Node pills — absolute-positioned via inline % computed from the
             SVG coordinates so they sit exactly where the path ends. -->
        <a
          v-for="(node, i) in allNodes"
          :key="`node-${node.slug}`"
          class="kyma-orbit-node"
          :class="[`kyma-orbit-node--${node.role}`, `kyma-orbit-node--${node.ring}`]"
          :style="{
            left:  `${(node.x / VIEW_W) * 100}%`,
            top:   `${(node.y / VIEW_H) * 100}%`,
            '--i': i,
          }"
          :title="node.label"
          :href="withBase(node.role === 'source' ? '/ingest/' : '/query/')"
        >
          <img :src="withBase(`/icons/brand/${node.slug}.svg`)" :alt="node.label" width="22" height="22" />
          <span class="kyma-orbit-label">{{ node.label }}</span>
        </a>
      </div>

      <!-- Live readout — a single representative query result, refreshes every 3s -->
      <div class="kyma-readout" role="status" aria-live="polite">
        <div class="kyma-readout-question">
          <span class="kyma-readout-label">latest query</span>
          <span class="kyma-readout-q">"{{ readout.question }}"</span>
        </div>
        <dl class="kyma-readout-stats">
          <div><dt>wall</dt><dd><span class="kyma-num">{{ readout.walltime_ms }}</span> ms</dd></div>
          <div><dt>extents pruned</dt><dd><span class="kyma-num">{{ readout.extents_pruned_pct }}</span>%</dd></div>
          <div><dt>scanned</dt><dd><span class="kyma-num">{{ readout.bytes_scanned_mib }}</span> MiB</dd></div>
          <div><dt>rows</dt><dd><span class="kyma-num">{{ readout.rows_returned }}</span></dd></div>
        </dl>
      </div>
    </section>

    <!-- ════════════════════════════════════ THE BIG CLAIM ═══════════════════════════ -->
    <section class="kyma-claim">
      <p class="kyma-claim-eyebrow">// THE HOOK</p>
      <h2 class="kyma-claim-title">
        Your coding agent forgets everything. <em>kyma is the memory it keeps.</em>
      </h2>
      <p class="kyma-claim-body">
        Install one binary, wire it to your agent, and it recalls durable, graph-aware
        memory into every prompt — across sessions and machines. That's the hook. Then it
        grows: the same engine lets your agent query your <strong>live data</strong> in
        KQL or SQL, and walks the <strong>graph</strong> that ties every memory to the
        real services, repos, and traces it's about. Memory → data → graph, one surface.
      </p>
    </section>

    <!-- ════════════════════════════════════ USE CASES ══════════════════════════════ -->
    <section class="kyma-usecases">
      <div class="kyma-usecases-header">
        <p class="kyma-section-eyebrow">// BEYOND MEMORY — THE LIVE DATA</p>
        <h2 class="kyma-section-title">The same engine runs your whole stack</h2>
        <p class="kyma-section-lede">
          Memory is the front door. Underneath, kyma is one columnar engine for every
          signal your org emits — so the live data your agent reasons over is the same
          data these teams already live in. One schema, one query language, one agent.
        </p>
      </div>

      <div class="kyma-usecases-grid">
        <article
          v-for="uc in useCases"
          :key="uc.label"
          class="kyma-usecase"
          :style="{ '--accent': uc.accent }"
        >
          <span class="kyma-usecase-label">{{ uc.label }}</span>
          <h3>{{ uc.title }}</h3>
          <p>{{ uc.body }}</p>
          <ul>
            <li v-for="b in uc.bullets" :key="b">{{ b }}</li>
          </ul>
        </article>
      </div>
    </section>

    <!-- ════════════════════════════════════ CONTEXT GRAPH + RECURSIVE INTEL ═══════ -->
    <section class="kyma-graph">
      <div class="kyma-graph-text">
        <p class="kyma-section-eyebrow">// THE GRAPH — WHERE IT ALL CONNECTS</p>
        <h2 class="kyma-section-title">Where memory meets your systems.</h2>
        <p class="kyma-section-lede">
          The third layer. A native graph over the same Arrow storage, where your agent's
          <strong>memories</strong> are nodes wired to the real services, repos, and traces
          they're about — millions of entities, hundreds of millions of relationships,
          queried in milliseconds.
        </p>
        <ul class="kyma-graph-bullets">
          <li><strong>Entities + relationships as native types.</strong> Customers, deploys, services, alerts, agent runs — all linked, all queryable.</li>
          <li><strong>Cypher-style traversals.</strong> "Find every customer affected by a deploy that touched the payments service in the last 24h" — one query.</li>
          <li><strong>Same pruning cascade.</strong> Graph queries hit the same time-bound + token-index pruning that makes log queries fast.</li>
          <li><strong>Federated edges.</strong> Pull entities from Postgres, edges from MongoDB, events from OTLP — kyma joins them in the graph layer.</li>
        </ul>
      </div>

      <div class="kyma-graph-visual" aria-hidden="true">
        <svg viewBox="0 0 400 400" class="kyma-graph-svg">
          <defs>
            <radialGradient id="kyma-graph-glow" cx="50%" cy="50%" r="50%">
              <stop offset="0%" stop-color="#7ed957" stop-opacity="0.25"/>
              <stop offset="80%" stop-color="#7ed957" stop-opacity="0"/>
            </radialGradient>
          </defs>
          <circle cx="200" cy="200" r="180" fill="url(#kyma-graph-glow)"/>

          <!-- Edges -->
          <g class="kyma-graph-edges">
            <line x1="200" y1="200" x2="80"  y2="100" />
            <line x1="200" y1="200" x2="320" y2="80"  />
            <line x1="200" y1="200" x2="340" y2="220" />
            <line x1="200" y1="200" x2="80"  y2="280" />
            <line x1="200" y1="200" x2="220" y2="340" />
            <line x1="80"  y1="100" x2="320" y2="80"  />
            <line x1="80"  y1="100" x2="80"  y2="280" />
            <line x1="320" y1="80"  x2="340" y2="220" />
            <line x1="340" y1="220" x2="220" y2="340" />
            <line x1="80"  y1="280" x2="220" y2="340" />
            <line x1="80"  y1="280" x2="180" y2="240" />
            <line x1="180" y1="240" x2="220" y2="340" />
          </g>

          <!-- Nodes -->
          <g class="kyma-graph-nodes">
            <circle cx="200" cy="200" r="14" class="kyma-graph-node kyma-graph-node--center" />
            <circle cx="80"  cy="100" r="9"  class="kyma-graph-node" />
            <circle cx="320" cy="80"  r="9"  class="kyma-graph-node" />
            <circle cx="340" cy="220" r="9"  class="kyma-graph-node" />
            <circle cx="80"  cy="280" r="9"  class="kyma-graph-node" />
            <circle cx="220" cy="340" r="9"  class="kyma-graph-node" />
            <circle cx="180" cy="240" r="6"  class="kyma-graph-node kyma-graph-node--small" />
            <!-- Decorative outer ring of small distant nodes (suggests scale) -->
            <circle cx="40"  cy="60"  r="3" class="kyma-graph-node kyma-graph-node--distant" />
            <circle cx="380" cy="40"  r="3" class="kyma-graph-node kyma-graph-node--distant" />
            <circle cx="370" cy="320" r="3" class="kyma-graph-node kyma-graph-node--distant" />
            <circle cx="40"  cy="340" r="3" class="kyma-graph-node kyma-graph-node--distant" />
            <circle cx="20"  cy="200" r="3" class="kyma-graph-node kyma-graph-node--distant" />
          </g>
        </svg>
        <div class="kyma-graph-stats">
          <div><span class="kyma-num">10M+</span><span class="kyma-graph-stat-label">entities</span></div>
          <div><span class="kyma-num">100M+</span><span class="kyma-graph-stat-label">relationships</span></div>
          <div><span class="kyma-num">&lt; 100ms</span><span class="kyma-graph-stat-label">traversal p99</span></div>
        </div>
      </div>
    </section>

    <!-- ════════════════════════════════════ RECURSIVE INTELLIGENCE ════════════════ -->
    <section class="kyma-recursive">
      <p class="kyma-section-eyebrow">// THE NEW PRIMITIVE</p>
      <h2 class="kyma-recursive-title">
        <em>Recursive intelligence</em> — agents that build on each other's answers.
      </h2>
      <p class="kyma-recursive-body">
        When every signal — including <strong>the agents themselves</strong> — flows through
        one engine, agents stop being one-shot tools. They become a recursive layer:
        a triage agent's output is the next triage agent's input. A finance agent
        reads what the SRE agent observed last hour. A security agent traces an
        anomaly through the graph the BI agent just queried. The engine that answered
        the last question is the engine that asked the next one.
      </p>
      <div class="kyma-recursive-grid">
        <div class="kyma-recursive-card">
          <div class="kyma-recursive-step">01</div>
          <h3>Agents observe</h3>
          <p>Every <code>/v1/agent/ask</code> call is itself logged into kyma — question, tool calls, result. Agent runs become first-class data.</p>
        </div>
        <div class="kyma-recursive-card">
          <div class="kyma-recursive-step">02</div>
          <h3>Agents query other agents</h3>
          <p>The next agent reads <code>agent_runs</code> the same way it reads <code>otel_logs</code>. It sees what colleagues asked, found, missed.</p>
        </div>
        <div class="kyma-recursive-card">
          <div class="kyma-recursive-step">03</div>
          <h3>Insights compound</h3>
          <p>Patterns across runs surface in the graph. The org's collective troubleshooting becomes an asset that improves with use, not a stream that vanishes.</p>
        </div>
      </div>
    </section>

    <!-- ════════════════════════════════════ KQL EXAMPLE ════════════════════════════ -->
    <section class="kyma-example">
      <div class="kyma-example-text">
        <p class="kyma-section-eyebrow">// THE QUERY SURFACE</p>
        <h2 class="kyma-section-title">KQL or SQL, take your pick</h2>
        <p class="kyma-section-lede">
          Both languages parse to the same logical plan. Both run through the same
          three-level pruning cascade. Both stream Arrow over the same Flight gRPC
          endpoint. Pick whichever your tool already speaks.
        </p>
        <div class="kyma-cta-row">
          <a :href="withBase('/query/kql')" class="kyma-cta kyma-cta--primary">KQL reference</a>
          <a :href="withBase('/query/sql')" class="kyma-cta">SQL reference</a>
        </div>
      </div>
      <div class="kyma-example-code">
        <pre class="kyma-codeblock">
<span class="kyma-cm">// errors per service in the last hour</span>
otel_logs
| <span class="kyma-kw">where</span> _timestamp &gt; <span class="kyma-kw">ago</span>(<span class="kyma-num">1h</span>)
| <span class="kyma-kw">where</span> severity_text == <span class="kyma-str">"ERROR"</span>
| <span class="kyma-kw">summarize</span> n = <span class="kyma-kw">count</span>() <span class="kyma-kw">by</span> service_name
| <span class="kyma-kw">order</span> <span class="kyma-kw">by</span> n <span class="kyma-kw">desc</span>
| <span class="kyma-kw">take</span> <span class="kyma-num">5</span></pre>
      </div>
    </section>

    <!-- ════════════════════════════════════ FAST ════════════════════════════════════ -->
    <section class="kyma-cascade-section">
      <div class="kyma-cascade-stats">
        <div class="kyma-stat">
          <div class="kyma-stat-num">99.94<span class="kyma-stat-pct">%</span></div>
          <div class="kyma-stat-label">extents pruned at the catalog</div>
        </div>
        <div class="kyma-stat">
          <div class="kyma-stat-num">~ 84<span class="kyma-stat-pct">ms</span></div>
          <div class="kyma-stat-label">typical wall time, decade of data</div>
        </div>
        <div class="kyma-stat">
          <div class="kyma-stat-num">2.1<span class="kyma-stat-pct">MiB</span></div>
          <div class="kyma-stat-label">bytes actually decoded</div>
        </div>
      </div>
      <div class="kyma-cascade-text">
        <p class="kyma-section-eyebrow">// HOW IT STAYS FAST</p>
        <h2 class="kyma-section-title">Three levels of pruning before any byte is decoded.</h2>
        <p class="kyma-section-lede">
          Catalog pruning eliminates 99% of extents using time-range and column statistics.
          Extent footers eliminate 90%+ of blocks via bloom filters. Block-level posting
          lists narrow to exact rows. The decode step only touches data that matters.
        </p>
        <a :href="withBase('/concepts/the-pruning-cascade')" class="kyma-cta">Read the deep dive →</a>
      </div>
    </section>

    <!-- ════════════════════════════════════ SECTIONS ══════════════════════════════ -->
    <section class="kyma-sections-wrap">
      <p class="kyma-section-eyebrow">// THE DOCS</p>
      <h2 class="kyma-section-title">Get going</h2>
      <div class="kyma-sections">
        <a class="kyma-section-card" :href="withBase('/quickstart/give-your-agent-memory')">
          <h3>Quickstart</h3>
          <p>Give your coding agent memory in one command — local, zero infra, ~5 min.</p>
        </a>
        <a class="kyma-section-card" :href="withBase('/concepts/how-kyma-works')">
          <h3>Concepts</h3>
          <p>How it all works: memory → data → graph, the modes, dreaming, and sync.</p>
        </a>
        <a class="kyma-section-card" :href="withBase('/ingest/')">
          <h3>Ingest</h3>
          <p>OTLP gRPC, REST/NDJSON, Kafka, file-drop. One write path on the far side.</p>
        </a>
        <a class="kyma-section-card" :href="withBase('/query/')">
          <h3>Query</h3>
          <p>KQL, SQL, and the agent endpoint. Arrow Flight gRPC for zero-copy streams.</p>
        </a>
        <a class="kyma-section-card" :href="withBase('/connectors/')">
          <h3>Connectors</h3>
          <p>Postgres, MySQL, MongoDB. Federate live, or sync via CDC.</p>
        </a>
        <a class="kyma-section-card" :href="withBase('/architecture/architecture')">
          <h3>Architecture</h3>
          <p>The five invariants and how they hold the engine together.</p>
        </a>
        <a class="kyma-section-card" :href="withBase('/recipes/')">
          <h3>Recipes</h3>
          <p>Worked examples — copy, paste, adapt.</p>
        </a>
        <a class="kyma-section-card" :href="withBase('/reference/')">
          <h3>Reference</h3>
          <p>HTTP API, CLI, env vars, KQL functions, schema mappings.</p>
        </a>
      </div>
    </section>

    <!-- ════════════════════════════════════ FOOTER ═════════════════════════════════ -->
    <footer class="kyma-landing-footer">
      <p class="kyma-landing-footer-meta">
        MIT licensed · Source on
        <a href="https://github.com/shakedaskayo/kyma" target="_blank" rel="noopener">GitHub</a>
        · Built {{ buildDate }}
      </p>
    </footer>
  </div>
</template>
