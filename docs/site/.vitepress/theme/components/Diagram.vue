<script setup lang="ts">
import { computed } from 'vue'

const props = defineProps<{
  name: string
  caption?: string
  alt?: string
}>()

// Vite glob import — bundles every SVG once, looked up at runtime by name.
// `query: '?raw'` returns the SVG source as a string so v-html can inline it,
// which is what makes CSS custom-property inheritance work.
const modules = import.meta.glob('/public/diagrams/*.svg', {
  query: '?raw',
  import: 'default',
  eager: true,
}) as Record<string, string>

const svg = computed(() => {
  const key = `/public/diagrams/${props.name}.svg`
  const raw = modules[key]
  if (!raw) return `<!-- diagram not found: ${props.name} -->`
  if (props.alt) {
    // Replace <title>...</title> with provided alt text for screen-reader override.
    return raw.replace(/<title[^>]*>[\s\S]*?<\/title>/, `<title>${props.alt}</title>`)
  }
  return raw
})
</script>

<template>
  <figure class="diagram">
    <!-- Safe: SVG content is build-time bundled via the eager glob above, not user-supplied. -->
    <div class="diagram__svg" v-html="svg" />
    <figcaption v-if="caption">{{ caption }}</figcaption>
  </figure>
</template>

<style scoped>
.diagram {
  margin: 1.5rem 0;
}
.diagram__svg :deep(svg) {
  width: 100%;
  height: auto;
  display: block;
}
.diagram figcaption {
  margin-top: 0.5rem;
  text-align: center;
  font-size: 0.875rem;
  color: var(--vp-c-text-2);
}
</style>
