<template>
  <section class="hero">
    <div class="hero-inner">
      <div>
        <span class="eyebrow">Rust · A Modern Typesetting Kernel</span>
        <h1>
          TeX, rewritten
          <br />as a <span class="grad">modern engine</span>
        </h1>
        <p class="lead">
          NTex keeps every macro semantic intact and replaces TeX's runtime —
          a single-threaded C interpreter becomes a modern VM with incremental
          computation and parallel layout.
        </p>
        <div class="hero-cta">
          <a class="btn btn-primary" href="#demo">Live Demo</a>
          <a class="btn btn-ghost" href="https://github.com/congduan/NTex" target="_blank" rel="noopener">
            GitHub Repository
          </a>
        </div>
        <div class="hero-stats">
          <div><b>50k+</b> lines of Rust</div>
          <div><b>700+</b> tests</div>
          <div><b>Byte-exact</b> DVI parity</div>
        </div>
      </div>

      <div class="terminal">
        <div class="terminal-bar">
          <span class="t-dot r"></span>
          <span class="t-dot y"></span>
          <span class="t-dot g"></span>
          <span class="terminal-title">ntex — zsh</span>
        </div>
        <div class="terminal-body">
          <div><span class="term-prompt">$ </span><span class="term-cmd">cargo run -q -p ntex-dvi -- samples/transformer-real/transformer-standalone.tex out.dvi --input-path samples/transformer-real</span></div>
          <div class="term-out">wrote out.dvi (45095 bytes, 16 pages, 87 fonts)</div>
          <br />
          <div><span class="term-prompt">$ </span><span class="term-cmd">cargo run -q -p ntex-pdf -- out.dvi demo.pdf</span></div>
          <div class="term-out">wrote demo.pdf (1270266 bytes, 16 pages)</div>
          <br />
          <div><span class="term-prompt">$ </span><span class="term-cmd">pdfinfo demo.pdf | head -3</span></div>
          <div class="term-out">Pages:&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;16<br />Page size:&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;612 x 792 pts (letter)</div>
          <div class="term-ok">Knuth-Plass line breaking · TFM metrics · page breaking · Type1/Type0 embedding</div>
        </div>
      </div>
    </div>
  </section>
</template>
