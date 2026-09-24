# One target per build step. `make` alone lists them.
.DEFAULT_GOAL := help
.PHONY: help asterisk-build asterisk-config asterisk-up asterisk-down asterisk-logs asterisk-cli \
	sip-accounts agent test eval

help: ## List the targets
	@grep -E '^[a-z-]+:.*## ' $(MAKEFILE_LIST) | awk 'BEGIN {FS = ":.*## "} {printf "  %-15s %s\n", $$1, $$2}'

asterisk-build: ## Build the Asterisk 22.11.0 image from source
	docker compose build asterisk

asterisk-config: ## Generate the SIP passwords (once) and read the Wi-Fi address into Asterisk's config
	asterisk/configure.sh

# --force-recreate: a running Asterisk is restarted, so it always runs with the current Wi-Fi
# address. A changed transport is not picked up by a reload (asterisk/config/pjsip.conf).
asterisk-up: asterisk-config ## Start Asterisk in the background, with the current config
	docker compose up -d --force-recreate asterisk

asterisk-down: ## Stop Asterisk
	docker compose down

asterisk-logs: ## Follow Asterisk's log
	docker compose logs -f asterisk

asterisk-cli: ## Open the Asterisk console
	docker compose exec asterisk asterisk -rvvv

sip-accounts: ## Print what to type into the two softphones, passwords included
	@asterisk/configure.sh accounts

agent: ## Run the agent natively in Ubuntu
	cargo run -p agent

test: ## Run the Rust tests
	cargo test --workspace

eval: ## Stream the scripted residents through the fake client and print the results table
	cargo run -p eval
