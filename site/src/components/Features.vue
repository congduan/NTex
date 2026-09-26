<script setup>
const features = [
  {
    title: 'State Snapshots + CoW',
    desc: 'Immutable state tables with microsecond snapshots — the foundation for incremental compilation and undo/redo.',
    icon: 'M4 6h16M4 12h10M4 18h16M14 6l6-3 2 3-6 3-2-3zm4 9l4-2v3l-4 2v-3z',
  },
  {
    title: 'Precompiled .fmt + mmap',
    desc: 'In-memory dumps make latex.ltx initialization a millisecond operation, turning cold compiles into warm ones.',
    icon: 'M5 8l7-5 7 5v8l-7 5-7-5V8zm14 0l-7 5m0 8v-8m0 0L5 8M12 13l7-5',
  },
  {
    title: 'Salsa-style Incremental Evaluation',
    desc: 'Paragraph-level increments are live: edit one spot, re-typeset only what is affected (bit-exact under continuous editing, 4.3x in benchmarks).',
    icon: 'M13 2L4 14h6l-1 8 9-12h-6l1-8z',
  },
  {
    title: 'Arena Memory Pools',
    desc: 'Tokens and nodes are allocated from contiguous arenas — zero GC pauses, stable on long documents.',
    icon: 'M4 4h16v16H4V4zm4 4h8v8H8V8zm4 4h8m-8 0v8',
  },
]
</script>

<template>
  <section class="section" id="features">
    <div class="section-head" v-reveal>
      <p class="kicker">Performance Pillars</p>
      <h2>Four Pillars of Performance</h2>
      <p>Classic TeX is a single-threaded state machine. NTex turns it into a cacheable, parallelizable, precompilable computation graph.</p>
    </div>
    <div class="feature-grid">
      <div class="card" v-for="f in features" :key="f.title" v-reveal>
        <div class="icon">
          <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round">
            <path :d="f.icon" />
          </svg>
        </div>
        <h3>{{ f.title }}</h3>
        <p>{{ f.desc }}</p>
      </div>
    </div>
  </section>
</template>
