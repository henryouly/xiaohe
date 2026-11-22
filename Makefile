.PHONY: build test lint fmt web-build docker-build clean

IMAGE_CORE ?= xiaohe-core:local
IMAGE_RUNTIME ?= xiaohe-runtime:local

build:
	cargo build

test:
	cargo test

lint:
	cargo fmt --all -- --check
	cargo clippy --all-targets -- -D warnings

fmt:
	cargo fmt --all

web-build:
	@if [ -f web/package.json ]; then \
		cd web && npm run build; \
	else \
		echo "web/package.json not found yet (scheduled for Phase 6); skipping web-build."; \
	fi

docker-build:
	docker build -f Dockerfile.core -t $(IMAGE_CORE) .
	docker build -f Dockerfile.runtime -t $(IMAGE_RUNTIME) .

clean:
	cargo clean
