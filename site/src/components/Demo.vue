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
const latexMode = ref(false)
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
    // LaTeX 模式（C 档）：fetch NTEXBND1 资产包 → set_bundle 注入 fmt/tex/tfm，
    // 再开 latex_mode。包 ~15MB 一次性下载，失败则静默回落 plain 模式
    // （源内 \documentclass 等会报 Undefined control sequence，属预期回落行为）。
    try {
      const bundleBytes = await (await fetch(`${base}latex-bundle.bin`)).arrayBuffer()
      engine.set_bundle(new Uint8Array(bundleBytes))
      engine.set_latex_mode(true)
      latexMode.value = true
    } catch {
      latexMode.value = false
    }
    engineState.value = 'ready'
    engineMsg.value = engine.engine_version() + (latexMode.value ? ' · LaTeX mode' : ' · plain mode')
    if (!src.value) src.value = engine.demo_tex()
    // 字体注入与首排并行（先方框，字形就绪后重渲染；口径同 tauri 工作台）
    const fonts = loadFonts()
    compileNow()
    await fonts
    if (fontsReady.value && glyphs.value && doc) {
      doc.set_glyphs(true)
      status.value = `rendered in ${renderCurrent().toFixed(1)}ms (real glyphs)`
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
    status.value = `compile failed: ${e.message || e}`
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
  status.value = `typeset ${tCompile.toFixed(1)}ms · render ${tRender.toFixed(1)}ms · ${doc.page_count} pages`
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
    status.value = 'no DVI to export (0 pages)'
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
  status.value = `rendered in ${t.toFixed(1)}ms`
})
// 真字形开关：句柄上切换轮廓/方框通道后只重渲染（字体未注入时保持方框口径）
watch(glyphs, () => {
  if (engineState.value !== 'ready' || !doc) return
  doc.set_glyphs(glyphs.value && fontsReady.value)
  const t = renderCurrent()
  statusErr.value = false
  status.value = `rendered in ${t.toFixed(1)}ms (${glyphs.value && fontsReady.value ? 'real glyphs' : 'boxes'})`
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
      <h2>Real Typesetting in Your Browser</h2>
      <p>
        The full engine runs in your browser via WebAssembly: edit the TeX on the left and after a 250ms debounce the whole pipeline re-runs — macro expansion, Knuth-Plass line breaking, page breaking, DVI, and software rasterization, with no server involved.
        Changing page, dpi, or overlay re-renders without re-typesetting; this is the same Rust code as the desktop engine.
      </p>
    </div>

    <div class="demo-shell" v-reveal>
      <div class="demo-tabs">
        <span class="demo-tab" :class="{ active: tab === 'live' }" @click="tab = 'live'">
          Live preview · WASM
        </span>
        <span class="demo-tab" :class="{ active: tab === 'pdf' }" @click="tab = 'pdf'">
          demo.pdf · rendered output
        </span>
        <span
          class="demo-engine"
          :class="engineState"
          :title="engineMsg || undefined"
        >
          {{ engineState === 'ready' ? engineMsg : engineState === 'loading' ? 'Loading engine…' : 'WASM unavailable — fell back to static PDF' }}
        </span>
      </div>

      <div class="demo-live" v-show="tab === 'live'">
        <div class="live-bar">
          <span class="live-ctl">
            <button class="live-btn" title="Previous page" :disabled="pageNo <= 0" @click="prevPage">‹</button>
            <span class="live-pages">{{ pageCount ? `${pageNo + 1} / ${pageCount}` : '– / –' }}</span>
            <button class="live-btn" title="Next page" :disabled="pageNo >= pageCount - 1" @click="nextPage">›</button>
          </span>
          <span class="live-ctl">
            <label for="live-dpi">dpi</label>
            <input id="live-dpi" type="range" min="72" max="288" step="12" v-model.number="dpi" />
            <span class="live-pages">{{ dpi }}</span>
          </span>
          <span class="live-ctl" title="Zoom with the wheel, pan by dragging">
            <button class="live-btn" title="Zoom out" :disabled="zoom <= ZOOM_MIN" @click="zoomOut">−</button>
            <span class="live-pages">{{ Math.round(zoom * 100) }}%</span>
            <button class="live-btn" title="Zoom in" :disabled="zoom >= ZOOM_MAX" @click="zoomIn">＋</button>
            <button class="live-btn" v-if="zoom !== 1" title="Reset zoom" @click="resetZoom">100%</button>
          </span>
          <span class="live-ctl">
            <label><input type="checkbox" v-model="debug" /> Typesetting overlay</label>
          </span>
          <span class="live-ctl">
            <label :title="fontsReady ? 'Latin Modern 轮廓字形' : '字体未注入（回落方框）'">
              <input type="checkbox" v-model="glyphs" :disabled="!fontsReady" />
              Real glyphs{{ fontsReady ? '' : ' (not ready)' }}
            </label>
          </span>
          <span class="grow"></span>
          <button class="live-btn" title="Restore the sample source" @click="resetDemo">Reset</button>
          <button class="live-btn" title="Download the DVI bytes of this job" @click="downloadDvi">DVI ⤓</button>
          <span class="live-status" :class="{ err: statusErr }">{{ status }}</span>
        </div>

        <div class="live-main">
          <div class="live-editor-wrap">
            <textarea
              class="live-editor"
              v-model="src"
              spellcheck="false"
              placeholder="Enter plain TeX…"
            ></textarea>
          </div>
          <div class="live-preview" ref="previewRef" @wheel.prevent="onWheel">
            <div v-if="engineState === 'loading'" class="live-hint">
              Loading the WASM engine (~830KB, one-time download)…
            </div>
            <div v-else-if="engineState === 'error'" class="live-hint">
              Failed to load the WASM engine: <code>{{ engineMsg }}</code>
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
              <div v-if="noPage" class="live-hint">No pages (the job produced no \shipout)</div>
            </template>
          </div>
        </div>

        <details class="live-log" open>
          <summary>Transcript (.log)</summary>
          <pre>{{ transcript || '—' }}</pre>
        </details>
      </div>

      <div class="demo-pdf" v-show="tab === 'pdf'">
        <iframe :src="pdfUrl" title="demo.pdf 预览"></iframe>
      </div>
    </div>
  </section>
</template>
