<script setup>
import { ref, onMounted } from 'vue'

const base = import.meta.env.BASE_URL
const pdfUrl = base + 'demo.pdf'
const texUrl = base + 'demo.tex'

const tab = ref('pdf')
const src = ref('')
const loadError = ref(false)

onMounted(async () => {
  try {
    const res = await fetch(texUrl)
    if (!res.ok) throw new Error(`HTTP ${res.status}`)
    src.value = await res.text()
  } catch (e) {
    loadError.value = true
  }
})
</script>

<template>
  <section class="section" id="demo">
    <div class="section-head" v-reveal>
      <p class="kicker">Live Output</p>
      <h2>引擎自举：.tex → PDF</h2>
      <p>
        下方 PDF 由 NTex 自己渲染：TFM 真实度量、Knuth-Plass 最优折行、自动断页，
        经正式 DVI→PDF 后端输出。每次部署时由 CI 重新生成，与仓库代码同步。
      </p>
    </div>

    <div class="demo-shell" v-reveal>
      <div class="demo-tabs">
        <span class="demo-tab" :class="{ active: tab === 'pdf' }" @click="tab = 'pdf'">demo.pdf</span>
        <span class="demo-tab" :class="{ active: tab === 'tex' }" @click="tab = 'tex'">demo.tex</span>
      </div>

      <div class="demo-body">
        <div class="demo-src" :style="{ display: tab === 'tex' ? 'block' : 'none' }">
          <pre v-if="src">{{ src }}</pre>
          <pre v-else-if="loadError">无法加载 demo.tex（本地预览请先构建站点）。</pre>
          <pre v-else>加载中…</pre>
        </div>
        <div class="demo-pdf" :style="{ display: tab === 'pdf' ? 'block' : 'none' }">
          <iframe :src="pdfUrl" title="demo.pdf 预览"></iframe>
        </div>
      </div>
    </div>
  </section>
</template>
