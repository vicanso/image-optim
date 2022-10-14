.PHONY: default

lint:
	cargo clippy --fix --allow-staged
fmt:
	cargo fmt --all --
dev:
	cargo run