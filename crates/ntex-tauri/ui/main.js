// NTex Studio（Tauri 壳）前端逻辑：排版与渲染全部在 ntex-wasm 内完成——
// compile_document() 产出页树句柄（Document），render_page() 走软光栅出
// RGBA 纹理，putImageData 上 canvas；翻页/调 dpi/debug 不重排版（B 档第一刀）。
// 引擎不进 Tauri Rust 进程：本文件是纯静态 ES module，无任何 IPC。
import init, {
  compile_document, demo_tex, engine_version, set_glyph_font, set_otf_font, set_utf8_input,
} from './pkg/ntex_wasm.js';

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
  utf8: true, inflight: false, dirty: false, timer: 0,
  // 预览显示缩放：zoom 为「位图像素 → 屏幕 CSS 像素」倍率；autoFit 时
  // 每次重排/窗口变化都重算为适配窗口的倍率（手缩放后自动关闭）。
  zoom: 1, autoFit: true,
};

/* ---------- 引擎 ---------- */

// 真字形：fetch Latin Modern OTF 注入 wasm（进程级注册表；映射见 ui/fonts/README.md）。
// 数学族 cmmi/cmsy/cmex 共用 OpenType MATH 单文件 latinmodern-math.otf
// （LM 无独立数学族 OTF；slot→Unicode 按 OML/OMS/OMX 编码分发，见 glyphs.rs）。
// 这些字体**有内嵌 TFM 度量**，注入只补轮廓 → set_glyph_font。
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

// 中文：Fandol Song 子集（OpenType 原生字体，**没有 TFM**）——排版阶段就要
// 字体字节建度量，故走 set_otf_font（一次写「度量 + 轮廓」两侧）；
// 用 set_glyph_font 会漏掉度量，`\font\zh=FandolSong-Regular` 直接 not loadable。
// 名字 = 文件主名，TeX 侧 `\font\zh=FandolSong-Regular at 11pt` 即命中。
const CJK_FONTS = [
  ['FandolSong-Regular', 'fonts/FandolSong-Regular.otf'],
];

async function loadFonts() {
  const fetchBytes = async (url) => new Uint8Array(await (await fetch(url)).arrayBuffer());
  const jobs = [
    ...GLYPH_FONTS.map(async ([name, url]) => {
      try { return set_glyph_font(name, await fetchBytes(url)); } catch { return false; }
    }),
    ...CJK_FONTS.map(async ([name, url]) => {
      try { return set_otf_font(name, await fetchBytes(url)); } catch { return false; }
    }),
  ];
  const results = await Promise.all(jobs);
  state.fontsReady = results.some(Boolean);
  if (!state.fontsReady) $('engine-info').textContent += ' · 字体加载失败（方框口径）';
}

// UTF-8 输入开关：走引擎参数注入口（不拼接源码——拼接会让 log 的 l.N 与
// 编辑器行号错位）。开 = 源文件可直接写中文；关 = bytes 模式（TeX 原语义）。
function applyUtf8() {
  state.utf8 = $('utf8').checked;
  set_utf8_input(state.utf8);
}

