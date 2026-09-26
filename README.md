# NTex

**English** | [简体中文](README.zh-CN.md)

> **Project status: work in progress** — NTex is currently in the LaTeX/expl3 conformance push and is
> **not yet in a usable product form**: the CLI, Rust APIs and asset layout may all change.
> Do not use it in production. The **single source of truth for progress is [plan.md](plan.md)**;
> the "Development progress" section below is only a snapshot.

**A modern typesetting & rendering engine that is 100% compatible with the LaTeX/TeX macro machinery** —
it does not change macro semantics; it only replaces the "runtime platform" underneath: the
single-threaded C interpreter of the 1970s (Web2C) is rewritten into a modern typesetting kernel built on
**a modern VM + incremental computation + parallel layout + GPU rendering**.

> In one sentence: **"a state-snapshot-based TeX virtual machine (evaluation) + a pure node-stream, multi-threaded incremental layout (typesetting) + GPU/Canvas (rendering)"**

![The ntex-studio live preview workbench (left: TeX source editor; right: vello GPU rendering)](screenshots/screenshot1.png)

## Development progress

> Snapshot date 2026-09-26. The **single source of truth for progress is [plan.md](plan.md)**; this table
> is only a quick overview, and the bars are rough estimates meant to convey order of magnitude.

**Milestones**:

| Milestone | Progress | Status |
|---|---|---|
| M0 Foundation | `██████████` | Done: workspace / CI / differential tooling / benchmark framework |
| M1 Interpreter core | `████████░░` | Core done: tokens / catcodes / macro expansion / core primitives; TRIP semantic diff not yet zero |
| M2 Bytecode compiler | `███████░░░` | Dual-track equivalence fully green (100 cases); throughput 1.01x — ≥2x is structurally unreachable → moved to M7 |
| M3 Typesetting core | `██████████` | Done: nodes / boxes / glue / Knuth-Plass / TFM / page breaking / DVI, byte-identical to real TeX |
| M4 Math + e-TeX | `████████░░` | Features complete (math state machine / e-TeX / hyphenation); ETRIP wrap-up ongoing |
| M5 Incremental computation | `████████░░` | Phases 1–5 done (body edit 4.3x / macro-body edit 1.1x); 7 items left in phase 6 |
| M6 Parallelism | `░░░░░░░░░░` | Not started (waiting on M5 side-effect boundaries) |
| M7 `.fmt` v2 | `░░░░░░░░░░` | Not started (the v1 in-memory snapshot is already usable) |
| M8 Rendering / output | `████████░░` | Usable: ntex-pdf + software rasterizer / vello GPU + studio / wasm / tauri workbench |
| M9 Ecosystem push | `███░░░░░░░` | Partial: CJK blades 1/2/3/5 (blade 4 pending), package manager parse / contract / fetch layers ①② |

**Current focus and key numbers** (2026-09-26):

- **Single main line**: the LaTeX/expl3 conformance push — `latex.ltx` body loading stops at an observed
  **88.4%** (pos=685828, 500 s timeout); this is the main wall and its root cause is still open.
  `\documentclass{article}` is not working yet.
- **expl3 official suite**: l3kernel `.lvt` × 187, load endpoint **89.4% of lines / 90.0% of bytes**;
  real load-time errors reduced 573 → **1** (non-blocking).
- **Quality gate**: `make check` fully green (`cargo fmt` + clippy `-D warnings` + unit tests
  **800+ passed / 0 failed**).
- **Measured incremental typesetting** (release, 120-segment document): body edit **4.3x**,
  macro-body edit **1.1x** (target 100x).
- **Rendering / preview**: the PDF backend, the software rasterizer and vello GPU path, the
  `ntex-studio` live workbench, and the wasm / Tauri frontend workbench are all usable.
- **Not working yet**: real LaTeX layout (structural macros / NFSS sizes / tables / footnotes /
  the package ecosystem); on 2026-09-03 the byte-exact TRIP/ETRIP criterion was downgraded to
  "semantic diff zeroed + spot-checked error blocks".
- **Four performance pillars**: state snapshot + CoW (available) / `.fmt` v1 (available, v2 pending) /
  segment-level incrementality (phases 1–5) / arena memory pool (pending).

## Core architecture

