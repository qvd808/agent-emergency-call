# One target per build step. `make` alone lists them.
.DEFAULT_GOAL := help
.PHONY: help asterisk-build asterisk-config asterisk-up asterisk-down asterisk-logs asterisk-cli \
	sip-accounts models agent test-wav agent-test-wav test eval

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

# Silero VAD and whisper tiny.en, into models/ (gitignored). URLs as fetched 2026-09-24.
models: ## Download the VAD and speech-to-text models
	mkdir -p models
	cd models && test -f silero_vad.onnx || curl -fL -O https://raw.githubusercontent.com/snakers4/silero-vad/master/src/silero_vad/data/silero_vad.onnx
	cd models && test -f ggml-tiny.en.bin || curl -fL -O https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-tiny.en.bin

# --release: whisper built without optimisation is far too slow for a live call (inferred;
# only release builds were timed).
agent: ## Run the agent natively in Ubuntu
	cargo run --release -p agent

# A synthetic 12 s clip at 22.05 kHz, Piper's usual rate, so it is resampled twice on the
# way out, as the agent's speech will be. 0-5 s: a short 440 Hz beep on every second, to
# judge the pace against a clock. 5-8 s: a steady 440 Hz tone, where a dropped or doubled
# frame is heard as a click. 8-12 s: a sweep from 300 Hz to 7 kHz. The line carries up to
# 4 kHz, so the sweep should rise and then fade out; a tone that turns and falls instead is
# aliasing.
TEST_WAV := target/test-tones.wav
test-wav: ## Write the synthetic test clip for a live call (needs ffmpeg)
	mkdir -p $(dir $(TEST_WAV))
	ffmpeg -loglevel error -y -f lavfi -i "aevalsrc='0.3*if(lt(t,5), sin(2*PI*440*t)*lt(mod(t,1),0.15), \
		if(lt(t,8), sin(2*PI*440*t), sin(2*PI*300/(log(7000/300)/4)*(exp(log(7000/300)/4*(t-8))-1))))':s=22050:d=12" \
		-c:a pcm_s16le $(TEST_WAV)

agent-test-wav: test-wav ## Run the agent: each call plays the test clip, then listens
	TEST_WAV=$(TEST_WAV) cargo run --release -p agent

test: ## Run the Rust tests
	cargo test --workspace

eval: ## Stream the scripted residents through the fake client and print the results table
	cargo run -p eval
