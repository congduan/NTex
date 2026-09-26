<script setup>
import Hero from './components/Hero.vue'
import Features from './components/Features.vue'
import Pipeline from './components/Pipeline.vue'
import Milestones from './components/Milestones.vue'
import Demo from './components/Demo.vue'

const docs = [
  {
    tag: 'PLAN',
    title: 'plan.md',
    desc: 'M0–M9 implementation plan with acceptance criteria',
    url: 'https://github.com/congduan/NTex/blob/main/plan.md',
  },
  {
    tag: 'RFC-1',
    title: 'Token Representation',
    desc: '8-byte tagged union / InternTable / versioned eqtb',
    url: 'https://github.com/congduan/NTex/blob/main/RFC-1-token.md',
  },
  {
    tag: 'RFC-3',
    title: 'Side-Effect Model',
    desc: 'VFS abstraction + commit-at-boundary semantics',
    url: 'https://github.com/congduan/NTex/blob/main/RFC-3-side-effects.md',
  },
  {
    tag: 'RFC-4',
    title: 'Bytecode Instruction Set',
    desc: 'Macro-expansion VM IR (fixed-width u64)',
    url: 'https://github.com/congduan/NTex/blob/main/RFC-4-bytecode.md',
  },
]
</script>

<template>
  <nav>
    <a class="brand" href="#top">
      <span class="brand-mark">N</span>
      NTex
    </a>
    <div class="nav-links">
      <a href="#features">Features</a>
      <a href="#pipeline">Architecture</a>
      <a href="#milestones">Roadmap</a>
      <a href="#demo">Demo</a>
      <a href="#docs">Docs</a>
    </div>
    <a class="btn btn-ghost" href="https://github.com/congduan/NTex" target="_blank" rel="noopener">
      GitHub
    </a>
  </nav>

  <main id="top">
    <Hero />
    <Features />
    <Pipeline />
    <Milestones />
    <Demo />

    <section class="section" id="docs">
      <div class="section-head" v-reveal>
        <p class="kicker">Documentation</p>
        <h2>Design Docs</h2>
        <p>Roadmap, architecture decision records, and the full implementation plan — all in the repository.</p>
      </div>
      <div class="docs-grid" v-reveal>
        <a class="card doc" v-for="d in docs" :key="d.tag" :href="d.url" target="_blank" rel="noopener">
          <span class="doc-tag">{{ d.tag }}</span>
          <h3>{{ d.title }}</h3>
          <p>{{ d.desc }}</p>
        </a>
      </div>
    </section>
  </main>

  <footer>
    <span>NTex — a modern typesetting kernel in Rust. Not affiliated with the TeX Users Group.</span>
    <span>
      <a href="https://github.com/congduan/NTex" target="_blank" rel="noopener">GitHub</a>
      · <a href="https://arxiv.org/abs/1706.03762" target="_blank" rel="noopener">Sample paper</a>
    </span>
  </footer>
</template>
