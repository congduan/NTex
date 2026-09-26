<script setup>
const strips = ['M0', 'M1', 'M2', 'M3', 'PDF', 'M4', 'M5', 'M8']
const segState = ['on', 'half', 'half', 'on', 'on', 'on', 'half', 'half']

const milestones = [
  {
    id: 'M0',
    name: 'Foundation',
    desc: 'workspace / CI / benchmarks / TRIP·diff tooling; RFC-1 / RFC-3 / RFC-4 finalized',
    state: 'done',
    label: 'Done',
  },
  {
    id: 'M1',
    name: 'Expansion Kernel',
    desc: 'Token / InternTable / eqtb / expansion engine + error recovery — trip.tex runs end-to-end without panics; semantic diff wrapping up',
    state: 'wip',
    label: 'Core done',
  },
  {
    id: 'M2',
    name: 'Bytecode VM',
    desc: 'Fixed-width u64 IR + dual-track equivalence (100 cases); hot-path allocation removal landed (+61% expansion throughput)',
    state: 'wip',
    label: 'All green',
  },
  {
    id: 'M3',
    name: 'Typesetting Core',
    desc: 'Knuth-Plass line breaking / page breaking / lig+kern / output routine / shipout→DVI — byte-exact against real TeX',
    state: 'done',
    label: 'Done',
  },
  {
    id: 'PDF',
    name: 'DVI→PDF Output',
    desc: 'Direct DVI→PDF + Type1/Type0 embedding; full-chain verification across the CM family',
    state: 'done',
    label: 'Usable',
  },
  {
    id: 'M4',
    name: 'Math + e-TeX',
    desc: 'Math-mode state machine / super-sub scripts / math styles + full e-TeX extensions — ETRIP sprint A 42/42, B 36/36',
    state: 'done',
    label: 'Done',
  },
  {
    id: 'M5',
    name: 'Incremental Computation',
    desc: 'Paragraph-level incremental re-typesetting (phases 1–5) — bit-exact under continuous editing, 4.3x on body-text edits',
    state: 'wip',
    label: 'Phases 1–5 done',
  },
  {
    id: 'M8',
    name: 'Rendering Ecosystem',
    desc: 'vello GPU rendering + real glyphs · ntex-studio live preview workbench · ntex-wasm runtime',
    state: 'wip',
    label: 'Early',
  },
]
</script>

<template>
  <section class="section" id="milestones">
    <div class="section-head" v-reveal>
      <p class="kicker">Roadmap</p>
      <h2>Milestone Progress</h2>
      <p>The three core pillars — expansion, typesetting, math — are complete with byte-exact DVI parity against real TeX; incremental computation and the rendering ecosystem are in progress.</p>
    </div>

    <div class="progress-strip" v-reveal>
      <div
        v-for="(seg, i) in strips"
        :key="seg"
        class="strip-seg"
        :class="segState[i]"
        :title="seg"
      ></div>
    </div>

    <div v-for="m in milestones" :key="m.id" class="ms-row" v-reveal>
      <div class="ms-id">{{ m.id }}</div>
      <div class="ms-body">
        <h4>{{ m.name }}</h4>
        <p>{{ m.desc }}</p>
      </div>
      <span class="pill" :class="m.state">
        <span class="dot"></span>{{ m.label }}
      </span>
    </div>
  </section>
</template>