```
TeX/LaTeX source
    ▼
0. Input layer (VFS / file abstraction, ntex-io)
    ▼
1. TeX virtual machine (stage 1: evaluation, ntex-core)
   ┌──────────────┐   ┌──────────────────────────────┐
   │ Token stream │◄─►│ Immutable state snapshot     │
   │ (8B token)   │   │ (catcode / macros /          │
   └──────┬───────┘   │  registers)                  │
          ▼           └──────────────────────────────┘
   macro expansion + execution loop (bytecode is the default path)
   pure typesetting node stream (Node List)
    ▼
2. Incremental cache layer (segment snapshots + dependency tracking + invalidation propagation)
    ▼
3. Layout engine (stage 2: typesetting, ntex-layout)
   Knuth-Plass paragraph breaking │ page breaking │ math typesetting │ hyphenation │ fonts
    ▼
4. Box tree + output routine (\shipout) → DVI
    ▼
5. Rendering backends (stage 3): ntex-pdf (DVI→PDF) / ntex-backend (vello GPU / software rasterizer)
```

**Four performance pillars**:

1. **State snapshot + CoW**: immutable state tables with microsecond-level snapshots, paving the way
   for incremental compilation and undo/redo
2. **Precompiled `.fmt` in-memory dump + mmap**: complete `latex.ltx` initialization in milliseconds
   (v1 available, v2 pending)
3. **Segment-level memoized incremental evaluation**: editing one segment recomputes only the affected
   segments (see [plan.md](plan.md) §2 M5 for current status)
4. **Arena memory pool**: contiguous allocation of tokens/nodes with zero GC pauses (pending)

Crate responsibilities are described in [AGENTS.md](AGENTS.md); the full rationale behind the
architectural vision is in [idea.md](idea.md). Current progress and milestone status always live in
[plan.md](plan.md).

## Quick start

```bash
# Quality gate: fmt + clippy (-D warnings) + unit tests (must be green before committing)
make check

# Diagnostic infrastructure (read docs/tooling-trust.md before starting any debugging work)
make instrument-check                   # instrument self-check: diagnostic primitives vs. pdfTeX, byte for byte
make abcheck TEX=probe.tex ARGS=--trace # A/B differential run between the two engines
make logtrace LOG=x.transcript          # transcript / log structural analysis
make blocker-track                      # blocker monotonicity dashboard

# End-to-end demo: samples/demo.tex → DVI → PDF
cargo run -p ntex-dvi -- samples/demo.tex                        # → samples/demo.dvi
cargo run -p ntex-pdf -- samples/demo.dvi                        # → samples/demo.pdf
cargo run -p ntex-backend -- samples/demo.tex demo 144 --vello   # → PNG (GPU + real glyphs)

# LaTeX fast path: no external TeX Live needed; looks in assets/fmt and assets/tex-minimal by default
cargo run -p ntex-dvi -- --fmt latex.fmt doc.tex
cargo run -p ntex-dvi -- --generate-fmt /tmp/latex.fmt    # regenerate the shipped fmt after engine semantic changes

# Live preview workbench (TeX editing on the left / vello GPU rendering on the right, 250 ms debounced relayout)
cargo run -p ntex-studio [file.tex]
make tauri                                                # Tauri desktop shell (typesetting + rendering entirely in frontend WASM)

# Conformance / differential / benchmark pipelines
make trip / make diff / make bench
make fixtures / make fixture-extras        # fetch TRIP·ETRIP / extra reference fixtures
make lvt-fetch / make lvt-run ARGS=--all   # run the official expl3 test suite
```

The default `\input` search chain of `ntex-dvi` is: cwd → explicit `--input-path` → `tex/` next to the
executable → `~/.ntex/tex/` → `TEXINPUTS` → the repository/distribution
`assets/tex-minimal/tex/` → a detected `~/.TinyTeX/texmf-dist/tex/` → finally falling back to
`kpsewhich`. See [assets/tex-minimal/README.md](assets/tex-minimal/README.md) for the full asset
description.

## Development conventions

### Coding standards

- **Project-wide lint**: the workspace uniformly sets `unsafe_code = "deny"` and enables all clippy
  lints — `unsafe` is strictly forbidden, as is `catch_unwind`;
- **Error model**: errors go through `Result`/`Error`; `unwrap`/`expect`/`panic` are forbidden on
  input-reachable paths (engine contract: **no panic on any malformed input**). Error types carry no
  blanket `From<io::Error>`; they must carry operation-intent context;
