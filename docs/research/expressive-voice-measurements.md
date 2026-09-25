# Holding the "no": which Piper tricks lengthen one word, measured

Hands-on measurements for issue #38. The same issue's research note,
[expressive-voice.md](expressive-voice.md), covers the TTS survey and the cues of concern. It
ran no synthesis and left several points untested. This note runs the voice on those points
and does not repeat the survey.

Measured on 2026-09-25 in a cloud container with 4 vCPUs, not on the maintainer's laptop. The
measurements use Python (piper-tts 1.8.0, onnx 1.23.0, onnxruntime 1.30.0, praat-parselmouth
0.4.7), not the agent's Rust path. The container could not build the agent: its network policy
blocks `static.crates.io`, so crates cannot be downloaded. Scripts, raw output and a listening
set are in [expressive-voice/](expressive-voice/).

Tags follow the sibling note:
- **measured**: from a command whose output is in `expressive-voice/output/`.
- **read**: from a fetched source listed at the end.
- **inferred**: worked out, not stated anywhere.

## Answer (short)

1. **The per-phoneme edit from the note's M3 runs, and lengthens exactly the phoneme it is
   given.** ONNX Runtime accepts the edited graph (measured). With Piper's randomness in
   durations turned off (`noise_w` 0) and the vowel of "no" scaled, the word grows by:
   - ×1.5: +69 ms
   - ×2.0: +151 ms
   - ×2.5: +232 ms

   These are against a base of 279 ms in `hfc_female` (measured). `lessac` gains +80, +160 and
   +240 ms from the same scales (measured).

   The word's pitch stays where Piper put it: a median of 174–175 Hz against 176 Hz, over 30
   runs (measured). The patched graph costs no measurable time. Two interleaved pairs of
   30-run sets gave medians of 233.0 and 238.1 ms patched, against 234.8 and 235.5 ms stock,
   for a 4 s reply on this container (measured).
2. **A fixed number of extra frames is a better hold than a multiplier.** At the agent's real
   `noise_w` (0.7 for Steady), Piper draws each phoneme's length at random.
   - A multiplier scales that random part too. Over 30 runs the spread (max − min) of "no" grew
     from 139 ms at base to 267 ms at ×2.0 and 279 ms at ×2.5 (measured).
   - A second new input that *adds* frames puts 13 frames (+151 ms) or 20 frames (+232 ms) on
     top of whatever Piper drew. That is exact at `noise_w` 0; at 0.7 the mean gain was +150
     and +219 ms. Its spread stays near the base's: 197 and 174 ms (measured).
