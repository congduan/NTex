<script setup>
const steps = [
  { idx: '00', title: '输入层', desc: 'VFS 文件抽象，本地 / 内存 / WASM 可互换' },
  { idx: '01', title: 'TeX VM', desc: '8B token 流 + 不可变状态快照 + 字节码求值' },
  { idx: '02', title: '增量缓存', desc: 'Salsa 式求值图，改 1 处只重算受影响段落' },
  { idx: '03', title: '布局引擎', desc: 'Knuth-Plass 折行 / 断页 / 数学 / 断字，并行' },
  { idx: '04', title: '渲染后端', desc: 'DVI→PDF 直出 · 未来 Skia / WASM' },
]
</script>

<template>
  <section class="section" id="pipeline">
    <div class="section-head" v-reveal>
      <p class="kicker">Architecture</p>
      <h2>三阶段解耦的流水线</h2>
      <p>求值、布局、渲染各司其职——纯节点流单向流出，副作用只在输出边界提交。</p>
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
