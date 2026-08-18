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
    desc: 'M0~M9 实施计划与验收标准',
    url: 'https://github.com/congduan/NTex/blob/main/plan.md',
  },
  {
    tag: 'RFC-1',
    title: 'Token 表示与内存布局',
    desc: '8B tagged union / InternTable / eqtb 版本化',
    url: 'https://github.com/congduan/NTex/blob/main/RFC-1-token.md',
  },
  {
    tag: 'RFC-3',
    title: '副作用模型',
    desc: 'VFS + 输出边界提交',
    url: 'https://github.com/congduan/NTex/blob/main/RFC-3-side-effects.md',
  },
  {
    tag: 'RFC-4',
    title: '字节码指令集',
    desc: '宏展开 VM IR（定长 u64）',
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
      <a href="#features">特性</a>
      <a href="#pipeline">架构</a>
      <a href="#milestones">里程碑</a>
      <a href="#demo">演示</a>
      <a href="#docs">文档</a>
    </div>
    <a class="btn btn-ghost" href="https://github.com/congduan/NTex" target="_blank" rel="noopener">
      GitHub
    </a>
  </nav>

  <Hero />

  <Features />

  <Pipeline />

  <Milestones />

  <Demo />

  <section class="section" id="docs">
    <div class="section-head" v-reveal>
      <p class="kicker">Documents</p>
      <h2>设计文档</h2>
    </div>
    <div class="doc-links">
      <a v-for="d in docs" :key="d.tag" class="doc-card" :href="d.url" target="_blank" rel="noopener" v-reveal>
        <span class="tag">{{ d.tag }}</span>
        <h4>{{ d.title }}</h4>
        <p>{{ d.desc }}</p>
      </a>
    </div>
  </section>

  <footer class="site">
    <a href="https://github.com/congduan/NTex" target="_blank" rel="noopener">github.com/congduan/NTex</a>
    &nbsp;·&nbsp; MIT OR Apache-2.0
  </footer>
</template>
