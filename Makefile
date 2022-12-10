.PHONY: default

lint-fix:
	cargo clippy --fix --allow-staged
lint:
	cargo clippy
fmt:
	cargo fmt --all --
dev:
	LOG_LEVEL=5 cargo run