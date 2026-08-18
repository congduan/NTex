<script setup>
const strips = ['M0', 'M1', 'M2', 'M3', 'PDF', 'M4+']
const segState = ['on', 'half', 'half', 'half', 'on', '']

const milestones = [
  {
    id: 'M0',
    name: '地基',
    desc: 'workspace / CI / 基准 / TRIP / 差分工具链；RFC-1 / RFC-4 定稿',
    state: 'done',
    label: '完成',
  },
  {
    id: 'M1',
    name: '展开内核',
    desc: 'Token / InternTable / eqtb / catcode / 展开引擎 —— 核心完成，TRIP 未全绿',
    state: 'wip',
    label: '核心完成',
  },
  {
    id: 'M2',
    name: '字节码 VM',
    desc: '定长 u64 IR + 编译器 + 双轨等价（100 用例）—— 吞吐 1.12x，arena 未做',
    state: 'wip',
    label: '等价全绿',
  },
  {
    id: 'M3',
    name: '排版核心',
    desc: 'Knuth-Plass 折行 / TFM / 断页 / lig+kern / output / shipout→DVI，与 pdfTeX 逐字节一致',
    state: 'wip',
    label: '核心完成',
  },
  {
    id: 'PDF',
    name: 'DVI→PDF 输出端',
    desc: 'DVI 解析 + Type1 嵌入 + PDF 1.4 写出（M8 提前）',
    state: 'done',
    label: '可用',
  },
  {
    id: 'M4+',
    name: '后续里程碑',
    desc: '数学 + e-TeX（ETRIP）/ 增量 / 并行 / .fmt v2 / 生态',
    state: 'todo',
    label: '待实施',
  },
]
</script>

<template>
  <section class="section" id="milestones">
    <div class="section-head" v-reveal>
      <p class="kicker">Roadmap</p>
      <h2>里程碑进度</h2>
      <p>M1~M3 语义与排版核心已成，TRIP 全绿是 M1 的硬口径；数学、增量、生态仍在路上。</p>
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