3. **The cheap tricks are unreliable, and they raise the pitch.**
   - Spelling "nooo" changes the vowel to `uː` (measured, and in the note's M1).
   - The length mark `ː` in the phoneme string adds +104 ms in `hfc_female` but only +32 ms in
     `lessac`. Repeating the vowel adds +34 ms (`nˈoʊoʊ`) or makes the word 24 ms shorter
     (`nˈooʊ`) (measured).
   - Every text or phoneme trick raised the "no" by 1.1–2.8 semitones: 188–207 Hz against
     176 Hz (measured). The note's section 3 points the other way for concern: low pitch
     (**inferred**).
4. **Saying "Oh no" separately buys the most length and the most side effects.**
   - Synthesised alone at ×1.5 speed, "Oh no," is +313 ms. As its own sentence ("Oh no."),
     which the agent would do by itself since `tts.rs` synthesises each sentence alone, it is
     +174 ms (measured).
   - Both slow "Oh" as well, raise the word by 1.8–2.8 semitones, and lengthen the whole line by
     0.5–1.0 s (measured).
   - The sentence route is voice-dependent: `lessac` gains only +48 ms (measured).
5. **A PSOLA stretch after synthesis is a working fallback.**
   - The vowel at ×2.0 is +175 ms in `hfc_female` and +177 ms in `lessac`, with the pitch kept
     (measured).
   - It costs 22.4 ms of CPU for a 4 s reply here (measured).
   - `prosody.rs` cannot do this yet: its `reshape` keeps the length
     (`agent/src/prosody.rs:65`).
6. **The comma already makes a pause.**
   - With punctuation kept, as `tts.rs` now does, the comma after "no" and the space after it
     take 151 ms of the voice's own durations. The space between "Oh" and "no" takes 70 ms
     (measured, `noise_w` 0).
   - How much of that is silence was not measured.
7. **Only listening can say which hold sounds sincere.** The listening set has the base line,
   the two fixed holds, and the main alternatives, at 8 kHz.

## Setup

**Voices.** Hugging Face is blocked from this container, so both voices came from GitHub
release assets:
- `en_US-hfc_female-medium`, from sherpa-onnx's `tts-models` release [R1]. Its graph is not the
  one in the maintainer's `models/`: 2 294 nodes, opsets `''`:17 and `com.microsoft`:1,
  against 2 755 nodes and opset 15 in the note's M2 (measured). It looks like an optimised
  re-export of the same voice (**inferred**). Its config matches: 22 050 Hz, defaults 0.667,
  1, 0.8. Its single `Ceil` is fed by the same `/Mul_1` (measured). Whether the durations
  match the maintainer's file exactly is unknown. The script runs on either file.
- `lessac-medium`, from rhasspy/piper's `v0.0.2` release [R2]. This export runs at 16 kHz, so a
  frame there is 16 ms, not 11.6 ms. Its graph has the 2 755-node, opset-15 shape of the note's
  M2 (measured).

**Phonemes.**
- piper-tts's espeak phonemizer [R3], which keeps clause punctuation as `tts.rs`'s `clauses()`
  now does (`agent/src/tts.rs:125`): `ˈoʊ nˈoʊ, aɪm sˈɑːɹi tə hˈɪɹ ðˈæt.` (measured).
- Ids follow piper-tts, with a pad after the start symbol. piper-rs omits that pad (the note,
  section 1). Its effect was not measured.

**Settings.**
- The Steady tone's `length_scale` 1.2 and `noise_scale` 0.6
  (`agent/src/tts.rs:57`), since "Oh no" is a reply to a concern.
- `noise_w` 0 gives repeatable durations (3 runs each, identical). With noise_w 0 the variants
  differ only by the trick; the flow's noise still varies, so pitch still varies run to run.
- `noise_w` 0.7, Steady's own value, gives the realistic spread (30 runs each).
- `prosody.rs`'s pitch reshaping was not applied.

**What "no" means.** The phonemes `n ˈ o ʊ` and their pads. Its length comes from the voice's
own per-phoneme frame counts: the `Ceil` output, as in piper-tts's alignment patch [R3], times
256 samples. On every run the script asserts that these add up to the audio's length.

**Pitch.**
- Praat autocorrelation through parselmouth, every 5 ms, 75–500 Hz, taking the median of the
  voiced frames inside the "no".
- `lessac`'s pitch figures are left out: its medians of 125–221 Hz and falls of 10–18
  semitones inside one word look like octave errors (**inferred**).

## Results

`hfc_female` at `noise_w` 0 (exact) and 0.7 (30 runs), and `lessac` at `noise_w` 0.
Source files: `output/hfc-noise_w-0.txt`, `output/hfc-noise_w-0.7.txt`,
`output/lessac-noise_w-0.txt`.

| Trick | hfc "no", ms (Δ) | hfc whole line, ms | hfc "no" at 0.7: mean (min–max) | hfc F0 of "no" at 0.7, Hz | lessac "no", ms (Δ) |
|---|---|---|---|---|---|
| base | 279 | 1997 | 295 (209–348) | 176 | 320 |
| **Text** | | | | | |
| spelled "nooo" → `nˈuːoʊ` | 325 (+46) | 2043 | 371 (325–453) | 193 | 416 (+96) |
| spelled "noooo" → `nˈuːuː` | 383 (+104) | 2148 | 391 (313–464) | 202 | 352 (+32) |
| "Oh, no," | 337 (+58) | 2136 | 375 (279–476) | 207 | 368 (+48) |
| **Phoneme string** | | | | | |
| `nˈoːʊ` | 383 (+104) | 2136 | 408 (337–488) | 206 | 352 (+32) |
| `nˈoʊː` | 383 (+104) | 2159 | 408 (313–464) | 193 | 352 (+32) |
| `nˈoːʊː` | 348 (+69) | 2043 | 386 (325–499) | 203 | 352 (+32) |
| `nˈoʊoʊ` | 313 (+34) | 2101 | 350 (267–406) | 196 | 336 (+16) |
| `nˈooʊ` | 255 (−24) | 2009 | 292 (186–441) | 188 | 304 (−16) |
| `nˈoːoːʊ` | 337 (+58) | 2090 | 379 (302–441) | 197 | 304 (−16) |
| **Per-phoneme input (edited graph)** | | | | | |
| vowel ×1.5 | 348 (+69) | 2067 | 377 (267–511) | 175 | 400 (+80) |
| vowel ×2.0 | 430 (+151) | 2148 | 466 (337–604) | 174 | 480 (+160) |
| vowel ×2.5 | 511 (+232) | 2229 | 524 (418–697) | 174 | 560 (+240) |
| `n` and vowel ×2.0 | 522 (+243) | 2241 | 542 (337–662) | 175 | 592 (+272) |
| vowel +13 frames | 430 (+151) | 2148 | 445 (337–534) | 174 | 528 (+208) |
| vowel +20 frames | 511 (+232) | 2229 | 514 (430–604) | 174 | 640 (+320) |
| **After synthesis** | | | | | |
| PSOLA, vowel ×1.5 | 366 (+87) | 2084 | 393 (268–511) | 176 | 408 (+88) |
| PSOLA, vowel ×2.0 | 454 (+175) | 2172 | 480 (326–651) | 175 | 497 (+177) |
| **Separate synthesis** | | | | | |
| "Oh no," alone at length ×1.5 | 592 (+313) | 2601 | 605 (499–708) | 207 | 464 (+144) |
| "Oh no," alone at length ×2.0 | 789 (+510) | 2984 | 798 (662–975) | 205 | 608 (+288) |
| "Oh no." as its own sentence | 453 (+174) | 2499 | 451 (383–557) | 200 | 368 (+48) |
| "Oh no." own sentence, length ×1.3 | 569 (+290) | 2719 | 565 (476–708) | 195 | 464 (+144) |

Notes on the rows:
- "Separate synthesis" mirrors `tts.rs`. "Oh no." as its own sentence gets 0.2 s of silence
  before the rest (`SENTENCE_SILENCE_S`, `agent/src/tts.rs:64`). "Oh no," alone is joined to the
  rest with no gap.
- 13 frames are 151 ms at 22 050 Hz and 208 ms at `lessac`'s 16 kHz.

### How the edit was made

The note's M3 replaced `scales[1]` at `/Mul_1` with the per-phoneme input. Here the edit keeps
`scales[1]` and adds two nodes between `/Mul_1` and `/Ceil`, so the tone's `length_scale`
still applies and a factor of 1 means "as Piper would":

```
/Mul_1 (exp(logw) * x_mask * length_scale)
  → Mul  × phoneme_length_scales   [1, 1, ids], 1.0 = unchanged
  → Add  + phoneme_extra_frames    [1, 1, ids], 0.0 = unchanged
  → /Ceil                          also exposed as a second output
```

In `hold_no.py`:
- A scale applies to a phoneme's own id and the pad after it.
- Extra frames go on the phoneme's own id only, split evenly over `o` and `ʊ`.
- The patched graph passes `onnx.checker` and runs in onnxruntime 1.30.0 (measured). Whether
  `ort` 2.0.0-rc.12, the version in the agent's `Cargo.lock`, runs it is untested here. It is
  the same kind of graph (**inferred**).

### Multiplier or fixed frames

| At `noise_w` 0.7, 30 runs | spread of "no" (max − min), ms |
|---|---|
| base | 139 |
| vowel ×1.5 / ×2.0 / ×2.5 | 244 / 267 / 279 |
| vowel +13 / +20 frames | 197 / 174 |
| PSOLA ×1.5 / ×2.0 | 243 / 325 |

(measured; from the min–max column above.)

The mean gain is about the same for both, e.g. ×2.0 gives +171 ms on average and +13 frames
+150 ms. The fixed frames add the same time to every draw, though, while a multiplier also
multiplies whatever Piper drew. A short draw then gives a short hold (**inferred** from the
graph: `Ceil(w·s + f)`).

Braver et al. (the note's [S52]) found the first step of emphatic lengthening is the largest
one, 285–389 ms. So a target in milliseconds is the natural thing to specify, which is what the
Add input takes (**inferred**).

### Pitch

- The edited-graph rows and the PSOLA rows keep the "no" at 174–176 Hz, the base's 176 Hz.
- Every text, phoneme and separate-synthesis row raises it to 188–207 Hz. That is +1.1 to +2.8
  semitones, taken from the Hz columns above.
- A plausible reading is that the voice treats an unusual spelling, or a word ending its own
  sentence, as more prominent and gives it a pitch accent (**inferred**; not tested).
- For a line of sympathy that is the wrong direction by the note's section 3: tenderness and
  sadness have low F0 [S48] (**inferred**).

### Cost

From `output/cost.txt`: stock against patched graph, the reply "Oh no, I'm sorry to hear that.
Were you able to get up by yourself?", 30 runs each, medians. The pairs were interleaved, and
the text is the same in both sets; different random draws gave 3.96 s and 4.25 s of speech.

| | stock | patched |
|---|---|---|
| first pair, 3.96 s of speech | 235.5 ms | 238.1 ms |
| second pair, 4.25 s of speech | 234.8 ms | 233.0 ms |

The difference is within run-to-run noise (measured). A Praat PSOLA stretch of one vowel,
re-synthesising the whole reply, took 22.4 ms (median of 10). These are container times. On
the laptop, Piper took 67–176 ms per reply (#18, the note's [S10]), so only the ratios carry
over (**inferred**).

### The pause at the comma

From `output/align.txt`, piper-tts's own alignment output [R3] for the base line:
- At `noise_w` 0, `,` takes 46 ms and the space after it 104 ms: 151 ms together. The space
  between "Oh" and "no" takes 70 ms.
- Over 30 runs at 0.7, the comma and its space average 159 ms, ranging 70–232 ms.

These are the voice's durations for those symbols, not measured silence (**inferred**:
VITS may voice part of them).

## What this settles in expressive-voice.md

- *"Whether ONNX Runtime accepts it"* (the per-phoneme edit, M3 and shortlist item 1): it does,
  in onnxruntime 1.30.0 from Python. Rust `ort` is untested.
- *"Whether the voice honours unseen sequences"* (`ː` after `oʊ`): `hfc_female` does, adding
  +104 ms. `lessac` barely does, adding +32 ms. And it raises the pitch.
- *"Roughly +150–300 ms"* for a vowel at ×2–×3: ×2.0 gives +151 ms and ×2.5 gives +232 ms at
  `noise_w` 0. At the tone's `noise_w` 0.7 the result swings with Piper's draw; use fixed frames.
- *"A join between the two clips"* (synthesising "Oh no," alone): it lengthens the word most,
  but slows "Oh" too and lifts the pitch about 2.7 semitones. The join itself is for the ear
  (sample 08).
- *"PSOLA artefacts on a held vowel"*: durations and pitch behave (+175 ms at ×2.0, pitch
  kept). Artefacts are for the ear (sample 06).

## For the ear: the listening set

`expressive-voice/samples/`, all "Oh no, I'm sorry to hear that." in `hfc_female`, Steady's
`length_scale` and `noise_scale`, `noise_w` 0.

Conditions:
- Resampled to 8 kHz with Praat's sinc resampler, as a softphone would receive them. The agent
  uses rubato instead, so small differences are possible (**inferred**).
- No `prosody.rs` reshaping.
- The flow's noise is random, so each file is one draw.

| File | What it is | "no", ms |
|---|---|---|
| `00-base.wav` | as Piper says it | 279 |
| `01-spelled-nooo.wav` | "Oh nooo", which espeak reads as `nˈuːoʊ` | 325 |
| `02-length-mark.wav` | phonemes `nˈoːʊ` | 383 |
| `03-hold-151ms.wav` | edited graph, vowel +13 frames | 430 |
| `04-hold-232ms.wav` | edited graph, vowel +20 frames | 511 |
| `05-whole-word-x2.wav` | edited graph, `n` and vowel ×2.0 | 522 |
| `06-psola-x2.wav` | base audio, vowel stretched ×2.0 by PSOLA | 454 |
| `07-own-sentence.wav` | "Oh no." then "I'm sorry to hear that." | 453 |
| `08-clause-alone-x1.5.wav` | "Oh no," synthesised alone at length ×1.5 | 592 |

Questions only the ear can answer:
- Does 03 or 04 sound held and sincere, or drawn out?
- Does 06 sound as natural as 03, which the voice rendered itself?
- Do the higher-pitched 02, 07 and 08 sound more or less concerned than 03?

## One candidate the note does not list: NeuTTS-2E

From neuphonic/neutts's README [R4], read:
- A "fixed-speaker emotional English model" with four speakers: `emily`, `paul`, `sophie` and
  `steven`.
- Their "pre-encoded references ... live in `samples/` ... so no reference audio is needed".
- Emotions: `angry`, `disgusted`, `fearful`, `happy`, `neutral`, `sad` and `surprised`, chosen
  per call: `tts.infer(text, speaker="emily", emotion="happy")`.
- About 125M active parameters, about 236M with embeddings. Text input.
- GGUF backbones, `q4` and `q8`, with streaming. An ONNX codec decoder, also `int8`.
- Watermarked outputs. "Real-time generation on mid-range devices", with no numbers.
- Licence: NeuTTS Open License 1.0, which "is intended to allow free research use and limited
  commercial use" [R4, `LICENSE`].

Inferred fit:
- Small enough for the CPU or the 2–3 GB of free GPU.
- A Rust path would be llama.cpp for the GGUF backbone plus `ort` for the decoder. Neither is
  official.
- There is no "sympathetic" or "concerned" label; `sad` is the nearest.
- Its speakers are stock voices shipped with the model, which the note accepts. Whether they
  are recordings of real people is not stated (unknown).
- It would join the bake-off in shortlist item 3.

Also read, and not added:
- **VoxCPM2** designs a voice from a description including emotion (Apache-2.0), but is a 2B
  model with RTF about 0.3 on an RTX 4090 [R5]. That puts it outside the budget
  (**inferred**).
- **OmniVoice** designs voices by attributes (gender, age, pitch, accent, whisper) and claims
  RTF as low as 0.025 [R6]. Its README does not give a size or an emotion control, so it is not
  assessed.

## Reproduce

On the laptop, against the maintainer's own voice file:

```
python -m venv .venv
.venv/bin/pip install piper-tts onnx onnxruntime praat-parselmouth numpy soundfile
.venv/bin/python docs/research/expressive-voice/hold_no.py models/en_US-hfc_female-medium.onnx target/hold-no 3 0
.venv/bin/python docs/research/expressive-voice/hold_no.py models/en_US-hfc_female-medium.onnx target/hold-no-07 30 0.7
.venv/bin/python docs/research/expressive-voice/align.py models/en_US-hfc_female-medium.onnx
.venv/bin/python docs/research/expressive-voice/cost.py models/en_US-hfc_female-medium.onnx
```

The laptop's `noise_w` 0 durations should match this note's if the two graph files carry the
same weights (**inferred**). A mismatch would mean the sherpa-onnx copy differs.

## Sources

Fetched or downloaded on 2026-09-25.

- [R1] k2-fsa/sherpa-onnx release `tts-models`, asset `vits-piper-en_US-hfc_female-medium.tar.bz2`
  (includes `MODEL_CARD`: "Finetuned from U.S. English lessac voice (medium quality)",
  dataset licence CC BY-NC-SA 4.0).
  https://github.com/k2-fsa/sherpa-onnx/releases/download/tts-models/vits-piper-en_US-hfc_female-medium.tar.bz2
- [R2] rhasspy/piper release `v0.0.2`, asset `voice-en-us-lessac-medium.tar.gz`.
  https://github.com/rhasspy/piper/releases/download/v0.0.2/voice-en-us-lessac-medium.tar.gz
- [R3] piper-tts 1.8.0, the wheel installed from PyPI (Home-page:
  http://github.com/OHF-voice/piper1-gpl):
  - `piper/patch_voice_with_alignment.py:15-57`: marks the single `Ceil` as an output.
  - `piper/voice.py`: `phonemize` with `[[ ]]` raw phonemes; `phoneme_ids_to_audio`, where
    samples per id are the second output × `hop_length`.
  - `piper/phoneme_ids.py:182-204`: a pad after BOS.
  - `docs/CLI.md` at OHF-Voice/piper1-gpl `main` (fetched raw), lines 56-61: "You can inject raw
    espeak-ng phonemes with `[[ <phonemes> ]]` blocks".
- [R4] neuphonic/neutts@ac69851f28fc63a487917e7c2e27f0d75c759cba, `README.md` (lines 14,
  36-74, 242-268) and `LICENSE`, fetched raw.
  https://github.com/neuphonic/neutts/tree/ac69851f28fc63a487917e7c2e27f0d75c759cba
- [R5] OpenBMB/VoxCPM@f772e498a45fbb5fb8e13fbf9b9c48be9fe33e69, `README.md` lines 43-54,
  fetched raw.
- [R6] k2-fsa/OmniVoice@08be0b4ccbac3e13e374e86fbfead4b4cac343e2, `README.md` lines 19-30,
  fetched raw.
- thewh1teagle/piper-rs@d70b0970a87453f3476b5fd6cf9edf329be8b445 (the commit the note's [S1]
  names), cloned: `src/model.rs:60-105`. `infer` passes inputs by position and reads only
  `outputs[0]`, so an extra *output* would not disturb it (**inferred**). An extra *input*
  needs the agent's own `ort` call, as the note says.
- This repository: `agent/src/tts.rs` (lines 54-57, 64, 90, 125) and `agent/src/prosody.rs:65`,
  at `b54ff37`.

Not fetched: Hugging Face, arXiv, PMC, Frontiers, Nature, ISCA, DOI, Semantic Scholar and
docs.rs are all blocked by this container's network policy. Nothing here relies on them; the
literature is the sibling note's.
