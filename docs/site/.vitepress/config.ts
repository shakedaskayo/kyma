import { defineConfig } from 'vitepress'
import { withMermaid } from 'vitepress-plugin-mermaid'
import attrs from 'markdown-it-attrs'

export default withMermaid(defineConfig({
  title: 'kyma',
  description: 'Production knowledge, as a query.',
  cleanUrls: true,

  // Defensive (per spec §3.1): keep any `superpowers/` content out of the build.
  // Currently a no-op — `docs/superpowers/` is a sibling of this site's srcDir,
  // so VitePress already won't scan it. Kept in case the layout ever changes.
  srcExclude: ['**/superpowers/**'],

  // Localhost references in code samples shouldn't fail the build.
  ignoreDeadLinks: [/^https?:\/\/localhost(:\d+)?/],

  head: [
    ['link', { rel: 'icon', href: '/favicon.svg' }],
    ['meta', { property: 'og:type', content: 'website' }],
    ['meta', { property: 'og:title', content: 'kyma docs' }],
  ],

  themeConfig: {
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
      '/concepts/':      [{ text: 'Concepts',      items: [] }],
      '/ingest/':        [{ text: 'Ingest',        items: [] }],
      '/query/':         [{ text: 'Query',         items: [] }],
      '/connectors/':    [{ text: 'Connectors',    items: [], collapsed: false }],
      '/reference/':     [{ text: 'Reference',     items: [] }],
      '/architecture/':  [{ text: 'Architecture',  items: [
        { text: 'Architecture overview', link: '/architecture/architecture' },
        { text: 'Benchmarks', link: '/architecture/benchmarks' },
      ]}],
      '/recipes/':       [{ text: 'Recipes',       items: [] }],
    },
    socialLinks: [
      { icon: 'github', link: 'https://github.com/shaked/engine' },
    ],
    search: {
      provider: 'local',
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
