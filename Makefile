ROOT := $(patsubst %/,%,$(dir $(abspath $(lastword $(MAKEFILE_LIST)))))

.PHONY: help test web serve deploy clean

## test: run the engine test suite (native, no browser needed)
test:
	@cargo test --workspace

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
