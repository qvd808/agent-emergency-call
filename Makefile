# One target per build step. `make` alone lists them.
.DEFAULT_GOAL := help
.PHONY: help asterisk-build asterisk-config asterisk-up asterisk-down asterisk-logs asterisk-cli \
	sip-accounts models espeak-data agent checkin test-wav agent-test-wav test eval

# whisper.cpp on an NVIDIA GPU (issue #52): with the CUDA toolkit installed, the agent and the
# eval build with the `cuda` feature and hear with large-v3-turbo. Without it they build for the
# CPU and hear with tiny.en, as before. /usr/local/cuda is where whisper-rs-sys 0.15.0's
# build.rs looks for CUDA's libraries on Linux.
CUDACXX ?= $(wildcard /usr/local/cuda/bin/nvcc)
ifneq ($(CUDACXX),)
export CUDACXX
CUDA_FEATURE := --features cuda
endif

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

# Silero VAD, whisper tiny.en (and large-v3-turbo for the GPU build), Smart Turn v3.2 and the
# Piper voices, into models/ (gitignored).
# URLs as fetched 2026-09-24; Smart Turn's is the one the turn-detection prototype used (branch
# prototype/turn-detection, turn/prototype/PROTOTYPE_README.md); the voices' paths follow
# piper-rs 0.2.0's examples/usage.rs. hfc_female is the agent's voice, picked by ear; lessac,
# the voice before it, is the eval's residents.
PIPER_VOICES := https://huggingface.co/rhasspy/piper-voices/resolve/main/en/en_US
models: ## Download the VAD, speech-to-text, end-of-turn and voice models
	mkdir -p models
	cd models && test -f silero_vad.onnx || curl -fL -O https://raw.githubusercontent.com/snakers4/silero-vad/master/src/silero_vad/data/silero_vad.onnx
	cd models && test -f ggml-tiny.en.bin || curl -fL -O https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-tiny.en.bin
	$(if $(CUDA_FEATURE),cd models && test -f ggml-large-v3-turbo-q5_0.bin || curl -fL -O https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-large-v3-turbo-q5_0.bin)
	cd models && test -f smart-turn-v3.2-cpu.onnx || curl -fL -O https://huggingface.co/pipecat-ai/smart-turn-v3/resolve/main/smart-turn-v3.2-cpu.onnx
	cd models && for v in hfc_female/medium/en_US-hfc_female-medium lessac/medium/en_US-lessac-medium; do \
		f=$$(basename $$v); \
		test -f $$f.onnx || curl -fL -O $(PIPER_VOICES)/$$v.onnx; \
		test -f $$f.onnx.json || curl -fL -O $(PIPER_VOICES)/$$v.onnx.json; \
	done

# Piper turns text into phonemes with espeak-ng, whose dictionaries the espeak-rs-sys crate
# compiles during the build. espeak-rs looks for them in PIPER_ESPEAKNG_DATA_DIRECTORY
# (espeak-rs 0.2.0, src/lib.rs:53-75), so they are copied out of the build into models/.
export PIPER_ESPEAKNG_DATA_DIRECTORY := models
espeak-data:
	cargo build --release -p agent $(CUDA_FEATURE)
	test -d models/espeak-ng-data || cp -r "$$(ls -d target/release/build/espeak-rs-sys-*/out/share/espeak-ng-data | head -1)" models/

# --release: whisper built without optimisation is far too slow for a live call (inferred;
# only release builds were timed).
# Ollama must be running: `ollama serve`, then `ollama pull` the model in .env once.
agent: espeak-data ## Run the check-in agent natively in Ubuntu
	cargo run --release -p agent $(CUDA_FEATURE)

# The agent as `make agent` runs it, plus one round of outbound check-ins as soon as it is ready
# (issue #22): it calls every extension in RESIDENTS, retrying as the CHECKIN_* settings in .env
# say, and keeps running to answer calls until stopped.
checkin: espeak-data ## Run the agent and call every resident now, with retries
	CHECKIN_NOW=on cargo run --release -p agent $(CUDA_FEATURE)

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

agent-test-wav: test-wav espeak-data ## Run the agent: each call plays the test clip, then listens
	TEST_WAV=$(TEST_WAV) cargo run --release -p agent $(CUDA_FEATURE)

test: ## Run the Rust tests
	cargo test --workspace

# Every call runs in real time, so a full run takes about a quarter of an hour. The table goes
# to the terminal and to calls/eval/<run>/report.md, the agent's own log to calls/eval/agent.log.
# `make eval REPEAT=3` is the sign-off run (issue #29); `PERSONA=doing_fine,barge_in` runs only
# those. Ollama must be running, as for `make agent`. Exits non-zero if a gate fails.
eval: espeak-data ## Call the agent with scripted residents and print the results table
	mkdir -p calls/eval
	cargo build --release -p eval $(CUDA_FEATURE)
	REPEAT=$(REPEAT) PERSONA=$(PERSONA) target/release/eval 2> calls/eval/agent.log
