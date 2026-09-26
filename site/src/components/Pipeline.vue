<script setup>
const steps = [
  { idx: '00', title: 'Input Layer', desc: 'VFS file abstraction — local, in-memory, and WASM backends are interchangeable' },
  { idx: '01', title: 'TeX VM', desc: '8-byte token stream + immutable state snapshots + bytecode evaluation' },
  { idx: '02', title: 'Incremental Cache', desc: 'Salsa-style evaluation graph — change one paragraph, recompute only what is affected' },
  { idx: '03', title: 'Layout Engine', desc: 'Knuth-Plass line breaking, page breaking, math, hyphenation — parallel-ready' },
  { idx: '04', title: 'Rendering Backends', desc: 'Direct DVI→PDF · vello GPU rendering · WASM runtime shipped' },
]
</script>

<template>
  <section class="section" id="pipeline">
    <div class="section-head" v-reveal>
      <p class="kicker">Architecture</p>
      <h2>A Three-Stage Decoupled Pipeline</h2>
      <p>Evaluation, layout, and rendering each do one job — a pure node stream flows one way, side effects commit only at output boundaries.</p>
    </div>
    <div class="pipeline" v-reveal>
      <div class="pipe-step" v-for="s in steps" :key="s.idx">
        <span class="idx">{{ s.idx }}</span>
        <h3>{{ s.title }}</h3>
        <p>{{ s.desc }}</p>
      </div>
    </div>
  </section>
</template>
