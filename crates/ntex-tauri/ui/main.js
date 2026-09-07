// NTex Studio（Tauri 壳）前端逻辑：排版与渲染全部在 ntex-wasm 内完成——
// compile_document() 产出页树句柄（Document），render_page() 走软光栅出
// RGBA 纹理，putImageData 上 canvas；翻页/调 dpi/debug 不重排版（B 档第一刀）。
// 引擎不进 Tauri Rust 进程：本文件是纯静态 ES module，无任何 IPC。
import init, { compile_document, demo_tex, engine_version, set_glyph_font } from './pkg/ntex_wasm.js';

const $ = (id) => document.getElementById(id);
const editor = $('editor'), hlcode = $('hlcode'), gutter = $('gutter'), hl = $('hl');
const canvas = $('preview'), ctx = canvas.getContext('2d', { willReadFrequently: true });
const pageLabel = $('page-label'), timing = $('timing'), statusEl = $('status');
const logEl = $('log'), errorBar = $('error-bar');

const DEBOUNCE_MS = 250;        // 与 ntex-studio 工作台同参数
const DRAFT_KEY = 'ntex-tauri-draft';
const HIGHLIGHT_LIMIT = 200_000; // 超长文档跳过高亮（叠层全量重排的护栏）

const state = {
  doc: null, page: 0, dpi: 96, debug: false, glyphs: true, fontsReady: false,
  inflight: false, dirty: false, timer: 0,
};

/* ---------- 引擎 ---------- */

// 真字形：fetch Latin Modern OTF 注入 wasm（进程级注册表；映射见 ui/fonts/README.md）。
// 数学族 cmmi/cmsy/cmex 共用 OpenType MATH 单文件 latinmodern-math.otf
// （LM 无独立数学族 OTF；slot→Unicode 按 OML/OMS/OMX 编码分发，见 glyphs.rs）。
const GLYPH_FONTS = [
  ['cmr10', 'fonts/lmroman10-regular.otf'],
  ['cmr12', 'fonts/lmroman12-regular.otf'],
  ['cmr7', 'fonts/lmroman7-regular.otf'],
  ['cmr5', 'fonts/lmroman5-regular.otf'],
  ['cmbx10', 'fonts/lmroman10-bold.otf'],
  ['cmti10', 'fonts/lmroman10-italic.otf'],
  ['cmtt10', 'fonts/lmmono10-regular.otf'],
  ['cmmi10', 'fonts/latinmodern-math.otf'],
  ['cmmi7', 'fonts/latinmodern-math.otf'],
  ['cmmi5', 'fonts/latinmodern-math.otf'],
  ['cmsy10', 'fonts/latinmodern-math.otf'],
  ['cmsy7', 'fonts/latinmodern-math.otf'],
  ['cmsy5', 'fonts/latinmodern-math.otf'],
  ['cmex10', 'fonts/latinmodern-math.otf'],
];

async function loadFonts() {
  const results = await Promise.all(GLYPH_FONTS.map(async ([name, url]) => {
    try {
      const bytes = await (await fetch(url)).arrayBuffer();
      return set_glyph_font(name, new Uint8Array(bytes));
    } catch { return false; }
  }));
  state.fontsReady = results.some(Boolean);
  if (!state.fontsReady) $('engine-info').textContent += ' · 字体加载失败（方框口径）';
}

async function boot() {
  await init();
  $('engine-info').textContent = `${engine_version()} · wasm 软光栅`;
  const fonts = loadFonts(); // 并行注入，不阻塞首屏（方框 → 字形就绪后重渲染）
  editor.value = localStorage.getItem(DRAFT_KEY) ?? demo_tex();
  refreshOverlay();
  compileNow();
  await fonts;
  renderPage(); // 字形就绪：按当前开关重渲染（不重排版）
}

function schedule() {
  clearTimeout(state.timer);
  state.timer = setTimeout(compileNow, DEBOUNCE_MS);
}

function compileNow() {
  if (state.inflight) { state.dirty = true; return; }
  state.inflight = true;
  setStatus('busy', '排版中…');
  const t0 = performance.now();
  try {
    const doc = compile_document(editor.value); // 同步：release wasm 下 demo 量级 ~几十 ms
    state.doc?.free?.();
    state.doc = doc;
    // 钳到 [0, page_count-1]：0 页作业会把上界压成 -1，须同时钳下界，
    // 否则 -1 经 wasm-bindgen 转 u32 回绕成 4294967295（与 www/index.html、
    // site Demo.vue 同口径）。
    state.page = Math.max(0, Math.min(state.page, doc.page_count - 1));
    errorBar.hidden = true;
    renderPage();
    setStatus('ok', `${doc.page_count} 页 · ${(performance.now() - t0).toFixed(0)} ms`);
    logEl.textContent = doc.transcript || '（转录为空）';
  } catch (e) {
    setStatus('err', '排版失败');
    errorBar.textContent = String(e);
    errorBar.hidden = false;
    logEl.textContent = state.doc?.transcript || logEl.textContent;
  } finally {
    state.inflight = false;
    if (state.dirty) { state.dirty = false; schedule(); }
  }
}

