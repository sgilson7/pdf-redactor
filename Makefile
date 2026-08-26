ROOT := $(patsubst %/,%,$(dir $(abspath $(lastword $(MAKEFILE_LIST)))))

.PHONY: help test test-ui test-ui-setup web serve deploy clean

## test: run the engine test suite (native, no browser needed)
test:
	@cargo test --workspace

## test-ui: drive the built app in a real browser and verify exports
test-ui:
	@$(ROOT)/testing/run.sh

## test-ui-setup: one-time install of headless Chromium for test-ui
test-ui-setup:
	@python3 -m venv $(ROOT)/.venv-test
	@$(ROOT)/.venv-test/bin/pip -q install playwright pillow
	@$(ROOT)/.venv-test/bin/playwright install chromium
	@echo "ready: make test-ui"

## web: build the browser app into dist/web/
web:
	@$(ROOT)/packaging/package-web.sh

## serve: build and open the app locally
serve: web
	@echo "Serving http://localhost:8080/ - Ctrl-C to stop"
	@(sleep 1 && open http://localhost:8080/) >/dev/null 2>&1 &
	@cd $(ROOT)/dist/web && python3 -m http.server 8080

## deploy: push to GitHub; Actions builds and publishes to Pages
deploy:
	@cargo test --workspace --quiet
	@git push
	@echo "Pushed. Actions builds and publishes; live in about two minutes."
	@echo "Watch: gh run watch"

## clean: remove build output
clean:
	@rm -rf $(ROOT)/dist $(ROOT)/target

help:
	@grep -hE '^## ' $(MAKEFILE_LIST) | sed 's/## //' | awk -F': ' '{printf "  \033[1m%-8s\033[0m %s\n", $$1, $$2}'
