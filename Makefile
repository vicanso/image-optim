.PHONY: default

hooks:
	cp hooks/* .git/hooks/

lint-fix:
	cargo clippy --fix --allow-staged
lint:
	cargo clippy
fmt:
	cargo fmt --all --
dev:
	RUST_ENV=dev bacon run
dev-debug:
	RUST_ENV=dev LOG_LEVEL=5 cargo run

release:
	cargo build --release 