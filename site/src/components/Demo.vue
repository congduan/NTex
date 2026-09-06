<script setup>
import { ref, watch, onMounted, onBeforeUnmount } from 'vue'

// 预览逻辑（2026-09-06 起为 WASM 实时形态）：
// 引擎经 ntex-wasm（B 档：compile_document → Document 句柄 → render_page 软光栅）
// 完整跑在浏览器里——编辑 250ms 防抖全链重排（展开→折行→断页→DVI→光栅），
// 翻页/调 dpi/切 overlay 只重渲染不重排；静态 demo.pdf（CI 产物）保留为对照与回落。
// wasm 绑定产物位于 src/wasm/（拷贝自 crates/ntex-wasm/www/pkg，随引擎发布更新）。
// 真字形：public/fonts/ 的 LM OTF（拷贝自 tauri ui/fonts/）fetch 注入 + set_glyphs 开关；
// 未注入/未开启时按引擎契约逐字符回落占位方框。

const base = import.meta.env.BASE_URL
const pdfUrl = base + 'demo.pdf'
const texUrl = base + 'demo.tex'

const DEBOUNCE_MS = 250

// 真字形：Latin Modern OTF（GUST Font License，与 CM 同源度量一致）经
// set_glyph_font 注入 wasm 进程级注册表；映射口径同 tauri ui/fonts/README.md
const GLYPH_FONTS = [
  ['cmr10', `${base}fonts/lmroman10-regular.otf`],
  ['cmbx10', `${base}fonts/lmroman10-bold.otf`],
  ['cmti10', `${base}fonts/lmroman10-italic.otf`],
]

const tab = ref('live') // live = WASM 实时预览；pdf = 静态 CI 产物
const engineState = ref('loading') // loading | ready | error
const engineMsg = ref('')
const src = ref('')
const initialSrc = ref('')
const transcript = ref('')
const status = ref('')
const statusErr = ref(false)
const pageNo = ref(0)
const pageCount = ref(0)
const dpi = ref(144)
const debug = ref(false)
const glyphs = ref(true)
const fontsReady = ref(false)
const noPage = ref(false)
const canvasRef = ref(null)
const previewRef = ref(null)

// 预览缩放：显示宽 = 96dpi 等比基准 × zoom（纯 CSS 宽度，不重排版不重渲染）；
// dpi 滑杆只改渲染分辨率（清晰度），与显示尺寸解耦
const ZOOM_MIN = 0.5
const ZOOM_MAX = 8
const zoom = ref(1)
let pan = null

// wasm 句柄与 Document 不进 Vue 响应式（避免 proxy 包裹 wasm-bindgen 类实例）
let engine = null
let doc = null
let compileTimer = 0

// —— 引擎装载：src/wasm 走 Vite 打包管线（new URL 资产模式自动携带 .wasm），
//    动态 import 懒加载，不阻塞首屏 ——
async function loadEngine() {
  try {
    engine = await import('../wasm/ntex_wasm.js')
    await engine.default()
    engineState.value = 'ready'
    engineMsg.value = engine.engine_version()
    if (!src.value) src.value = engine.demo_tex()
    // 字体注入与首排并行（先方框，字形就绪后重渲染；口径同 tauri 工作台）
    const fonts = loadFonts()
    compileNow()
    await fonts
    if (fontsReady.value && glyphs.value && doc) {
      doc.set_glyphs(true)
      status.value = `渲染 ${renderCurrent().toFixed(1)}ms（真字形）`
    }
  } catch (e) {
    engineState.value = 'error'
    engineMsg.value = String(e.message || e)
    tab.value = 'pdf'
  }
}

// —— 字体注入：fetch LM OTF → set_glyph_font（进程级，一次注册持续生效） ——
async function loadFonts() {
  const results = await Promise.all(
    GLYPH_FONTS.map(async ([name, url]) => {
      try {
        const bytes = await (await fetch(url)).arrayBuffer()
        return engine.set_glyph_font(name, new Uint8Array(bytes))
      } catch {
        return false
      }
    }),
  )
  fontsReady.value = results.some(Boolean)
}

// —— 编译（防抖）→ 重建 Document；渲染只依赖句柄，不重排版 ——
function scheduleCompile() {
  clearTimeout(compileTimer)
  compileTimer = setTimeout(compileNow, DEBOUNCE_MS)
}

function compileNow() {
  if (engineState.value !== 'ready' || !src.value) return
  const t0 = performance.now()
  let d
  try {
    d = engine.compile_document(src.value)
  } catch (e) {
    // TeX 式错误：保留上一次成功渲染，状态行报错（编辑中间态常见，不清屏）
    statusErr.value = true
    status.value = `编译失败：${e.message || e}`
    return
  }
  doc = d
  doc.set_glyphs(glyphs.value && fontsReady.value)
  pageCount.value = doc.page_count
  pageNo.value = Math.min(pageNo.value, Math.max(0, doc.page_count - 1))
  transcript.value = doc.transcript || '（空转录）'
  const tCompile = performance.now() - t0
  const tRender = renderCurrent()
  statusErr.value = false
  status.value = `编译 ${tCompile.toFixed(1)}ms · 渲染 ${tRender.toFixed(1)}ms · ${doc.page_count} 页`
}

