import { defineConfig } from 'vitepress'
import { withMermaid } from 'vitepress-plugin-mermaid'
import attrs from 'markdown-it-attrs'

export default withMermaid(defineConfig({
  title: 'kyma',
  description: 'Production knowledge, as a query.',
  cleanUrls: true,

  // Excluded from the build:
  // - `superpowers/**` is a sibling of srcDir today, but kept in case layout
  //   ever changes (defensive, per spec §3.1).
  // - `README.md` is operator-facing notes on the docs site itself; not
  //   part of the published surface. It's a sibling of `index.md` so it
  //   would otherwise be picked up as a routable page.
  srcExclude: ['**/superpowers/**', 'README.md'],

  // Localhost code samples shouldn't fail the build.
  ignoreDeadLinks: [/^https?:\/\/localhost(:\d+)?/],

  sitemap: {
    hostname: 'https://getkyma.dev',
  },

  head: [
    ['link', { rel: 'icon', type: 'image/svg+xml', href: '/icons/kyma-mark.svg' }],
    ['link', { rel: 'canonical', href: 'https://getkyma.dev/' }],
    ['meta', { property: 'og:type', content: 'website' }],
    ['meta', { property: 'og:title', content: 'kyma — production knowledge, as a query' }],
    ['meta', { property: 'og:description', content: 'A unified columnar query engine for every signal your stack emits. Logs, traces, metrics, plus first-class federation to Postgres, MySQL, MongoDB. Sub-second latency over a decade of history.' }],
    ['meta', { property: 'og:url', content: 'https://getkyma.dev/' }],
    ['meta', { property: 'og:image', content: 'https://getkyma.dev/icons/kyma-mark.svg' }],
    ['meta', { name: 'twitter:card', content: 'summary' }],
    ['meta', { name: 'twitter:title', content: 'kyma — production knowledge, as a query' }],
    ['meta', { name: 'twitter:description', content: 'One columnar query engine for every signal your stack emits. By AgentcyLabs.' }],
    ['meta', { name: 'twitter:image', content: 'https://getkyma.dev/icons/kyma-mark.svg' }],
  ],

  themeConfig: {
    logo: '/icons/kyma-mark.svg',
    siteTitle: 'kyma',
    nav: [
      { text: 'Quickstart', link: '/quickstart/' },
      { text: 'Concepts', link: '/concepts/' },
      { text: 'Ingest', link: '/ingest/' },
      { text: 'Query', link: '/query/' },
      { text: 'Connectors', link: '/connectors/' },
      { text: 'Reference', link: '/reference/' },
      { text: 'Architecture', link: '/architecture/' },
      { text: 'Recipes', link: '/recipes/' },
    ],
    sidebar: {
      '/quickstart/':    [{ text: 'Quickstart',    items: [
        { text: 'Five-minute start',   link: '/quickstart/five-minute-start' },
        { text: 'First real run',      link: '/quickstart/first-real-run' },
        { text: 'Concepts cheatsheet', link: '/quickstart/concepts-cheatsheet' },
      ]}],
      '/concepts/': [{
        text: 'Concepts',
        items: [
          { text: 'What kyma is',          link: '/concepts/what-is-kyma' },
          { text: 'The five invariants',   link: '/concepts/the-five-invariants' },
          { text: 'The pruning cascade',   link: '/concepts/the-pruning-cascade' },
          { text: 'Extents and snapshots', link: '/concepts/extents-and-snapshots' },
          { text: 'Schema model',          link: '/concepts/schema-model' },
          { text: 'Dynamic and vectors',   link: '/concepts/dynamic-and-vectors' },
          { text: 'The agent loop',        link: '/concepts/the-agent-loop' },
          { text: 'Multi-source data',     link: '/concepts/multi-source-data' },
          { text: 'Retention and compaction', link: '/concepts/retention-and-compaction' },
          { text: 'Observability',         link: '/concepts/observability' },
        ],
      }],
      '/ingest/': [{
        text: 'Ingest',
        items: [
          { text: 'REST / NDJSON',           link: '/ingest/rest-ndjson' },
          { text: 'OTLP gRPC',               link: '/ingest/otlp-grpc' },
          { text: 'Kafka',                   link: '/ingest/kafka' },
          { text: 'File-drop',               link: '/ingest/file-drop' },
          { text: 'Idempotency and coercion', link: '/ingest/idempotency-and-coercion' },
        ],
      }],
      '/query/': [{
        text: 'Query',
        items: [
          { text: 'KQL',                       link: '/query/kql' },
          { text: 'SQL',                       link: '/query/sql' },
          { text: 'PromQL',                    link: '/query/promql' },
          { text: 'Arrow Flight',              link: '/query/arrow-flight' },
          { text: 'Agent endpoint',            link: '/query/agent-endpoint' },
          { text: 'Pruning and performance',   link: '/query/pruning-and-performance' },
        ],
      }],
      '/connectors/': [{
        text: 'Connectors',
        collapsed: false,
        items: [
          { text: 'Framework',           link: '/connectors/framework' },
          { text: 'Prometheus',          link: '/connectors/prometheus' },
          { text: 'Postgres',            link: '/connectors/postgres' },
          { text: 'MySQL',               link: '/connectors/mysql' },
          { text: 'MongoDB',             link: '/connectors/mongo' },
          { text: 'Multi-source data',   link: '/connectors/multi-source-data' },
        ],
      }],
      '/reference/': [{
        text: 'Reference',
        items: [
          { text: 'HTTP API',         link: '/reference/api' },
          { text: 'CLI',              link: '/reference/cli' },
          { text: 'Environment vars', link: '/reference/env' },
          { text: 'KQL functions',    link: '/reference/kql-functions' },
          { text: 'Schema mappings',  link: '/reference/schema-mappings' },
        ],
      }],
      '/architecture/': [{
        text: 'Architecture',
        items: [
          { text: 'Architecture overview', link: '/architecture/architecture' },
          { text: 'Slice roadmap',         link: '/architecture/slice-roadmap' },
          { text: 'Storage format',        link: '/architecture/storage-format' },
          { text: 'Benchmarks',            link: '/architecture/benchmarks' },
        ],
      }],
      '/recipes/': [{
        text: 'Recipes',
        items: [
          { text: 'Debug a prod incident',     link: '/recipes/debug-prod-incident' },
          { text: 'Triage an alert spike',     link: '/recipes/triage-alert-spike' },
          { text: 'Agent watching a table',    link: '/recipes/agent-watching-a-table' },
          { text: 'Ingest something custom',   link: '/recipes/ingest-something-custom' },
        ],
      }],
    },
    socialLinks: [
      { icon: 'github', link: 'https://github.com/shaked/engine' },
    ],
    search: {
      provider: 'local',
    },
    footer: {
      message: 'An open project by <a href="https://agentcylabs.com" target="_blank" rel="noopener" class="kyma-footer-link"><img src="/icons/brand/agentcy.svg" alt="" width="12" height="12" class="kyma-footer-mark"/> AgentcyLabs</a> · Apache-2.0 licensed.',
      copyright: 'Copyright © 2026 AgentcyLabs · <a href="https://agentcylabs.com" target="_blank" rel="noopener" class="kyma-footer-link">agentcylabs.com</a> · <a href="https://github.com/shaked/engine" target="_blank" rel="noopener" class="kyma-footer-link">GitHub</a>',
    },
  },

  markdown: {
    config(md) {
      // markdown-it-attrs lets authors set { .class #id data-* } on elements.
      // Restrict to id, class, and data-* — never style/onclick/etc. (XSS).
      md.use(attrs, { allowedAttributes: ['id', 'class', /^data-/] })
    },
  },

  mermaid: {
    theme: 'dark',
  },
}))