async function boot() {
  await init();
  $('engine-info').textContent = `${engine_version()} · wasm 软光栅`;
  applyUtf8();               // UTF-8 输入开关：必须早于首次编译
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
  applyZoom(); // 位图尺寸变了（dpi/页高），显示倍率需重新落到 style 上
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
$('prev-page').addEventListener('click', () => {
  if (state.page > 0) { state.page--; renderPage(); previewScroll.scrollTop = 0; }
});
$('next-page').addEventListener('click', () => {
  if (state.doc && state.page < state.doc.page_count - 1) {
    state.page++; renderPage(); previewScroll.scrollTop = 0;
  }
});
$('utf8').addEventListener('change', () => {
  // 输入编码是编译期语义 → 改了要重排（不只是重渲染）
  applyUtf8();
  clearTimeout(state.timer);
  compileNow();
});

/* ---------- 预览缩放 / 平移 ---------- */
// 口径与 ntex-studio（egui 形态）对齐：滚轮/捏合以指针为锚点缩放、主键拖拽平移；
// 上下限 5%~1600% 取自 studio 的 ZOOM_MIN/ZOOM_MAX。区别是这里显示的是**倍率**
// （1:1 = 一个位图像素对一个屏幕 CSS 像素），studio 显示的是相对"适配"的倍数。

const ZOOM_MIN = 0.05, ZOOM_MAX = 16.0;
const FIT_MIN = 0.01; // 页面远大于视口时的适配兜底，避免倍率退化到 0

const previewScroll = $('preview-scroll'), previewStage = $('preview-stage');
const zoomLabel = $('zoom-label');

// stage 的 padding（适配计算要从视口里扣掉）；padding 由 CSS 固定，取一次即可
let stagePad = null;
function padOf() {
  if (!stagePad) {
    const s = getComputedStyle(previewStage);
    stagePad = {
      x: parseFloat(s.paddingLeft) + parseFloat(s.paddingRight),
      y: parseFloat(s.paddingTop) + parseFloat(s.paddingBottom),
    };
  }
  return stagePad;
}

// 适配倍率：视口内容区里完整放下当前位图
function fitScale() {
  if (!canvas.width || !canvas.height) return 1;
  const p = padOf();
  const cw = previewScroll.clientWidth - p.x, ch = previewScroll.clientHeight - p.y;
  if (cw <= 0 || ch <= 0) return 1;
  return Math.max(FIT_MIN, Math.min(cw / canvas.width, ch / canvas.height));
}

// 把当前倍率落到 canvas 的显示尺寸上（不重排版、不重渲染——纯显示层）
function applyZoom() {
  if (!canvas.width || !canvas.height) return;
  const s = state.autoFit ? fitScale() : state.zoom;
  state.zoom = s;
  canvas.style.width = `${canvas.width * s}px`;
  canvas.style.height = `${canvas.height * s}px`;
  zoomLabel.textContent = `${(s * 100).toFixed(s < 0.1 ? 1 : 0)}%`;
}

function setZoom(s, autoFit = false) {
  state.autoFit = autoFit;
  state.zoom = Math.min(ZOOM_MAX, Math.max(ZOOM_MIN, s));
  applyZoom();
}

// 以视口坐标 (cx, cy) 为锚点缩放：锚点下的那个内容点保持不动（先量后写再校正）
function zoomAt(cx, cy, factor) {
  if (!canvas.width || !canvas.height) return;
  const before = canvas.getBoundingClientRect();
  const fx = (cx - before.left) / before.width, fy = (cy - before.top) / before.height;
  setZoom(state.zoom * factor);
  const after = canvas.getBoundingClientRect();
  previewScroll.scrollLeft += after.left + fx * after.width - cx;
  previewScroll.scrollTop += after.top + fy * after.height - cy;
}

function zoomCenter(factor) {
  const r = previewScroll.getBoundingClientRect();
  zoomAt(r.left + r.width / 2, r.top + r.height / 2, factor);
}

// 缩放到指定倍率（以视口中心为锚点）——「1:1」按钮与 Cmd/Ctrl+1 用
function zoomTo(target) { zoomCenter(target / state.zoom); }

// 滚轮：Cmd/Ctrl + 滚轮 或 触控板捏合（浏览器把捏合合成为 ctrlKey+wheel）→ 缩放；
// 其余滚动交给原生（纵向滚动、Shift 横滚）。手感系数 exp(∓d/240) 与 studio 一致。
previewScroll.addEventListener('wheel', (e) => {
  if (!e.ctrlKey && !e.metaKey) return;
  e.preventDefault();
  const unit = e.deltaMode === 1 ? 16 : e.deltaMode === 2 ? 400 : 1; // 行/页模式归一化
  zoomAt(e.clientX, e.clientY, Math.exp((-e.deltaY * unit) / 240));
}, { passive: false });

$('zoom-in').addEventListener('click', () => zoomCenter(1.25));
$('zoom-out').addEventListener('click', () => zoomCenter(0.8));
$('zoom-fit').addEventListener('click', () => setZoom(1, true));
$('zoom-one').addEventListener('click', () => zoomTo(1));

// 双击在「适配窗口 ↔ 1:1」之间切换（PDF 阅读器习惯）
canvas.addEventListener('dblclick', () => (state.autoFit ? zoomTo(1) : setZoom(1, true)));

// 键盘：Cmd/Ctrl + / − / 0 / 1（命中编辑框时也生效——这些组合不产生字符输入）
addEventListener('keydown', (e) => {
  if (!e.metaKey && !e.ctrlKey) return;
  switch (e.key) {
    case '=': case '+': e.preventDefault(); zoomCenter(1.25); break;
    case '-': case '_': e.preventDefault(); zoomCenter(0.8); break;
    case '0': e.preventDefault(); setZoom(1, true); break;
    case '1': e.preventDefault(); zoomTo(1); break;
    default: break;
  }
});

// 主键/中键拖拽平移（3px 阈值内不接管，避免误伤双击与点击）
let panDrag = null;
previewScroll.addEventListener('mousedown', (e) => {
  if (e.button !== 0 && e.button !== 1) return;
  // 只在页面本体/舞台留白上起手：滚动条、标题栏等处的按下不该拖页面
  if (e.target !== canvas && e.target !== previewStage) return;
  panDrag = {
    x: e.clientX, y: e.clientY,
    sl: previewScroll.scrollLeft, st: previewScroll.scrollTop, on: false,
  };
});
addEventListener('mousemove', (e) => {
  if (!panDrag) return;
  const dx = e.clientX - panDrag.x, dy = e.clientY - panDrag.y;
  if (!panDrag.on) {
    if (Math.abs(dx) < 3 && Math.abs(dy) < 3) return;
    panDrag.on = true;
    previewScroll.classList.add('grabbing');
  }
  e.preventDefault();
  previewScroll.scrollLeft = panDrag.sl - dx;
  previewScroll.scrollTop = panDrag.st - dy;
});
const endPan = () => {
  if (!panDrag) return;
  previewScroll.classList.remove('grabbing');
  panDrag = null;
};
addEventListener('mouseup', endPan);
addEventListener('blur', endPan); // 在窗口外松手：别把拖拽状态留在身上

// 分栏拖动/窗口缩放后，处于「适配」模式则跟随重算（倍率不变即不动，避免观察者自激）
new ResizeObserver(() => {
  if (!state.autoFit) return;
  if (Math.abs(fitScale() - state.zoom) < 1e-4) return;
  applyZoom();
}).observe(previewScroll);

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
