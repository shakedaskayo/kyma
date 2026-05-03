import { defineConfig } from 'vitepress'
import { withMermaid } from 'vitepress-plugin-mermaid'
import attrs from 'markdown-it-attrs'

export default withMermaid(defineConfig({
  title: 'kyma',
  description: 'Production knowledge, as a query.',
  cleanUrls: true,

  // Per spec §3.1: `docs/superpowers/` stays out of the build.
  srcExclude: ['**/superpowers/**'],

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
      '/quickstart/':    [{ text: 'Quickstart',    items: [] }],
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
  },

  markdown: {
    config(md) {
      // The agentcy docs use markdown-it-attrs for { .class .id } inline attrs.
      md.use(attrs)
    },
  },

  mermaid: {
    theme: 'dark',
  },
}))
