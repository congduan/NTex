<template>
  <section class="hero">
    <div class="hero-inner">
      <div>
        <span class="eyebrow">Rust · 现代排版内核</span>
        <h1>
          把 1970 年代的 TeX
          <br />
          重写为<span class="grad"> 现代排版引擎</span>
        </h1>
        <p class="lead">
          NTex 不修改宏语义，只换掉 TeX 的运行平台——从单线程 C 解释器，
          升级为现代 VM、增量计算与并行布局的内核。
        </p>
        <div class="hero-cta">
          <a class="btn btn-primary" href="#demo">在线演示</a>
          <a class="btn btn-ghost" href="https://github.com/congduan/NTex" target="_blank" rel="noopener">
            GitHub 仓库
          </a>
        </div>
        <div class="hero-stats">
          <div><b>49k+</b>行 Rust</div>
          <div><b>620+</b>测试用例</div>
          <div><b>字节级</b>DVI 一致性</div>
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
          <div><span class="term-prompt">$ </span><span class="term-cmd">cargo run -q -p ntex-dvi -- demo.tex</span></div>
          <div class="term-out">已写出 demo.dvi（1933 字节，2 页，1 字体）</div>
          <br />
          <div><span class="term-prompt">$ </span><span class="term-cmd">cargo run -q -p ntex-pdf -- demo.dvi -p 595.28x841.89</span></div>
          <div class="term-out">已写出 demo.pdf（103007 字节，2 页）</div>
          <br />
          <div><span class="term-prompt">$ </span><span class="term-cmd">pdfinfo demo.pdf | head -3</span></div>
          <div class="term-out">Pages:&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;2<br />Page size:&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;595.28 x 841.89 pts</div>
          <div class="term-ok">Knuth-Plass 折行 · TFM 度量 · 断页 · Type1 嵌入</div>
        </div>
      </div>
    </div>
  </section>
</template>
