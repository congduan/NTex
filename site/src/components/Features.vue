<script setup>
const features = [
  {
    title: '状态快照 + CoW',
    desc: '不可变状态表，微秒级快照，为增量编译与撤销/重做铺路。',
    icon: 'M4 6h16M4 12h10M4 18h16M14 6l6-3 2 3-6 3-2-3zm4 9l4-2v3l-4 2v-3z',
  },
  {
    title: '预编译 .fmt + mmap',
    desc: '内存 Dump 毫秒级完成 latex.ltx 初始化，冷编变热编。',
    icon: 'M5 8l7-5 7 5v8l-7 5-7-5V8zm14 0l-7 5m0 8v-8m0 0L5 8M12 13l7-5',
  },
  {
    title: 'Salsa 式增量求值',
    desc: '改第 50 页的一个字，前 49 页零毫秒跳过。',
    icon: 'M13 2L4 14h6l-1 8 9-12h-6l1-8z',
  },
  {
    title: 'Arena 内存池',
    desc: 'token / 节点连续分配，零 GC 暂停，长文档稳定。',
    icon: 'M4 4h16v16H4V4zm4 4h8v8H8V8zm4 4h8m-8 0v8',
  },
]
</script>

<template>
  <section class="section" id="features">
    <div class="section-head" v-reveal>
      <p class="kicker">Performance Pillars</p>
      <h2>四大性能支柱</h2>
      <p>传统 TeX 是单线程状态机；NTex 把它拆成可缓存、可并行、可预编译的现代计算图。</p>
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
