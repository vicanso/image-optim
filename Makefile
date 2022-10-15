.PHONY: default

lint-fix:
	cargo clippy --fix --allow-staged
lint:
	cargo clippy
fmt:
	cargo fmt --all --
dev:
	cargo run