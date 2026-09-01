import { defineConfig } from 'vitepress'
import { withMermaid } from 'vitepress-plugin-mermaid'
import attrs from 'markdown-it-attrs'

export default withMermaid(defineConfig({
  title: 'pensieve',
  description: 'The context engine for coding agents — durable graph-aware memory, live data in KQL/SQL, and the graph that links them. One local binary, zero infra.',
  cleanUrls: true,

  // Light is the default; the dark-mode toggle still works and persists per-visitor.
  appearance: { initialValue: 'light' },

  // Served from GitHub Pages at shakedaskayo.github.io/pensieve/.
  base: '/pensieve/',

  // `README.md` is operator-facing notes on the docs site itself, not part of
  // the published surface. Sibling of `index.md`, so without exclusion it
  // would otherwise be picked up as a routable page.
  srcExclude: ['README.md'],

  // Localhost code samples shouldn't fail the build.
  ignoreDeadLinks: [/^https?:\/\/localhost(:\d+)?/],

  sitemap: {
    // Must include the base — VitePress sitemap URLs are page paths without it.
    hostname: 'https://shakedaskayo.github.io/pensieve/',
  },

  head: [
    ['link', { rel: 'icon', type: 'image/svg+xml', href: '/pensieve/icons/pensieve-mark.svg' }],
    ['link', { rel: 'icon', type: 'image/x-icon', href: '/pensieve/favicon.ico', sizes: 'any' }],
    ['link', { rel: 'canonical', href: 'https://shakedaskayo.github.io/pensieve/' }],
    ['meta', { property: 'og:type', content: 'website' }],
    ['meta', { property: 'og:title', content: 'pensieve — production knowledge, as a query' }],
    ['meta', { property: 'og:description', content: 'A unified columnar query engine for every signal your stack emits. Logs, traces, metrics, plus first-class federation to Postgres, MySQL, MongoDB. Sub-second latency over a decade of history.' }],
    ['meta', { property: 'og:url', content: 'https://shakedaskayo.github.io/pensieve/' }],
    ['meta', { property: 'og:image', content: 'https://shakedaskayo.github.io/pensieve/og.png' }],
    ['meta', { property: 'og:image:width', content: '1200' }],
    ['meta', { property: 'og:image:height', content: '630' }],
    ['meta', { name: 'twitter:card', content: 'summary_large_image' }],
    ['meta', { name: 'twitter:title', content: 'pensieve — production knowledge, as a query' }],
    ['meta', { name: 'twitter:description', content: 'One columnar query engine for every signal your stack emits.' }],
    ['meta', { name: 'twitter:image', content: 'https://shakedaskayo.github.io/pensieve/og.png' }],
  ],

  themeConfig: {
    logo: '/icons/pensieve-mark.svg',
    siteTitle: 'pensieve',
    // Five scannable top-level entries. The capability pages (ingest/query/
    // data-sources/agent/embed) collapse into a "Guides" dropdown and the
    // learning material into "Learn", so the two things an operator looks for —
    // getting started and shipping to production — stay as prominent standalone
    // items (Quickstart, Deploy) rather than getting lost in a long flat bar.
    nav: [
      { text: 'Quickstart', link: '/quickstart/' },
      {
        text: 'Guides',
        items: [
          { text: 'Ingest data',     link: '/ingest/' },
          { text: 'Query',           link: '/query/' },
          { text: 'Data sources',    link: '/data-sources/' },
          { text: 'Agent & memory',  link: '/agent/' },
          { text: 'Embed (React SDK)', link: '/embed/' },
        ],
      },
      { text: 'Deploy', link: '/deploy/' },
      { text: 'Reference', link: '/reference/' },
      {
        text: 'Learn',
        items: [
          { text: 'Concepts',     link: '/concepts/' },
          { text: 'Architecture', link: '/architecture/' },
          { text: 'Use cases',    link: '/use-cases/' },
        ],
      },
    ],
    sidebar: {
      '/quickstart/':    [{ text: 'Quickstart',    items: [
        { text: 'Give your agent memory', link: '/quickstart/give-your-agent-memory' },
        { text: 'Five-minute start',   link: '/quickstart/five-minute-start' },
        { text: 'First real run',      link: '/quickstart/first-real-run' },
        { text: 'Concepts cheatsheet', link: '/quickstart/concepts-cheatsheet' },
      ]}],
      '/concepts/': [{
        text: 'Concepts',
        items: [
          { text: 'What pensieve is',          link: '/concepts/what-is-pensieve' },
          { text: 'How pensieve works',        link: '/concepts/how-pensieve-works' },
          { text: 'The five invariants',   link: '/concepts/the-five-invariants' },
          { text: 'The pruning cascade',   link: '/concepts/the-pruning-cascade' },
          { text: 'Extents and snapshots', link: '/concepts/extents-and-snapshots' },
          { text: 'Schema model',          link: '/concepts/schema-model' },
          { text: 'Dynamic and vectors',   link: '/concepts/dynamic-and-vectors' },
          { text: 'The agent loop',        link: '/concepts/the-agent-loop' },
          { text: 'Local & cluster mode',  link: '/concepts/local-and-cluster-mode' },
          { text: 'Sync',                  link: '/concepts/sync' },
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
          { text: 'Graph',                     link: '/query/graph' },
          { text: 'Pruning and performance',   link: '/query/pruning-and-performance' },
        ],
      }],
      '/data-sources/': [{
        text: 'Data Sources',
        collapsed: false,
        items: [
          { text: 'Framework',           link: '/data-sources/framework' },
          { text: 'OAuth sources',    link: '/data-sources/oauth' },
          { text: 'GitHub',              link: '/data-sources/github' },
          { text: 'GitLab',              link: '/data-sources/gitlab' },
          { text: 'Bitbucket',           link: '/data-sources/bitbucket' },
          { text: 'Notion',              link: '/data-sources/notion' },
          { text: 'Google Drive',        link: '/data-sources/googledrive' },
          { text: 'Gmail',               link: '/data-sources/gmail' },
          { text: 'Slack',               link: '/data-sources/slack' },
          { text: 'Jira',                link: '/data-sources/jira' },
          { text: 'Confluence',          link: '/data-sources/confluence' },
          { text: 'Prometheus',          link: '/data-sources/prometheus' },
          { text: 'Microsoft Fabric',    link: '/data-sources/msfabric' },
          { text: 'Postgres',            link: '/data-sources/postgres' },
          { text: 'MySQL',               link: '/data-sources/mysql' },
          { text: 'MongoDB',             link: '/data-sources/mongo' },
          { text: 'Multi-source data',   link: '/data-sources/multi-source-data' },
        ],
      }],
      '/agent/': [{
        text: 'Agent',
        collapsed: false,
        items: [
          { text: 'Overview',            link: '/agent/' },
          { text: 'Connect your agent',  link: '/agent/connect' },
          { text: 'Connect via CLI',     link: '/agent/connect-from-cli' },
          { text: 'Coding agents',       link: '/agent/coding-agents' },
          { text: 'Agentic Memory',      link: '/agent/memory' },
          { text: 'Dreaming',            link: '/agent/dreaming' },
          { text: 'Brain repos',         link: '/agent/brains' },
          { text: 'Workers & nodes',     link: '/agent/workers' },
          { text: 'Engines',             link: '/agent/engines' },
          { text: 'Claude Code engine',  link: '/agent/claude-cli' },
          { text: 'Claude Code plugin',  link: '/agent/claude-code-plugin' },
          { text: 'Skills',              link: '/agent/skills' },
        ],
      }],
      '/deploy/': [{
        text: 'Production deployment',
        items: [
          { text: 'Overview',            link: '/deploy/' },
          { text: 'pensieve deploy (CLI)',   link: '/deploy/cli' },
          { text: 'Terraform',           link: '/deploy/terraform' },
          { text: 'Pulumi',              link: '/deploy/pulumi' },
          { text: 'Helm',                link: '/deploy/helm' },
          { text: 'Kubernetes (EKS)',    link: '/deploy/kubernetes' },
        ],
      }],

      '/reference/': [{
        text: 'Reference',
        items: [
          { text: 'HTTP API',         link: '/reference/api' },
          { text: 'CLI',              link: '/reference/cli' },
          { text: 'MCP',              link: '/reference/mcp' },
          { text: 'Environment vars', link: '/reference/env' },
          { text: 'KQL functions',    link: '/reference/kql-functions' },
          { text: 'Schema mappings',  link: '/reference/schema-mappings' },
          { text: 'Migrating from kyma', link: '/reference/migrating-from-kyma' },
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
      '/embed/': [{
        text: 'Embed (React SDK)',
        collapsed: false,
        items: [
          { text: 'Overview',              link: '/embed/' },
          { text: 'Quickstart',            link: '/embed/quickstart' },
          { text: 'Authentication',        link: '/embed/authentication' },
          { text: 'CORS',                  link: '/embed/cors' },
          { text: 'Theming',               link: '/embed/theming' },
          { text: 'Components',            link: '/embed/components' },
          { text: 'Headless hooks',        link: '/embed/hooks' },
          { text: 'Server compatibility',  link: '/embed/server-compat' },
          { text: 'Versioning & release',  link: '/embed/versioning' },
        ],
      }],
      '/use-cases/': [{
        text: 'Use cases',
        items: [
          { text: 'Overview',                        link: '/use-cases/' },
          { text: 'Remember across sessions',        link: '/use-cases/remember-across-sessions' },
          { text: 'Debug from your editor',          link: '/use-cases/debug-from-your-editor' },
          { text: 'Ask your stack in English',       link: '/use-cases/ask-your-stack' },
        ],
      }],
    },
    socialLinks: [
      { icon: 'github', link: 'https://github.com/shakedaskayo/pensieve' },
    ],
    search: {
      provider: 'local',
    },
    footer: {
      message: 'An open-source project · MIT licensed.',
      copyright: 'Copyright © 2026 the pensieve authors · <a href="https://github.com/shakedaskayo/pensieve" target="_blank" rel="noopener" class="pensieve-footer-link">GitHub</a>',
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
