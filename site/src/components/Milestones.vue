<script setup>
const strips = ['M0', 'M1', 'M2', 'M3', 'PDF', 'M4', 'M5', 'M8']
const segState = ['on', 'half', 'half', 'on', 'on', 'on', 'half', 'half']

const milestones = [
  {
    id: 'M0',
    name: '地基',
    desc: 'workspace / CI / 基准 / TRIP·差分工具链；RFC-1 / RFC-3 / RFC-4 定稿',
    state: 'done',
    label: '完成',
  },
  {
    id: 'M1',
    name: '展开内核',
    desc: 'Token / InternTable / eqtb / 展开引擎 + 错误恢复 —— trip.tex 全程可跑不 panic，semantic diff 收尾中',
    state: 'wip',
    label: '核心完成',
  },
  {
    id: 'M2',
    name: '字节码 VM',
    desc: '定长 u64 IR + 双轨等价（100 用例）；热路径消分配落地（展开吞吐 +61%）',
    state: 'wip',
    label: '等价全绿',
  },
  {
    id: 'M3',
    name: '排版核心',
    desc: 'Knuth-Plass 折行 / 断页 / lig+kern / output 例程 / shipout→DVI，与真实 TeX 逐字节一致',
    state: 'done',
    label: '完成',
  },
  {
    id: 'PDF',
    name: 'DVI→PDF 输出端',
    desc: 'DVI→PDF 直出 + Type1 嵌入；CM 全家族 6 族多字体嵌入全链验证',
    state: 'done',
    label: '可用',
  },
  {
    id: 'M4',
    name: '数学 + e-TeX',
    desc: '数学模式状态机 / 上下标 / 字阶 + e-TeX 全扩展 —— ETRIP 冲刺 A 组 42/42、B 组 36/36',
    state: 'done',
    label: '完成',
  },
  {
    id: 'M5',
    name: '增量计算',
    desc: '段级增量重排（阶段一~五）—— 连续编辑逐位一致，改正文 4.3x，增量墙钟翻正',
    state: 'wip',
    label: '阶段一~五完成',
  },
  {
    id: 'M8',
    name: '渲染生态',
    desc: 'vello GPU 渲染 + 真字形 · ntex-studio 实时预览工作台 · ntex-wasm A 档骨架',
    state: 'wip',
    label: '起步',
  },
]
</script>

<template>
  <section class="section" id="milestones">
    <div class="section-head" v-reveal>
      <p class="kicker">Roadmap</p>
      <h2>里程碑进度</h2>
      <p>展开、排版、数学三大核心已成，DVI 与真实 TeX 逐字节一致；增量计算与渲染生态推进中。</p>
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