- **Comments/docs/commit messages are in Chinese**; module-level docs reference milestone IDs
  (e.g. `M1-4`) and RFC sections (e.g. `RFC-1 §3`); code changes must update the corresponding
  comments in sync;
- **Formatting**: `rustfmt.toml` = edition 2021 / max_width 100 / use_field_init_shorthand;
- Commit message style: Chinese, prefixed with `feat:`, with a space after the colon
  (e.g. `feat: ETRIP 冲刺迭代 —— 表达式 i128 中间量 + 胶水阶语义`).

### Testing discipline

- **`make check` must be green before committing** (fmt + clippy `-D warnings` + unit tests);
- Testing pyramid: unit tests (locked semantic messages / bit-exact comparisons) > differential tests
  (the same `.tex` diffed across two engines) > TRIP/ETRIP (the hard conformance criterion). The M2
  dual-track equivalence framework (interpreter vs. bytecode) must stay green;
- **`cargo test --release` will fail**: `[profile.release]` enables `panic = "abort"` + `lto` +
  `codegen-units = 1` — **always run tests with the default dev profile** (CI does the same);
- For anything touching typesetting conformance / line-breaking results, verify by comparing
  `demo.tex → DVI` against dvipdfmx / real TeX;
- Read [docs/tooling-trust.md](docs/tooling-trust.md) (instrument failure history + interpretation
  discipline) before making any debugging-related change, and always run `make instrument-check`
  after modifying diagnostic primitives.

### Documentation conventions

- **Progress goes only in [plan.md](plan.md)** (the single source of truth for progress); directory
  structure goes only in [AGENTS.md](AGENTS.md);
- Long-lived reference docs ("what it is / how to do it") live in `docs/`; historical battle reports
  and full investigation write-ups live in `docs/archive/`;
- Any new "simplified / no-op / not yet" implementation must be registered the same day in
  [docs/KNOWN-SIMPLIFICATIONS.md](docs/KNOWN-SIMPLIFICATIONS.md) (file:line), and marked ✅ + commit
  once fixed.

## AI-assisted development

The project is human-led in architecture and acceptance, while day-to-day implementation relies heavily
on **AI coding tools — including DeepSeek / GLM / Codex / Claude**.

- **Humans own**: architecture decisions ([idea.md](idea.md) / RFCs), milestone criteria and acceptance,
  A/B judgments against real TeX / pdfTeX, commits and releases;
- **AI produces**: bulk implementation, defect localization and refactoring, test completion,
  documentation and archive upkeep;
- **Bar for landing**: every change must pass `make check` (fmt + clippy `-D warnings` + unit tests)
  plus differential / A/B evidence. Debugging conclusions must also follow the instrument
  interpretation discipline in [docs/tooling-trust.md](docs/tooling-trust.md) — a conclusion gains no
  credibility from being AI-produced; only evidence counts.

## Documentation map

| Document | Scope |
|---|---|
| [plan.md](plan.md) | **Overall progress**: current focus / status per workstream / milestone details / TODOs / risks |
| [AGENTS.md](AGENTS.md) | **Directory structure**: crate responsibilities, source file layout, docs index |
| [idea.md](idea.md) | Architectural vision (full rationale for the three-stage decoupling / incremental computation / parallel layout) |
| [RFC-1-token.md](RFC-1-token.md) | Token representation and memory layout (8B tagged union) |
| [RFC-3-side-effects.md](RFC-3-side-effects.md) | Side-effect isolation (VFS + commit at the shipout boundary) |
| [RFC-4-bytecode.md](RFC-4-bytecode.md) | Bytecode instruction set (macro-expansion VM IR) |
| [RFC-5-parallel.md](RFC-5-parallel.md) | Parallelization design |
| [docs/KNOWN-SIMPLIFICATIONS.md](docs/KNOWN-SIMPLIFICATIONS.md) | Technical debt list (consult before changing anything) |
| [docs/tooling-trust.md](docs/tooling-trust.md) | Diagnostic infrastructure and instrument trustworthiness |
| [docs/MATH-STATE-MACHINE.md](docs/MATH-STATE-MACHINE.md) | Math state machine semantic specification |
| [docs/archive/](docs/archive/README.md) | Historical archive index (battle reports, full investigations) |

## License

MIT OR Apache-2.0
