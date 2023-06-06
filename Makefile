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
	cargo watch -w src -x 'run'
dev-debug:
	LOG_LEVEL=5 cargo run