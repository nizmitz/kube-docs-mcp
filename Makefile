# Dev shortcuts. CI is the source of truth (.github/workflows).
export PATH := $(HOME)/.cargo/bin:$(PATH)
.PHONY: setup lint test build-web build-server index serve

setup:
	cd ingest && uv sync && uv run pre-commit install
	cd web && npm ci
	cd server && cargo fetch

lint:
	cd ingest && uv run pre-commit run --all-files

test:
	cd ingest && uv run pytest
	cd server && cargo test

build-web:
	cd web && npm run build

build-server: build-web
	cd server && cargo build --release

# make index SLUG=k8s
index:
	cd ingest && uv run ingest build $(SLUG) --out ../build/shards/$(SLUG).sqlite --work ../build/work
	cd ingest && uv run ingest merge --shards ../build/shards/*.sqlite --out ../build/index.sqlite --ingest-sha local --registry-sha local

serve: build-server
	./server/target/release/kube-docs-mcp serve --http 127.0.0.1:8080 --index build/index.sqlite --allowed-host 127.0.0.1:8080