// —— 渲染当前页：软光栅 RGBA → ImageData 直灌 canvas；显示宽 = 96dpi 基准 × zoom ——
function renderCurrent() {
  const canvas = canvasRef.value
  if (!canvas) return 0
  if (!doc || doc.page_count === 0) {
    noPage.value = true
    return 0
  }
  const t0 = performance.now()
  const img = doc.render_page(pageNo.value, dpi.value, debug.value)
  canvas.width = img.width
  canvas.height = img.height
  canvas.style.width = `${Math.round(((img.width * 96) / dpi.value) * zoom.value)}px`
  const ctx = canvas.getContext('2d')
  ctx.putImageData(
    new ImageData(new Uint8ClampedArray(img.rgba), img.width, img.height),
    0,
    0,
  )
  noPage.value = false
  return performance.now() - t0
}

// —— 预览缩放：只改行内宽度（不触发 wasm 重渲染）；anchor 让缩放锚定光标/中心 ——
function setZoom(next, anchor) {
  next = Math.min(ZOOM_MAX, Math.max(ZOOM_MIN, next))
  const canvas = canvasRef.value
  const box = previewRef.value
  const prev = zoom.value
  if (next === prev || !canvas || !doc) return
  zoom.value = next
  canvas.style.width = `${Math.round(((canvas.width * 96) / dpi.value) * next)}px`
  if (anchor && box) {
    const k = next / prev
    box.scrollLeft = anchor.cx * k - anchor.px
    box.scrollTop = anchor.cy * k - anchor.py
  }
}

// 滚轮缩放（触摸板捏合同样以 ctrl+wheel 进入这里）：锚定光标下的内容点
function onWheel(e) {
  if (!doc || noPage.value) return
  const box = previewRef.value
  if (!box) return
  const px = e.clientX - box.getBoundingClientRect().left
  const py = e.clientY - box.getBoundingClientRect().top
  setZoom(zoom.value * (e.deltaY < 0 ? 1.25 : 0.8), {
    cx: box.scrollLeft + px,
    cy: box.scrollTop + py,
    px,
    py,
  })
}

function zoomStep(mult) {
  const box = previewRef.value
  if (!box) return setZoom(zoom.value * mult)
  const w = box.clientWidth / 2
  const h = box.clientHeight / 2
  setZoom(zoom.value * mult, { cx: box.scrollLeft + w, cy: box.scrollTop + h, px: w, py: h })
}
const zoomIn = () => zoomStep(1.25)
const zoomOut = () => zoomStep(0.8)
const resetZoom = () => setZoom(1)

// 拖拽平移：overflow 滚动的指针化（pointer capture 跟随出画布也不丢）
function onPanStart(e) {
  if (!doc || noPage.value) return
  const box = previewRef.value
  if (!box) return
  e.preventDefault()
  pan = { x: e.clientX, y: e.clientY, sl: box.scrollLeft, st: box.scrollTop }
  e.currentTarget.setPointerCapture(e.pointerId)
}
function onPanMove(e) {
  const box = previewRef.value
  if (!pan || !box) return
  box.scrollLeft = pan.sl - (e.clientX - pan.x)
  box.scrollTop = pan.st - (e.clientY - pan.y)
}
const onPanEnd = () => {
  pan = null
}

function prevPage() {
  if (pageNo.value > 0) pageNo.value--
}
function nextPage() {
  if (pageNo.value < pageCount.value - 1) pageNo.value++
}

function resetDemo() {
  const text = initialSrc.value || (engine && engine.demo_tex()) || ''
  if (text && text !== src.value) src.value = text
}

function downloadDvi() {
  if (!doc || doc.dvi.length === 0) {
    statusErr.value = true
    status.value = '无 DVI 可导出（0 页）'
    return
  }
  const blob = new Blob([doc.dvi], { type: 'application/octet-stream' })
  const a = document.createElement('a')
  a.href = URL.createObjectURL(blob)
  a.download = 'ntex.dvi'
  a.click()
  URL.revokeObjectURL(a.href)
}

// —— 事件：编辑防抖重排；翻页/dpi/overlay 只重渲染 ——
watch(src, scheduleCompile)
watch([pageNo, dpi, debug], () => {
  if (engineState.value !== 'ready') return
  const t = renderCurrent()
  statusErr.value = false
  status.value = `渲染 ${t.toFixed(1)}ms`
})
// 真字形开关：句柄上切换轮廓/方框通道后只重渲染（字体未注入时保持方框口径）
watch(glyphs, () => {
  if (engineState.value !== 'ready' || !doc) return
  doc.set_glyphs(glyphs.value && fontsReady.value)
  const t = renderCurrent()
  statusErr.value = false
  status.value = `渲染 ${t.toFixed(1)}ms（${glyphs.value && fontsReady.value ? '真字形' : '方框'}）`
})