function renderPage() {
  if (!state.doc || state.doc.page_count === 0) { pageLabel.textContent = '– / –'; return; }
  state.doc.set_glyphs(state.glyphs && state.fontsReady); // 口径同步：所有渲染路径共用
  const img = state.doc.render_page(state.page, state.dpi, state.debug);
  canvas.width = img.width;
  canvas.height = img.height;
  ctx.putImageData(new ImageData(new Uint8ClampedArray(img.rgba), img.width, img.height), 0, 0);
  pageLabel.textContent = `${state.page + 1} / ${state.doc.page_count}`;
  $('prev-page').disabled = state.page === 0;
  $('next-page').disabled = state.page >= state.doc.page_count - 1;
}

function setStatus(kind, text) {
  statusEl.className = `status ${kind}`;
  statusEl.title = text;
  timing.textContent = text;
}

/* ---------- 编辑器：高亮叠层 + 行号 + 滚动同步 ---------- */

function escapeHtml(s) {
  return s.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;');
}

function refreshOverlay() {
  const src = editor.value;
  if (src.length > HIGHLIGHT_LIMIT) { hlcode.innerHTML = escapeHtml(src) + '\n'; }
  else {
    // 分组一次扫描：注释 > 控制序列 > 数学定界 > 特殊符号 > 数字（HTML 已转义）。
    hlcode.innerHTML = escapeHtml(src).replace(
      /(%[^\n]*)|(\\[a-zA-Z@]+|\\.)|(\$\$?)|([{}#&^_~])|(\d+)/g,
      (m, cm, cs, math, spec, num) => {
        if (cm) return `<span class="tk-cm">${cm}</span>`;
        if (cs) return `<span class="tk-cs">${cs}</span>`;
        if (math) return `<span class="tk-math">${math}</span>`;
        if (spec) return `<span class="tk-spec">${spec}</span>`;
        return `<span class="tk-num">${num}</span>`;
      },
    ) + '\n'; // 尾行换行：保证叠层与 textarea 滚动高度一致
  }
  const lines = src.split('\n').length;
  gutter.textContent = Array.from({ length: lines }, (_, i) => i + 1).join('\n');
}

editor.addEventListener('input', () => {
  refreshOverlay();
  schedule();
  saveDraft();
});
editor.addEventListener('scroll', () => {
  hl.scrollTop = editor.scrollTop;
  hl.scrollLeft = editor.scrollLeft;
  gutter.scrollTop = editor.scrollTop;
});
editor.addEventListener('keydown', (e) => {
  if (e.key === 'Tab') {
    e.preventDefault();
    const { selectionStart: s, selectionEnd: t } = editor;
    editor.setRangeText('  ', s, t, 'end');
    refreshOverlay(); schedule(); saveDraft();
  }
  if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === 's') {
    e.preventDefault();
    clearTimeout(state.timer);
    compileNow();
  }
});

let draftTimer = 0;
function saveDraft() {
  clearTimeout(draftTimer);
  draftTimer = setTimeout(() => localStorage.setItem(DRAFT_KEY, editor.value), 800);
}

/* ---------- 预览交互 ---------- */

$('dpi').addEventListener('change', (e) => { state.dpi = Number(e.target.value); renderPage(); });
$('debug').addEventListener('change', (e) => { state.debug = e.target.checked; renderPage(); });
$('glyphs').addEventListener('change', (e) => {
  state.glyphs = e.target.checked;
  if (state.glyphs && !state.fontsReady) loadFonts().then(renderPage); // 迟到重试
  else renderPage();
});
$('prev-page').addEventListener('click', () => { if (state.page > 0) { state.page--; renderPage(); } });
$('next-page').addEventListener('click', () => {
  if (state.doc && state.page < state.doc.page_count - 1) { state.page++; renderPage(); }
});
canvas.addEventListener('click', () => canvas.classList.toggle('zoom100'));

/* ---------- 分栏拖动 + log 折叠 ---------- */

const divider = $('divider');
divider.addEventListener('mousedown', (e) => {
  e.preventDefault();
  const startX = e.clientX, startW = $('editor-pane').getBoundingClientRect().width;
  const move = (ev) => {
    const w = Math.max(220, Math.min(window.innerWidth - 320, startW + ev.clientX - startX));
    $('editor-pane').style.flex = `0 0 ${w}px`;
  };
  const up = () => { removeEventListener('mousemove', move); removeEventListener('mouseup', up); };
  addEventListener('mousemove', move);
  addEventListener('mouseup', up);
});

$('log-toggle').addEventListener('click', () => $('log-pane').classList.toggle('collapsed'));

boot();
