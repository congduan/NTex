# NTex 开发常用命令。
# 质量门禁：`make check` = fmt + lint + test 三件套全绿。

.PHONY: fmt lint test check fixtures fixture-extras trip diff bench

fmt:
	cargo fmt --all -- --check

lint:
	cargo clippy --workspace --all-targets -- -D warnings

test:
	cargo test --workspace

check: fmt lint test

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
