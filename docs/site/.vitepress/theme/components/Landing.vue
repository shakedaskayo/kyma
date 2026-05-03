<script setup lang="ts">
// D4 replaces this with the real landing (hero, animated logo, featured recipes).
</script>

<template>
  <div class="landing-placeholder">
    <h1 class="hero-title">kyma</h1>
    <p class="hero-tagline">Production knowledge, as a query.</p>
    <div class="feature-grid">
      <a href="/quickstart/" class="feature-card">
        <h3>Quickstart</h3>
        <p>Five minutes from <code>docker compose up</code> to your first query.</p>
      </a>
      <a href="/concepts/" class="feature-card">
        <h3>Concepts</h3>
        <p>How the engine thinks: pruning, extents, the agent loop.</p>
      </a>
      <a href="/connectors/" class="feature-card">
        <h3>Connectors</h3>
        <p>Postgres, MySQL, MongoDB, OTLP, Kafka, file-drop. One query surface.</p>
      </a>
    </div>
  </div>
</template>

<style scoped>
.landing-placeholder { padding: 2rem 0; text-align: center; }
.hero-title {
  font-size: 4rem;
  background: var(--vp-home-hero-name-background);
  -webkit-background-clip: text;
  background-clip: text;
  color: transparent;
  margin: 0;
}
.hero-tagline { font-size: 1.5rem; opacity: 0.85; margin-bottom: 2rem; }
</style>
