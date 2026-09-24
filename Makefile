# One target per build step. `make` alone lists them.
.DEFAULT_GOAL := help
.PHONY: help asterisk-build asterisk-up asterisk-down asterisk-logs asterisk-cli agent test eval

help: ## List the targets
	@grep -E '^[a-z-]+:.*## ' $(MAKEFILE_LIST) | awk 'BEGIN {FS = ":.*## "} {printf "  %-15s %s\n", $$1, $$2}'

asterisk-build: ## Build the Asterisk 22.11.0 image from source
	docker compose build asterisk

asterisk-up: ## Start Asterisk in the background
	docker compose up -d asterisk

asterisk-down: ## Stop Asterisk
	docker compose down

asterisk-logs: ## Follow Asterisk's log
	docker compose logs -f asterisk

asterisk-cli: ## Open the Asterisk console
	docker compose exec asterisk asterisk -rvvv

agent: ## Run the agent natively in Ubuntu
	cargo run -p agent

test: ## Run the Rust tests
	cargo test --workspace

eval: ## Stream the scripted residents through the fake client and print the results table
	cargo run -p eval