onMounted(async () => {
  try {
    const res = await fetch(texUrl)
    if (!res.ok) throw new Error(`HTTP ${res.status}`)
    const text = await res.text()
    initialSrc.value = text
    if (!src.value) src.value = text
  } catch {
    // fetch 失败则回退 wasm 内置 demo_tex()（loadEngine 里处理）
  }
  loadEngine()
})

onBeforeUnmount(() => clearTimeout(compileTimer))
</script>

<template>
  <section class="section" id="demo">
    <div class="section-head" v-reveal>
      <p class="kicker">Live Engine</p>
      <h2>浏览器内实时排版</h2>
      <p>
        完整引擎经 WebAssembly 跑在你的浏览器里：编辑左侧 TeX，250ms 防抖后全链重排
        ——宏展开、Knuth-Plass 折行、断页、DVI、软光栅，全程不经任何服务器。
        翻页 / 调 dpi / 切 overlay 只重渲染不重排；Rust 侧与桌面引擎同一份代码。
      </p>
    </div>

    <div class="demo-shell" v-reveal>
      <div class="demo-tabs">
        <span class="demo-tab" :class="{ active: tab === 'live' }" @click="tab = 'live'">
          实时预览 · WASM
        </span>
        <span class="demo-tab" :class="{ active: tab === 'pdf' }" @click="tab = 'pdf'">
          demo.pdf · CI 产物
        </span>
        <span
          class="demo-engine"
          :class="engineState"
          :title="engineMsg || undefined"
        >
          {{ engineState === 'ready' ? engineMsg : engineState === 'loading' ? '引擎装载中…' : 'WASM 不可用，已回落静态 PDF' }}
        </span>
      </div>

      <div class="demo-live" v-show="tab === 'live'">
        <div class="live-bar">
          <span class="live-ctl">
            <button class="live-btn" title="上一页" :disabled="pageNo <= 0" @click="prevPage">‹</button>
            <span class="live-pages">{{ pageCount ? `${pageNo + 1} / ${pageCount}` : '– / –' }}</span>
            <button class="live-btn" title="下一页" :disabled="pageNo >= pageCount - 1" @click="nextPage">›</button>
          </span>
          <span class="live-ctl">
            <label for="live-dpi">dpi</label>
            <input id="live-dpi" type="range" min="72" max="288" step="12" v-model.number="dpi" />
            <span class="live-pages">{{ dpi }}</span>
          </span>
          <span class="live-ctl" title="预览缩放：滚轮缩放 / 拖拽平移">
            <button class="live-btn" title="缩小预览" :disabled="zoom <= ZOOM_MIN" @click="zoomOut">−</button>
            <span class="live-pages">{{ Math.round(zoom * 100) }}%</span>
            <button class="live-btn" title="放大预览" :disabled="zoom >= ZOOM_MAX" @click="zoomIn">＋</button>
            <button class="live-btn" v-if="zoom !== 1" title="复位缩放" @click="resetZoom">100%</button>
          </span>
          <span class="live-ctl">
            <label><input type="checkbox" v-model="debug" /> 排版 overlay</label>
          </span>
          <span class="live-ctl">
            <label :title="fontsReady ? 'Latin Modern 轮廓字形' : '字体未注入（回落方框）'">
              <input type="checkbox" v-model="glyphs" :disabled="!fontsReady" />
              真字形{{ fontsReady ? '' : '（未就绪）' }}
            </label>
          </span>
          <span class="grow"></span>
          <button class="live-btn" title="恢复示例源码" @click="resetDemo">重置</button>
          <button class="live-btn" title="导出当前作业的 DVI 字节" @click="downloadDvi">DVI ⤓</button>
          <span class="live-status" :class="{ err: statusErr }">{{ status }}</span>
        </div>

        <div class="live-main">
          <div class="live-editor-wrap">
            <textarea
              class="live-editor"
              v-model="src"
              spellcheck="false"
              placeholder="输入 plain TeX…"
            ></textarea>
          </div>
          <div class="live-preview" ref="previewRef" @wheel.prevent="onWheel">
            <div v-if="engineState === 'loading'" class="live-hint">
              正在装载 WASM 引擎（约 830KB，一次性下载）…
            </div>
            <div v-else-if="engineState === 'error'" class="live-hint">
              WASM 引擎装载失败：<code>{{ engineMsg }}</code>
            </div>
            <template v-else>
              <canvas
                ref="canvasRef"
                v-show="!noPage"
                @pointerdown="onPanStart"
                @pointermove="onPanMove"
                @pointerup="onPanEnd"
                @pointercancel="onPanEnd"
              ></canvas>
              <div v-if="noPage" class="live-hint">无页面（作业没有 \shipout 产出）</div>
            </template>
          </div>
        </div>

        <details class="live-log" open>
          <summary>转录（.log）</summary>
          <pre>{{ transcript || '—' }}</pre>
        </details>
      </div>

      <div class="demo-pdf" v-show="tab === 'pdf'">
        <iframe :src="pdfUrl" title="demo.pdf 预览"></iframe>
      </div>
    </div>
  </section>
</template>
