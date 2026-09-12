# NTex 开发常用命令。
# 质量门禁：`make check` = fmt + lint + test 三件套全绿。

.PHONY: fmt lint test check fixtures fixture-extras trip diff bench tauri-wasm tauri \
        instrument-check abcheck blocker-track logtrace lvt-run lvt-fetch recovery-check oracle-verify

fmt:
	cargo fmt --all -- --check

lint:
	cargo clippy --workspace --all-targets -- -D warnings

test:
	cargo test --workspace

check: fmt lint test

# 仪器自检（P0 纪律）：诊断原语（\meaning/	he/\number/\romannumeral/\string）
# 与 pdfTeX 逐字对拍。**失真即回归**——见 docs/tooling-trust.md。
# 依赖：~/.local/bin/pdftex（TinyTeX）+ 先 `cargo build -p ntex-dvi`。
instrument-check:
	cargo build -q -p ntex-dvi
	python3 scripts/instrument-check.py

# 双引擎差分对拍：`make abcheck TEX=probe.tex [ARGS=--trace]`
abcheck:
	python3 scripts/abcheck.py $(TEX) $(ARGS)

# 阻塞点单调性看板：跑 latex_probe → 记录 (行号, 首错签名) → 单调性判定
blocker-track:
	./scripts/blocker-track.sh

# 错误恢复语义矩阵（fixtures/recovery/cases，oracle=pdfTeX 冻结判据）
# 修复验收：指定 case DIFF→OK 且 oracle-verify 无 DRIFT。详见 docs/tooling-trust.md §2.6
recovery-check:
	python3 scripts/abmatrix.py run

# oracle 仪器自检：复跑 pdfTeX 对比冻结值（DRIFT=环境/语义漂移，先查再跑矩阵）
oracle-verify:
	python3 scripts/abmatrix.py verify

# 转录/log 结构分析：`make logtrace LOG=path`（加 ARGS=--compare ref.log 对比参考）
# 回答「首现场在哪 / 震中是哪行 / 级联是扇出还是链式 / 哪些错我方独有」
logtrace:
	python3 scripts/logtrace.py $(LOG) $(ARGS)

# expl3 官方测试套件（l3kernel .lvt，187 例）：抓取 / 跑单例 / 全量跑分
# 见 docs/expl3-lvt-scoreboard.md
lvt-fetch:
	python3 scripts/lvt-run.py --fetch

lvt-run:
	python3 scripts/lvt-run.py $(CASE) $(ARGS)

# 获取 TRIP 测试 fixtures（优先 kpsewhich，失败则从 CTAN/GitHub 下载）
fixtures:
	./scripts/fetch-trip-fixtures.sh

# 获取补充对照 fixtures（pdftex expanded.{tex,txt} 等，详见 fixtures/README.md）
fixture-extras:
	./scripts/fetch-extras-fixtures.sh

# TRIP 框架（stub 驱动用于验证管路；接入真实引擎后作为一致性门禁）
trip:
	cargo run -p ntex-trip -- --driver stub

# 差分测试（示例 fixtures，参考/引擎均可换为 external=<程序>）
diff:
	cargo run -p ntex-diff -- --fixtures fixtures/diff --reference stub --engine stub

# 基准（stub 驱动验证管路；真实引擎接入后提供数字）
bench:
	cargo run -p ntex-bench -- --driver stub

# 构建 ntex-wasm 并生成 Tauri 前端绑定（wasm-bindgen 版本须与 Cargo.lock 一致，
# 见 crates/ntex-wasm/README.md「构建」）。注意：Homebrew rust 不带 wasm32 std，
# 须前置 rustup 工具链路径。
tauri-wasm:
	PATH="$$HOME/.cargo/bin:$$PATH" cargo build -p ntex-wasm --target wasm32-unknown-unknown --release
	wasm-bindgen --out-dir crates/ntex-tauri/ui/pkg --target web \
		target/wasm32-unknown-unknown/release/ntex_wasm.wasm

# Tauri 实时预览工作台（wasm 渲染形态）：排版+软光栅全在前端 WASM 内，
# Tauri 仅桌面壳；先 make tauri-wasm 再启动。
tauri: tauri-wasm
	cargo run -p ntex-tauri
