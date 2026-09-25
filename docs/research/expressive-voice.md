# A voice that sounds sincere: word-level control in Piper, expressive local TTS, and the cues of concern

Research for issue #38. It follows #37 (pitch reshaping after Piper) and sits beside the
hands-on Piper measurements for the same issue
([expressive-voice-measurements.md](expressive-voice-measurements.md)), which this note does
not repeat: no speech was synthesised and no TTS model was run for it.

Fetched on 2026-09-24. Repository files are pinned to the commit shown in **Sources**. Model
metadata (licence tag, parameter count, file sizes) is from the Hugging Face model API
(`/api/models/<id>?blobs=true`) on that date.

Bracketed tags like [S1] point to the **Sources** list at the end. Each claim is marked by how
it was obtained:

- **read**: stated in a fetched source (the default when a tag follows).
- **measured**: produced by a command run for this note. The only measurements are
  phonemisation with espeak-rs (the code path piper-rs uses) and inspection of the voice's
  ONNX graph. Commands and raw output are in the appendix.
- **inferred**: worked out from the above, not stated anywhere.

A number with no source is reported as **unknown**.

---

## Question

1. Can Piper (piper-rs 0.2.0, rhasspy/piper, OHF-Voice/piper1-gpl) lengthen or stress one
   word? SSML, per-phoneme durations, phoneme input with length marks, a per-phrase
   `length_scale`, or a voice export that returns phoneme alignments.
2. Which open TTS models with emotion or style control could run on this laptop (a CPU, and an
   8 GB GPU of which the LLM holds about 5 GB, so 2–3 GB free)? Licence, size, how emotion is
   controlled, published speed, streaming, a Rust path, and whether a voice needs a real
   person's recording (ruled out by the synthetic-data rule; a stock voice shipped with the
   model is acceptable).
3. Which acoustic cues carry sincere concern, empathy or sympathy? Anything on older
   listeners?
4. A ranked shortlist of two or three ways forward, each with its cost to reply latency. Replies
   start 2.6–3.9 s after the resident stops; 1.5 s of that is the turn-end wait, the LLM takes
   1.0–4.6 s and Piper 67–176 ms [S10].

## Answer (short)

1. **Piper has no SSML and no per-word control, but its graph has a clean place for one.**
   SSML was planned in 2023 and never shipped; neither repository contains the string "ssml"
   [S7, S3]. `length_scale` is one scalar that multiplies every predicted phoneme duration
   before rounding up to whole frames (`w = exp(logw) * x_mask * length_scale`,
   `w_ceil = ceil(w)`) [S5]. In the voice this project uses, that is a single node, `/Mul_1`,
   fed by `scales[1]` and feeding the only `Ceil` in the graph (measured). Two routes follow:
   - **Read durations out.** piper1-gpl ships a script that marks the `Ceil` output as a graph
     output, which gives the number of frames for every phoneme id; the patched model still
     works with existing Piper [S4]. Times the hop length (256 samples at 22 050 Hz, 11.6 ms
     per frame; **inferred** from [S3] and the voice config) that is where each word starts
     and ends, so `prosody.rs` could stretch one word after synthesis.
   - **Feed durations in.** Replacing `scales[1]` at `/Mul_1` with a new input of shape
     `[1, 1, phonemes]` gives one length scale per phoneme id. The edited graph passes the ONNX
     checker and shape inference (measured); whether it sounds right is untested.

   Neither route is reachable through piper-rs 0.2.0's API: its inference function is private
   and reads only the first output [S1]. Both need about 60 lines of our own `ort` code
   (**inferred**).

   Phoneme tricks are narrower. The IPA length mark "ː" is in the voice's phoneme map (id 122,
   measured), and piper-rs accepts phonemes directly [S1]. But espeak-ng emits "ː" only after
   some vowels (`sˈɑːɹi`), never after `oʊ` in the phrases tried, and spelling "nooo" gives
   `nˈuːoʊ`, a different vowel (measured). The hands-on session is listening to these. The
   same measurement shows piper-rs 0.2.0 drops every comma and full stop: "Oh no, I'm sorry to
   hear that." reaches the voice as `ˈoʊ nˈoʊaɪm sˈɑːɹi tə hˈɪɹ ðˈæt` (measured), which
   Piper's own phonemizer would have kept [S3]. The hands-on session found the same and is
   fixing it.

2. **Few expressive models fit, and the most expressive ones do not.** Of the models checked,
   those that fit 2–3 GB of GPU memory or run on the CPU, and have a Rust path, are:
   - Kokoro-82M: no emotion control; sherpa-onnx [S11, S13].
   - Chatterbox-Turbo (350M): inline tags including `[sigh]`, `[whispering]` and `[crying]`, a
     built-in voice, and an official ONNX export of about 1.7 GB in fp16 [S15, S16].
   - Chatterbox-Nano (110M, CPU): the same tags; no ONNX export found.
   - Pocket TTS (100M, CPU): style only through the choice of voice clip; sherpa-onnx [S26, S14].
   - Parler-TTS Mini Expresso (647M): a text description with "sad" and "emphasis"; candle
     [S22, S36].

   Qwen3-TTS 1.7B has the best-documented instruction control over emotion and can design a
   voice from a description with no recording [S33]. It needs about 4.5 GB in BF16, and its
   0.6B model has no instruction control. The models with word-level emotion tags and the best
   arena scores (Breeze TTS 2, Fish Audio S2 Pro, Maya1, Voxtral TTS, Step-Audio-EditX) need
   7.7–16 GB of GPU memory, and several are non-commercial [S38–S42]. No fetched source gives a
   speed for any candidate on a laptop GPU of this class.

3. **Concern sounds slow, soft, low and steady, and voice quality may matter more than
   pitch.**
   - In a review of 104 studies, tenderness and sadness shared one pattern: slow rate, low
     intensity, low F0, little F0 variability, falling contours and little high-frequency energy
     [S48].
   - Oncologists slowed down, and most lowered their pitch, when giving bad news. Listeners who
     could not hear the words rated that speech as more caring and sympathetic [S49].
   - For an empathetic healthcare-robot voice, "apologetic" and "worried" were the low-arousal
     secondary emotions: lower F0, lower intensity, 2.93–2.99 syllables/s against 3.24 for
     "enthusiastic" [S50].
   - Breathy and lax-creaky voices cued low-activation states such as relaxed, intimate, content
     and sad. The authors propose that voice quality carries mild attitudes more than pitch
     does [S53].
   - English speakers lengthen an emphasised vowel in graded steps, and the first step is the
     largest [S52].

   For older listeners, the evidence warns against the obvious fixes:
   - Slowing synthetic speech overall made word identification worse at every age [S56].
   - High pitch and a slow rate did not help older adults, and made them report more
     communication problems [S55].
   - A dialogue system for seniors slowed its speech by adding pauses, not by stretching [S58].

   **Inferred:** #37's Warm tone (+1 semitone, range ×1.5) is nearer the literature's happiness
   pattern than its tenderness pattern. Steady's `length_scale` 1.2 slows the whole reply, not
   one word.

4. **Shortlist, ranked by cost to latency:**
   1. **Per-phoneme durations inside Piper**, plus the cues from item 3: a pause after
      "Oh no,", the vowel of "no" held, and that phrase softer and lower instead of the whole
      reply slower. Latency cost: about 0 ms of compute (**inferred**). The only added time is
      the held vowel itself, which plays after the reply has started.
   2. **Pre-rendered openers.** A handful of fixed sympathy phrases, tuned by hand once with
      route 1 and played from memory. The agent already renders its fixed lines at startup this
      way. Latency cost: 0 ms at runtime. Coverage is limited to the phrases chosen.
   3. **An offline bake-off of expressive models** before integrating any: Chatterbox-Turbo
      through `ort`, Kokoro through sherpa-onnx, and possibly Parler Mini Expresso through
      candle. Latency cost: unknown until measured. Published figures point to roughly 0.5–1.7 s
      per reply without streaming, against Piper's 67–176 ms (**inferred**; see
      [Shortlist](#4-shortlist)).

---

## 1. Word-level control in Piper

### What the three code bases expose

**piper-rs 0.2.0** (the crate the agent uses):
- `Piper::create(text, is_phonemes, speaker_id, length_scale, noise_scale, noise_w)` takes
  either text or a phoneme string (`src/lib.rs:74-103`) [S1].
- Text goes through `espeak_rs::text_to_phonemes(...).join(" ")` (`src/lib.rs:86`).
- `phonemes_to_ids` maps each character through the voice's `phoneme_id_map` and silently
  skips any character that is not in the map (`src/model.rs:50-54`).
- `infer` builds `scales = [noise_scale, length_scale, noise_w]` (`src/model.rs:73`). It returns
  only `outputs[0]`, the audio (`src/model.rs:100`).
- The module holding `infer` is private (`src/lib.rs:1`, `mod model;`). A caller cannot pass
  extra inputs or read extra outputs without its own `ort` code (**inferred**).

**OHF-Voice/piper1-gpl** (where Piper development moved; rhasspy/piper's README now says only
"Development has moved" [S6]):
- `SynthesisConfig` has one `length_scale`, `noise_scale`, `noise_w_scale` and `volume` per
  call (`src/piper/config.py:133-151`) [S3].
- Text can mix in raw phonemes in `[[ ... ]]` blocks (`src/piper/voice.py:274-293`;
  `docs/CLI.md:56-75`).
- `--sentence-silence` adds silence between sentences (`docs/CLI.md:52`).
- Its espeak phonemizer appends each clause's punctuation to the phonemes, plus a space after a
  comma (`src/piper/phonemize_espeak.py:51-55`).
- It adds a pad after the start symbol (`src/piper/phoneme_ids.py:191-192`); piper-rs does not
  (`src/model.rs:49`) [S1]. The effect of that difference is unknown.

**rhasspy/piper** (archived) [S6]:
- Its C++ engine could add a set silence after chosen phonemes: `phoneme_silence` in the voice
  config splits a sentence into phrases there (`src/cpp/piper.cpp:174-191, 508-528`).
- No per-word duration or pitch input.

**SSML: none.**
- In the SSML issue, the maintainer wrote in 2023 that the next version "should support breaks
  (pauses), word/phoneme substitutions, and some say-as forms". Laughter and sighs were ruled
  out because "those would have had to be present in the original datasets" [S7].
- A code search of both repositories for "ssml" returned nothing on the fetch date [S3, S6].
- On emotion, the maintainer's two suggestions were a multi-speaker voice with one "speaker" per
  emotion, or new "phonemes" marking emotion. Both need an emotion-labelled dataset [S8].

### How `scales` and `length_scale` act inside the graph

The ONNX export wraps `infer` so that `scales[0]`, `scales[1]` and `scales[2]` become
`noise_scale`, `length_scale` and `noise_scale_w` (`src/piper/train/export_onnx.py:57-68`),
with one output named `output` (`:98-99`) [S5]. Inside `infer`
(`src/piper/train/vits/models.py:791-802`):

```python
logw = self.dp(x, x_mask, g=g, reverse=True, noise_scale=noise_scale_w)   # duration predictor
w = torch.exp(logw) * x_mask * length_scale                               # frames per phoneme id
w_ceil = torch.ceil(w)
attn = commons.generate_path(w_ceil, attn_mask)                          # expand to frames
```

So:
- `noise_w` is the noise of the stochastic duration predictor, which makes phoneme lengths
  vary.
- `length_scale` scales every phoneme id's duration uniformly.
- `noise_scale` scales the noise added to the prior before the flow and decoder
  (`models.py:811`).

All three are per call, not per phoneme [S5]. rhasspy/piper has the same lines
(`vits/models.py:702-709`) [S6].

In `en_US-hfc_female-medium.onnx` (measured, appendix M2):
- The graph has 2 755 nodes and inputs `input`, `input_lengths` and `scales` (no speaker id).
- There is exactly one `Ceil` node (`/Ceil`). Its input is `/Mul_1`, whose two operands are
  `exp(logw) * x_mask` (`/Mul`) and `scales[1]` (`/Gather_1`).
- `scales[1]` has no other consumer.

### Phoneme input: the length mark and "Oh no" (measured)

The voice's `phoneme_id_map` has 159 entries, including `ː` (U+02D0) as id 122, the stress marks
`ˈ` (120) and `ˌ` (121), and `,` `.` `!` `?` (measured from `en_US-hfc_female-medium.onnx.json`
[S9]). espeak-ng, called exactly as piper-rs 0.2.0 calls it, gives:

| Text | Phonemes the voice receives |
|---|---|
| `Oh no` | `ˈoʊ nˈoʊ` |
| `Oh no, I'm sorry to hear that.` | `ˈoʊ nˈoʊaɪm sˈɑːɹi tə hˈɪɹ ðˈæt` |
| `Oh, no.` | `ˈoʊnˈoʊ` |
| `Oh nooo` | `ˈoʊ nˈuːoʊ` |
| `I'm so sorry.` | `aɪm sˌoʊ sˈɑːɹi` |

What this shows (the first three points measured):
- espeak writes `ː` after `ɑ` (`sˈɑːɹi`), `u` and `ɜ`, but after `oʊ` in none of these phrases.
  So `noʊː` would be a symbol sequence the voice probably never saw in training (**inferred**;
  the training transcripts were not checked).
- Spelling the word longer changes the vowel to `uː`.
- No clause punctuation survives, and the word boundary after the comma disappears
  (`nˈoʊaɪm`). The cause is in espeak-rs 0.2.0: it calls `espeak_TextToPhonemes`
  (`src/lib.rs:137-142`), which returns no terminator. The vendored espeak-ng has a separate
  `TranslateClauseWithTerminator` for that (`translate.c:922-923`) [S2]. The crate's own
  comment (`lib.rs:156`) and test (`lib.rs:184`, expecting `tˈɛst.`) assume the opposite; here
  `test` came back as `tˈɛst`.
- Piper's own phonemizer keeps the comma, so its voices were presumably trained with pauses at
  commas (**inferred** from [S3]).

For this ticket the lost comma matters directly: the pause after "Oh no," is one of the cues
in section 3 (**inferred**).

Repeating a phoneme (`noʊoʊ`) or inserting `ː` are the cheap tricks the hands-on session is
measuring. Their effect on duration and naturalness is unknown here.

### Reading durations out: piper1-gpl's alignment patch

`python3 -m piper.patch_voice_with_alignment model.onnx` finds the graph's single `Ceil` node
and appends its output to the graph outputs. It fails if there are several (the file,
`:31-57`) [S4]. The docs say:
- "Patched ONNX models should still work fine with existing Piper installations"
  (`docs/ALIGNMENTS.md:14`).
- The extra output is "the number of audio samples for each phoneme id" (`:4`).
- It is converted from frames by multiplying by the hop length
  (`src/piper/voice.py:570-573`) [S3], which defaults to 256 (`config.py:11`).

The ids come as `[BOS, p1, PAD, p2, PAD, ..., EOS]` (`docs/ALIGNMENTS.md:47-51`), so a word's
samples are the sum over its phoneme ids and their pads [S4]. The feature arrived in 1.3.1
(experimental), and in-memory patching in 1.5.0 (`CHANGELOG.md`) [S3].

For this voice the autodetect would succeed, since there is one `Ceil` (**inferred** from the
measurement above). What it would take:
1. Run the patch once, in Python, with the `onnx` package.
2. In Rust, read `outputs[1]` next to the audio and map ids back to words.
3. Give `prosody.rs` a time-stretch factor for one span.

Its `Shape` has only a pitch shift and a range today (`agent/src/prosody.rs`, `struct Shape`),
so step 3 is new code (**inferred**).

### Feeding durations in: a per-phoneme `length_scales` input

No Piper repository offers this. It is a small graph edit (measured structurally, appendix M3):
- Add a float input `length_scales` of shape `[1, 1, phonemes]`, and make it `/Mul_1`'s second
  operand in place of `scales[1]`.
- Expose `/Ceil` as a second output.

The edited model passes `onnx.checker` and shape inference. It has not been run, so whether
ONNX Runtime accepts it and how a ×2 or ×3 vowel sounds are unknown.

An alternative with the same effect is to re-export from a checkpoint with a changed
`infer_forward` [S5]. That needs the voice's training checkpoint, which the project does not
have (**inferred**).

The attraction over stretching afterwards (**inferred**): the voice itself renders the longer
vowel, so coarticulation and the vocoder stay natural, and pitch reshaping in `prosody.rs`
still applies afterwards. The risk: VITS was trained on natural durations, and a much longer
vowel may come out buzzy or flat. Only listening can tell.

### The routes side by side

| Route | Where | What it can do | Extra latency | Unknown |
|---|---|---|---|---|
| Put punctuation back | text → phonemes | pauses at commas and full stops | none (**inferred**) | being measured by the hands-on session |
| `ː` or a repeated vowel | phoneme string | maybe a longer vowel | none | whether the voice honours unseen sequences |
| Synthesise "Oh no," alone with its own `length_scale` | two Piper calls | whole phrase slower | one extra call (Piper is 67–176 ms per reply [S10]) | a join between the two clips |
| Alignment output + TD-PSOLA stretch | patched model + `prosody.rs` | one word longer, any factor | a few ms of DSP (**inferred**) | PSOLA artefacts on a held vowel |
| Per-phoneme `length_scales` input | edited model + own `ort` code | any phoneme longer or shorter | about 0 (**inferred**) | whether VITS renders a long vowel cleanly |

---

## 2. Expressive local TTS models

### What "fits" means here (inferred)

- The GPU has 2–3 GB free next to the LLM, and the published weight file sizes are the only
  memory figure available for most models. So "fits" below means weights under about 2.5 GB at
  the precision the project would run, before activations. This is **inferred**; actual memory
  is unknown until measured.
- The CPU is free during TTS, since whisper runs before the LLM and the LLM runs on the GPU
  (**inferred** from #18's pipeline [S10]).
- A voice must be a stock voice shipped with the model, or designed from a text description.
  A model that only clones from a reference clip needs a synthetic clip. Whether a clip
  rendered by another model's stock voice is acceptable is a question for the maintainer, not
  settled here.

### Comparison

Speeds are as published, on the hardware named. RTF below 1 means faster than real time,
except in the Kyutai paper [S25], whose RTF is audio seconds per compute second (higher is
faster); those cells say so.

| Model | Weights licence | Size | Emotion / style control | Voice without a real person's recording | Published speed (hardware) | Streaming | Rust path | Fits here? (**inferred**) |
|---|---|---|---|---|---|---|---|---|
| Piper `hfc_female-medium` (current) | per voice | 63 MB ONNX | none per word (section 1) | stock voice | 67–176 ms per reply on this laptop's CPU [S10] | no | piper-rs (in use) | yes |
| Kokoro-82M [S11, S12] | Apache-2.0 | 82M; 327 MB `.pth`, ONNX 163 MB fp16 | voice choice (54 voices) and speed only; inline phoneme overrides `[word](/IPA/)` | stock voices | Raspberry Pi 4 RTF 7.64 (1 thread), 3.19 (4 threads); Piper lessac-medium 0.77 and 0.36 on the same board [S14] | unknown | sherpa-onnx Rust API [S13] | yes, CPU or GPU |
| Chatterbox (original) [S15, S16] | MIT | 500M | `exaggeration` knob (one scalar per utterance) and `cfg_weight` | built-in `conds.pt` | RTF 1.8 (higher is faster) on an H100 [S25] | not stated | none official | GPU fp32 weights about 3.2 GB; tight |
| Chatterbox-Turbo [S15, S16] | MIT | 350M; ONNX fp16 about 1.66 GB, q4 about 0.72 GB | inline tags: `[sigh]`, `[whispering]`, `[crying]`, `[gasp]`, `[happy]`, `[angry]`, `[fear]`, `[sarcastic]`, `[laugh]`, `[chuckle]`, ...; no `exaggeration` | built-in `conds.pt` | none published | not stated | official ONNX via the `ort` crate; the generation loop must be ported | yes (fp16 or q4) |
| Chatterbox-Nano [S15, S16] | MIT | 110M | same tags as Turbo | built-in `conds.pt` | "3x faster than realtime on 8 CPU cores" (CPU not named) | not stated | none found | yes, CPU |
| Pocket TTS [S26] | CC-BY-4.0 | 100M | the voice clip or embedding chosen; nothing else | stock voices (precomputed embeddings) | about 200 ms to first chunk, about 6× real time, 2 cores of a MacBook Air M4 | yes | sherpa-onnx Rust API (int8) [S14], candle port, llama.cpp [S35] | yes, CPU |
| Parler-TTS Mini v1 / Mini Expresso [S22] | Apache-2.0 | 880M / 647M | text description; Expresso adds happy, confused, laughing, sad, whisper, emphasis | 34 named speakers / 4 Expresso speakers | "under 500ms on a modern GPU" to first audio (GPU not named) | yes | candle example [S36] | Expresso fp16 about 1.3 GB, maybe |
| Qwen3-TTS 1.7B CustomVoice / VoiceDesign [S33, S34] | Apache-2.0 | 1.92B; 3.8 GB BF16 + 0.68 GB tokenizer | natural-language instructions (tone, emotion, pace) over 9 stock speakers, or a voice designed from a description | stock voices (English ones are male) or designed from text | first packet 101 ms, RTF 0.313 with vLLM on "a single typical computational resource" (not named) | yes | llama.cpp `llama-tts` for the Base (clone) model only [S35] | no (about 4.5 GB) |
| Qwen3-TTS 0.6B CustomVoice [S33] | Apache-2.0 | 0.91B; 1.8 GB + 0.68 GB | none: "Instruction Control" not ticked | stock voices | first packet 97 ms (same setup) | yes | as above | borderline |
| Kyutai TTS 1.6B [S24, S25] | CC-BY-4.0 | 1.8B; 3.7 GB | a precomputed voice embedding; the voice repo includes Expresso "sad-sympathetic" and "calm" clips | stock embeddings; Expresso and EARS are CC-BY-NC | latency 150 ms, 3.2× real time, H100, batch 1 | yes, text in and audio out | Rust `moshi-server` (install needs its Python side) | no; the 0.75B variant is 1.9 GB |
| CosyVoice 2 / Fun-CosyVoice3 0.5B [S27] | Apache-2.0 | 0.5B; 4.9 GB of files | instructions (Calm, Sad, slow ...), `<strong>word</strong>` emphasis, `[breath]` | v2/v3 instructions take a prompt clip; the older 300M-SFT/Instruct have built-in speakers | "as low as 150ms" (hardware not stated) | yes | none official | unknown |
| Orpheus 3B [S17] | Apache-2.0 | 3.78B; Q4_K_M GGUF 2.36 GB (third party) | tags `<sigh>`, `<sniffle>`, `<laugh>`, ... | stock voices (tara, leah, ...) | "~200ms streaming latency" (hardware not stated); RTF 0.7 (slower than real time) on an H100 [S25] | yes | candle example [S36] | no: too slow |
| Dia 1.6B / Dia2 [S18] | Apache-2.0 | 1.6B / 1–2B | tags `(sighs)`, `(inhales)`, `(exhales)`, ...; tone from an audio prompt | none: a new voice each run unless prompted or seeded | RTF ×2.1 at about 4.4 GB, RTX 4090 bf16 compiled; 0.7 on an H100 [S25] | Dia2 yes | none | no |
| Sesame CSM-1B [S28] | Apache-2.0 | 1.55B | context audio | none ("not been fine-tuned on any specific voice") | RTF 1.0 on an H100 [S25] | not stated | candle example [S36] | no |
| Zonos v0.1 [S29] | Apache-2.0 | 1.6B | emotion vector (happiness, sadness, fear, anger ...), rate, pitch variation | speaker embedding from a 10–30 s clip | RTF about 2× on an RTX 4090; "6GB+ VRAM" | not stated | none | no |
| StyleTTS 2 [S19, S20] | MIT code | 750 MB checkpoint | style sampled from the text; a reference for other voices | LJSpeech single-speaker model | RTF 0.0185 on an RTX 2080 Ti (VITS 0.0599) | not stated | none (Kokoro is its descendant) | yes |
| F5-TTS [S21] | CC-BY-NC-4.0 | 336M | only through the reference clip | always clones a reference | RTF 0.147 PyTorch on an L20 | no | community ONNX | ruled out: licence and reference |
| XTTS-v2 [S31] | CPML (non-commercial) | 1.9 GB | "Emotion and style transfer by cloning" | reference clip | "<200ms" streaming (hardware not stated) | yes | none | ruled out: licence |
| Bark [S30] | MIT | 12 GB VRAM full, 8 GB small | tags `[sighs]`, `[laughs]`, ... | 100+ speaker presets | "roughly real-time" on enterprise GPUs | no | none | no |
| Matcha-TTS [S32] | MIT | about 71 MB ONNX | none | LJSpeech | Pi 4 RTF 0.94 (1 thread) [S14] | no | sherpa-onnx | yes, no gain |

These were checked and are outside the budget. Each is listed with its most expressive
feature:

- **Breeze TTS 2** (3.5B, research and non-commercial only): voice design from text,
  `(sigh)`-style events, 7.7 GiB of GPU memory eager. First on the arena below [S38].
- **Fish Audio S2 Pro** (4.4B + 0.4B, research licence): free-form word-level tags such as
  `[emphasis]`, `[sad]`, `[sigh]`, `[pause]`; RTF 0.195 on an H200 [S39].
- **Maya1** (3.3B, Apache-2.0): a voice from a description plus `<sigh>`, `<cry>`,
  `<whisper>`; "16GB+ VRAM" [S40].
- **Step-Audio-EditX** (3.5B, Apache-2.0): edits the emotion or style of existing audio
  (sad, gentle, soulful ...); 12 GB, or 6–8 GB with AWQ 4-bit [S41].
- **Voxtral TTS** (4B, CC-BY-NC): 20 preset voices; 16 GB or more [S42].
- **NVIDIA Magpie Multilingual 357M**: five English voices, cloning removed, no documented
  emotion control [S43].
- **Supertonic 3** (99M ONNX, `<sigh>` and `<breath>` tags, in sherpa-onnx): its repository
  is archived and development has ended [S44].
- **IndexTTS-2.5**: 8-way emotion vector and a `duration_factor`, but always needs a speaker
  clip; bilibili licence [S45].
- **Spark-TTS**: pitch, speed and gender levels only; CC-BY-NC-SA [S46].
- **Kitten TTS** (15–80M, no emotion control) [S47].

### Quality evidence

The Artificial Analysis open-weights arena ranks models by blind preference between each
provider's own voices [S37]:

| Rank | Model | Elo |
|---|---|---|
| 1 | Breeze TTS 2 | 1205 |
| 5 | Magpie-Multilingual 357M | 1063 |
| 6 | Kokoro 82M v1.0 | 1063 |
| 7 | Maya1 | 1044 |
| 10 | Chatterbox | 1023 |
| 11 | Zonos-v0.1 | 1000 |
| 14 | XTTS v2 | 916 |
| 15 | StyleTTS 2 | 892 |

Piper, Pocket TTS, Qwen3-TTS, Chatterbox-Turbo and Parler are not on the list. The page did not
show a date. It measures general preference, not sincerity or empathy. No fetched benchmark
measures perceived empathy for any of these models.

On how reliable instruction control is: a 2026 perception study built its stimuli with
Qwen3-TTS-1.7B-CustomVoice. It prompted for slow, flat or soft delivery, and still generated
100 candidates per utterance and condition, keeping those whose measured acoustics matched
[S54]. Instruction following is therefore a distribution to sample from, not a precise knob
(**inferred**).

### Notes on the candidates that fit

- **Chatterbox-Turbo** is the closest match to "say this line with a sigh" among the models
  that fit.
  - Its tokenizer adds 19 bracket tags, among them `[sigh]`, `[whispering]`, `[crying]` and
    `[happy]`. There is no `[sad]` or `[sympathetic]` tag [S16].
  - Turbo ignores `exaggeration` and CFG (`tts_turbo.py:290-291`) [S15].
  - The built-in voice loads from `conds.pt` when present (`tts_turbo.py:183-186`). Resemble's
    ONNX example instead encodes a target voice wav with `speech_encoder` [S16]. So using the
    built-in voice from Rust means either exporting `conds.pt` to the ONNX inputs or encoding a
    synthetic clip once (**inferred**).
  - Speed is unpublished. The original Chatterbox made audio 1.8× faster than real time on an
    H100 [S25]; Turbo's decoder needs one step instead of ten [S15].
- **Pocket TTS** is the only candidate built for the CPU with a published speed [S26].
  - Its style control is the voice clip. Whether a sad or sympathetic clip carries that tone
    into new text is not stated in any fetched source (unknown).
  - The Expresso clips in Kyutai's voice repository are real speakers' recordings under
    CC-BY-NC [S24]. Using one as a "stock voice" is a licence and rule question for the
    maintainer.
- **Kokoro** has no emotion control. Its Python code exposes per-token durations and word
  timestamps (`kokoro/model.py:107-119`, `pipeline.py:290-318`), and `speed` is one scalar
  divisor of the predicted durations (`model.py:108`) [S12]. It would bring the same word-level
  question as Piper, at about 9–10× Piper's compute on the one board where both were timed
  [S14].
- **Parler-TTS Mini Expresso** is controlled by sentences like "Thomas speaks moderately slowly
  in a sad tone with emphasis" (card, line 57) [S22]. candle's example runs the Large
  checkpoint [S36]; that it runs Mini Expresso is **inferred** from the shared architecture.

---

## 3. Cues that read as sincere concern

### Emotion-level patterns

Juslin & Laukka reviewed 104 studies of vocal expression and 41 of music performance [S48].
Their summary table (Table 11, p. 802) gives tenderness:

> Slow speech rate/tempo, low voice intensity/sound level, little voice intensity/sound level
> variability, little high-frequency energy, low F0/pitch level, little F0/pitch variability,
> falling F0/pitch contours, slow voice onsets/tone attacks, and microstructural regularity

Sadness is the same except for "microstructural irregularity". Happiness is the opposite:
fast, high F0, "much F0/pitch variability", rising contours. They also found that cues
"contribute in an additive fashion": "Each cue is neither necessary nor sufficient, but the
larger the number of cues used, the more reliable the communication" (p. 802).

### Breaking bad news

McHenry, Parker, Baile & Lenzi recorded oncology providers [S49]:
- "All but one provider reduced speaking rate, the majority also reduced pitch" when giving bad
  news.
- 27 listeners heard the speech low-pass filtered, with the words made unintelligible but the
  intonation and rate kept. They rated the slower, lower speech "as more caring and
  sympathetic".

Only the abstract was fetched, so the sizes of the changes are unknown.

### An empathetic healthcare-robot voice

James, Balamurali, Watson & Mixdorff [S50] built on two earlier studies of a healthcare
robot's voice (greetings, medicine reminders, guidance). Those found that "for the voice of a
robot to be perceived as empathetic, not only primary emotions but secondary emotions are also
essential". The ones identified were anxious, apologetic, confident, enthusiastic and worried.

Their measurements on the JL corpus:
- The low-arousal "apologetic" and "worried" had the lowest intensity (55.14 and 56.34 dB) and
  the slowest rates (2.93 and 2.99 syllables/s), against 3.24 and 63.91 dB for "enthusiastic"
  (Table 3).
- Their F0 was low as well.

When resynthesised by rule (Fujisaki F0 contours, rate, intensity), the emotions were
identified pairwise at 87% on average (29 listeners). But apologetic and worried were often
confused, "indicat[ing] that synthesising emotions such as apologetic and worried may require
modelling other acoustic features". 24 of the 29 listeners were aged 16–35 and none over 65.
The earlier robot study itself (James et al., 2021) was not fetched (see Unfetched).

### Lengthening

Braver, Dresher & Kawahara had eight English speakers read degree adverbs spelled with more and
more letters ("so", "soo", ... "soooooo") [S52]:
- All produced longer vowels at higher emphasis levels (p < .001).
- The largest jump was always from no emphasis to the first level: 285 ms and 389 ms for
  speakers 1 and 2 (Table 3).
- After that, each level added 22–92 ms depending on the speaker (Table 2).

This is emphasis on intensifiers, not sympathy on "no". It gives the order of magnitude a
listener hears as deliberately lengthened (**inferred**). At Piper's 11.6 ms per frame, 285 ms
is about 25 frames (**inferred** arithmetic).

### Voice quality

Gobl & Ní Chasaide synthesised one utterance with seven voice qualities for 12 listeners [S53]:
- The breathy, whispery, creaky and "especially the lax–creaky" versions were associated with
  low-activation states, both positive (relaxed, content, intimate, friendly) and negative (sad,
  bored).
- Lax–creaky got the highest ratings for sad and friendly.
- Their hypothesis: voice quality plays "a critical role in the general communication of milder
  affective distinctions (general speaker states, moods and attitudes)". Large changes in F0
  level and range are more critical for strong emotions.

Concern is a mild attitude, so pitch reshaping alone may be pulling the weaker lever
(**inferred**). Piper has no voice-quality control, which a model with a "gentle" or
"whispering" style might add (**inferred**).

### Sincerity versus sarcasm in synthetic speech

In a controlled study with Qwen3-TTS stimuli, louder renderings were rated significantly more
sarcastic, and loudness was the main cue for human listeners [S54]. That was a sarcasm rating,
not sincerity, but it argues for keeping a sympathy line soft (**inferred**).

### Older listeners

- **Kemper & Harden**, three experiments on elderspeak [S55]:
  - Reducing subordinate and embedded clauses, and adding semantic elaborations, helped older
    adults.
  - "Reducing sentence length, slowing speaking rate, and using high pitch do not."
  - "The use of short sentences, a slow rate of speaking, and high pitch resulted in older
    adults' reporting more communication problems."
- **Roring, Hines & Charness**: 96 participants of three ages identified words in natural and
  synthetic speech. "Slower speech rates worsened performance for all groups", and hearing
  acuity accounted for the age interaction [S56]. Wolters et al. describe the slow condition as
  150 words per minute against 210, made by setting the synthesiser's duration factor to 1.5
  [S57].
- **Wolters et al.**:
  - Older users understood synthetic reminders as well as younger ones when the words were
    familiar.
  - Unfamiliar words (medication names) exposed hearing loss, even in listeners who would pass
    the usual screening.
  - The hearing threshold averaged over 1–3 kHz, "the range of F2", correlated best with
    understanding [S57].
  - That band is inside the telephone band (**inferred**).
- **CMU GetGoing**, a phone dialogue system for seniors, slowed its synthesised speech "by
  inserting pauses into the SSML", not by stretching it [S58].
- **Han et al.**, elder-facing Cantonese TTS rated by 8 older adults: the variant that "overemphasizes the slow
  speaking rate ..., inserting too many pauses and elongating the generated speech" had "among
  the lowest" MOS [S59].

### What this means for the current tones (inferred)

These are inferences for the maintainer to weigh against listening. The maintainer judged the
"sorry" line at ×1.3 and ×1.5 range "pretty good" [S10], and the ear outranks a review of
other voices.

- **Warm raises pitch and widens its range.** Warm's +1 semitone and range ×1.5 move toward the
  review's happiness pattern (high F0, much variability) and away from tenderness (low F0,
  little variability) [S48]. High pitch is also part of the elderspeak that older adults
  reported as a problem [S55]. For a line of sympathy, a lower level and a range no wider than
  Piper's own fit the evidence better.
- **Steady slows the whole reply.** Steady's `length_scale` 1.2 slows every word, which is the
  manipulation that hurt older listeners [S56] and was rated lowest in [S59]. The evidence
  points instead to slowing the key phrase, holding one vowel [S52], and a pause after "Oh no,"
  [S58].
- **Softness and voice quality are the untried cues.** Low intensity [S48, S50] and a softer,
  breathier voice quality [S53] are cues neither tone uses. Intensity can be lowered on one
  phrase after synthesis. Voice quality needs another model.

---

## 4. Shortlist

Today's reply starts 2.6–3.9 s after the resident stops, of which Piper is 67–176 ms [S10].
Every 0.5 s added to TTS is 13–19% more waiting (**inferred** arithmetic). The agent
synthesises a whole reply before playing it, so a streaming TTS helps only if playback starts
on the first chunk. That would be new code (**inferred** from #18's pipeline [S10]).

### 1. Per-phoneme durations inside Piper, with the cues from section 3

**What.**
- Keep the voice. Edit its graph once to take `length_scales` and return `/Ceil`
  (section 1).
- Replace piper-rs's private inference with a small function of our own on the `ort` session.
- Mark which words to hold: for example, the vowel of "no" in "Oh no" ×2–×3, which is
  roughly +150–300 ms (**inferred** from [S52]).
- Keep the restored punctuation, so "Oh no," is followed by a pause.
- Apply the pitch and intensity choices from section 3 to that phrase only.

**Latency cost:** about 0 ms of compute (**inferred**). It is the same network; synthesis time
grows with the audio produced, so +0.3 s of audio adds roughly a tenth to Piper's 67–176 ms.
The held vowel lengthens the reply, not the wait before it.

**Integration:**
- A one-off Python script with the `onnx` package, kept in the repository so the edited model
  can be rebuilt.
- About 60 lines in `tts.rs` in place of `Piper::create`: phonemise, map ids, build three or
  four tensors, run.
- A rule or markup telling which syllable to hold, from the LLM's text or a small phrase list.
- Everything stays in the one Rust process and on the CPU.

**Unknown until measured:**
- Whether ONNX Runtime runs the edited graph.
- Whether a ×2–×3 vowel sounds held or synthetic.
- Which syllables and factors read as sincere rather than drawn out. Most of this needs the
  maintainer's ears.

The alignment output alone plus a TD-PSOLA stretch in `prosody.rs` is the fallback if the
edited graph misbehaves.

### 2. Pre-rendered sympathy openers

**What.**
- Pick a handful of fixed phrases ("Oh no, I'm sorry to hear that.", "Oh, I'm so sorry.").
- Hand-tune each once with route 1: hold the vowel, set the pause, pitch and loudness by ear.
- Store the audio, and have the LLM choose an opener, or none, as a field of its reply.
- The rest of the reply is synthesised as now.

The agent already renders its greeting, holding line and escalation line at startup and plays
them from memory (`agent/src/conversation.rs`, the fields built with `tts.speak(...)`
there).

**Latency cost:** 0 ms at runtime, and the opener is audio that no longer needs synthesising
(**inferred**).

**Integration:**
- A new field in the LLM's structured turn, and a prompt change.
- An audio bank.
- The opener must be the same voice as the rest, so it should come from Piper, via route 1.

**Unknown:**
- Whether a fixed opener sounds canned when it recurs in one call.
- How the join between the stored opener and the live rest sounds.
- Whether the LLM picks openers well.

### 3. An offline bake-off of expressive models, before integrating any

**What.**
1. Render the six check-in lines of #37 with Chatterbox-Turbo (fp16 ONNX, built-in voice, with
   and without `[sigh]`) and Kokoro (sherpa-onnx). Add Parler Mini Expresso ("sad tone with
   emphasis") if it runs in the memory left.
2. Time each on this laptop.
3. Have the maintainer listen through the phone path.

Integrate only a model that is clearly more sincere and adds well under a second.

**Latency cost:** unknown until measured. Rough **inferred** figures:
- **Kokoro:** 0.6–1.7 s per reply on the CPU. That is 9–10× Piper on the Raspberry Pi
  comparison [S14], applied to Piper's 67–176 ms here [S10].
- **Chatterbox-Nano:** about 1 s for a 3 s reply at its published 3× real time [S15].
- **Pocket TTS:** about 0.2 s to first audio if streamed [S26].
- **Chatterbox-Turbo on this GPU:** no published figure.

**Integration:**
- Chatterbox-Turbo: the `ort` crate the agent already links, four sessions, and a ported
  sampling loop. It also shares the GPU with the LLM.
- Kokoro and Pocket TTS: the sherpa-onnx Rust crate, which bundles its own ONNX Runtime. Its
  coexistence with piper-rs's pinned `ort` is unknown.

**Unknown:**
- Speed on this hardware.
- GPU memory with the LLM loaded.
- Whether tags like `[sigh]` sound sincere or theatrical in this voice.
- Whether any stock voice is acceptable to the maintainer.
- A voice change: every model here replaces the voice picked by ear in #25.

**Why third:** it is the only route that could add voice quality (breathiness, softness),
which section 3 suggests matters for mild attitudes [S53]. It is also the only one that risks
seconds of latency.

---

## Sources

All fetched on 2026-09-24 at the URL shown. Repository files were fetched raw at the commit
given.

Piper
- [S1] thewh1teagle/piper-rs, crate `piper-rs` 0.2.0, `src/lib.rs`, `src/model.rs`. Read from
  the crates.io package in the local cargo registry, whose `.cargo_vcs_info.json` names commit
  d70b0970a87453f3476b5fd6cf9edf329be8b445.
  https://github.com/thewh1teagle/piper-rs/tree/d70b0970a87453f3476b5fd6cf9edf329be8b445
- [S2] Same repository, crates `espeak-rs` 0.2.0 (`src/lib.rs`) and `espeak-rs-sys` 0.2.0
  (vendored espeak-ng, `src/libespeak-ng/speech.c:853-870`, `translate.c:922-923`). Read from
  the local cargo registry at the same commit.
- [S3] OHF-Voice/piper1-gpl@5b355b110aecf3de8f4e000ede1ce06831acff35: `src/piper/voice.py`,
  `src/piper/config.py`, `src/piper/phoneme_ids.py`, `src/piper/phonemize_espeak.py`,
  `docs/CLI.md`, `docs/API_PYTHON.md`, `CHANGELOG.md`. The code search for "ssml" was run with
  `gh search code`. https://github.com/OHF-Voice/piper1-gpl/tree/5b355b110aecf3de8f4e000ede1ce06831acff35
- [S4] Same commit: `docs/ALIGNMENTS.md`, `src/piper/patch_voice_with_alignment.py`.
- [S5] Same commit: `src/piper/train/export_onnx.py`, `src/piper/train/vits/models.py`.
- [S6] rhasspy/piper@73c04d81d5590ecc46e522de3601ce7fb29fc2be (archived): `README.md`,
  `src/cpp/piper.cpp`, `src/python/piper_train/export_onnx.py`,
  `src/python/piper_train/vits/models.py`. https://github.com/rhasspy/piper/tree/73c04d81d5590ecc46e522de3601ce7fb29fc2be
- [S7] rhasspy/piper issue #275, "SSML Support?", with comments. https://github.com/rhasspy/piper/issues/275
- [S8] rhasspy/piper issue #150, "Emotions / expressions?", with comments. https://github.com/rhasspy/piper/issues/150
- [S9] `models/en_US-hfc_female-medium.onnx` and `.onnx.json`, the voice the agent uses,
  downloaded by `make models` (`Makefile:40`) from the rhasspy/piper-voices repository.

This project
- [S10] qvd808/agent-emergency-call issue #18, comment with per-turn timings from three live
  calls; issue #37, the maintainer's listening comment. The current tones are
  `Tone::settings` in `agent/src/tts.rs` (working tree on 2026-09-24).

TTS models
- [S11] Model card `hexgrad/Kokoro-82M`@f3ff3571791e39611d31c381e3a41a3af07b4987, and the file
  list of `onnx-community/Kokoro-82M-v1.0-ONNX`@1939ad2a8e416c0acfeecc08a694d14ef25f2231.
  https://huggingface.co/hexgrad/Kokoro-82M
- [S12] hexgrad/kokoro@dfb907a02bba8152ca444717ca5d78747ccb4bec: `README.md`,
  `kokoro/model.py`, `kokoro/pipeline.py`; hexgrad/misaki@fba1236595f2d2bf21d414ba6e57d25256afada3
  `README.md` (line 15, inline phoneme syntax). https://github.com/hexgrad/kokoro
- [S13] k2-fsa/sherpa-onnx@040afe360a38e25daaa325ce8889abf93ea02609 (release v1.13.8):
  `sherpa-onnx/rust/sherpa-onnx/src/tts.rs` (model configs for VITS, Matcha, Kokoro, Kitten,
  ZipVoice, Pocket, Supertonic), `rust-api-examples/`. https://github.com/k2-fsa/sherpa-onnx
- [S14] k2-fsa/sherpa@c02f72ca1540163a54019e845127fa52d5de175b:
  `docs/source/onnx/tts/pretrained_models/rtf.rst` (RTF on a Raspberry Pi 4 Model B),
  `docs/source/onnx/tts/pocket.rst`. https://github.com/k2-fsa/sherpa
- [S15] resemble-ai/chatterbox@5de7a54aa4e5e2baadb0182dde554908b48b85c2: `README.md` (lines
  30, 32, 165-169), `src/chatterbox/tts.py`, `src/chatterbox/tts_turbo.py`,
  `src/chatterbox/models/t3/modules/cond_enc.py`. https://github.com/resemble-ai/chatterbox
- [S16] Hugging Face: `ResembleAI/chatterbox-turbo`@749d1c1a46eb10492095d68fbcf55691ccf137cd
  (card, `added_tokens.json`, file list); `ResembleAI/chatterbox-turbo-ONNX`@d21799bd0354adb85e348b8a0442a8405110a2cf
  (card, ONNX file sizes); `ResembleAI/chatterbox-nano`@71ccd1d0081b430592cea481f4307e764e07bc64;
  `ResembleAI/chatterbox`@5bb1f6ee58e50c3b8d408bc82a6d3740c2db6e18.
- [S17] canopyai/Orpheus-TTS@e64661fe6d02c414fc77c53578c9d64082614861 `README.md` (lines 18-21,
  108, 113, 163); Hugging Face metadata for `canopylabs/orpheus-3b-0.1-ft` (card gated) and
  `isaiahbjork/orpheus-3b-0.1-ft-Q4_K_M-GGUF`. https://github.com/canopyai/Orpheus-TTS
- [S18] nari-labs/dia@876125e461a03b157ec905b0fe8b57a0f8b9e7a0 `README.md` (lines 165, 176,
  183-192); nari-labs/dia2@8687268f4ed3ed20704638fd353b51491de3b476 `README.md` (lines 16, 58).
  https://github.com/nari-labs/dia
- [S19] yl4579/StyleTTS2@5cedc71c333f8d8b8551ca59378bdcc7af4c9529 `README.md`.
- [S20] Y. A. Li, C. Han, V. S. Raghavan, G. Mischler, N. Mesgarani, "StyleTTS 2: Towards
  Human-Level Text-to-Speech through Style Diffusion and Adversarial Training with Large
  Speech Language Models", arXiv:2306.07691 (Table 4; RTF on an RTX 2080 Ti, Appendix).
  https://arxiv.org/pdf/2306.07691
- [S21] SWivid/F5-TTS@283252563dbf91be625e0c27926acfaac449186c `README.md` (lines 131-137,
  276-278); model card `SWivid/F5-TTS`. https://github.com/SWivid/F5-TTS
- [S22] huggingface/parler-tts@d108732cd57788ec86bc857d99a6cabd66663d68 `README.md` (lines
  11-12, 82-85), `INFERENCE.md` (line 3); model cards `parler-tts/parler-tts-mini-v1` and
  `parler-tts/parler-tts-mini-expresso` (lines 29, 57, 69-70). https://github.com/huggingface/parler-tts
- [S24] kyutai-labs/delayed-streams-modeling@4c4f65e147df056adf3346290d64c7b9649b18c9
  `README.md` (lines 218, 258); model card `kyutai/tts-1.6b-en_fr` (lines 22, 41, 49, 58);
  `kyutai/tts-voices`@323332d33f997de8394f24a193e1a76df720e01a `README.md` (lines 45-49) and
  file list. https://huggingface.co/kyutai/tts-1.6b-en_fr
- [S25] N. Zeghidour et al., "Streaming Sequence-to-Sequence Learning with Delayed Streams
  Modeling", arXiv:2509.08753, Table 6 (latency and RTF of DSM-TTS, Dia, CSM, Orpheus and
  Chatterbox on one H100). https://arxiv.org/pdf/2509.08753
- [S26] kyutai-labs/pocket-tts@3dbee45d343d7dddd0d105468d17f8dcba14db3e `README.md` (lines
  23-30, 62-89, 256-267); model card `kyutai/pocket-tts-without-voice-cloning` (line 301);
  S. Rouard et al., "Continuous Audio Language Models", arXiv:2509.06926.
  https://github.com/kyutai-labs/pocket-tts
- [S27] FunAudioLLM/CosyVoice@074ca6dc9e80a2f424f1f74b48bdd7d3fea531cc `README.md` (lines 19-20),
  `example.py` (lines 30-31, 57, 86); Z. Du et al., "CosyVoice 2: Scalable Streaming Speech
  Synthesis with Large Language Models", arXiv:2412.10117, section 2.6 and Table 1.
  https://github.com/FunAudioLLM/CosyVoice
- [S28] SesameAILabs/csm@daed31e6d42cf71873999075de204fa37d2acec3 `README.md` (lines 59,
  131-133). https://github.com/SesameAILabs/csm
- [S29] Zyphra/Zonos@bc40d98e1e1ab54fc65c483be127a90e3c7c0645 `README.md` (lines 21, 84, 93);
  model cards `Zyphra/Zonos-v0.1-transformer`, `Zyphra/ZONOS2`. https://github.com/Zyphra/Zonos
- [S30] suno-ai/bark@f4f32d4cd480dfec1c245d258174bc9bde3c2148 `README.md` (lines 236-239,
  249-251, 312). https://github.com/suno-ai/bark
- [S31] Model card `coqui/XTTS-v2`@6c2b0d75eae4b7047358e3b6bd9325f857d43f77;
  idiap/coqui-ai-TTS@ca2cf5155bca892ea820ad384400efbfac41b178 `README.md` (line 30).
  https://huggingface.co/coqui/XTTS-v2
- [S32] shivammehta25/Matcha-TTS@bd4d90d93214b37f7a159cf205ae85762c2c10aa `README.md`.
  https://github.com/shivammehta25/Matcha-TTS
- [S33] QwenLM/Qwen3-TTS@022e286b98fbec7e1e916cb940cdf532cd9f488e `README.md` (lines 54-55,
  73-79, 150, 187-199, 290); Hugging Face metadata for `Qwen/Qwen3-TTS-12Hz-1.7B-CustomVoice`,
  `-1.7B-VoiceDesign`, `-0.6B-CustomVoice`. https://github.com/QwenLM/Qwen3-TTS
- [S34] H. Hu et al., "Qwen3-TTS Technical Report", arXiv:2601.15621, section 3.4 and Table 2.
  https://arxiv.org/pdf/2601.15621
- [S35] ggml-org/llama.cpp@84e76d8a23162eca70490da131945ebec1f09bf4 `tools/tts/README.md`;
  file list of `ggml-org/Qwen3-TTS-12Hz-1.7B-Base-GGUF`. https://github.com/ggml-org/llama.cpp
- [S36] huggingface/candle@66a8cf184a5a519671454066b1b9efd446ec9f5c:
  `candle-examples/examples/{orpheus,csm,parler-tts}/README.md`. https://github.com/huggingface/candle
- [S37] Artificial Analysis, "Text to Speech Open Weights Leaderboard". Read through a
  summarising fetch tool; the page showed no date.
  https://artificialanalysis.ai/text-to-speech/leaderboard/provider-voice/open-weights
- [S38] Model card `BreezeBlue/Breeze-TTS-2`@3e28c5151381a722f1d8661b4118c298caa77aa4 (lines
  30, 48-53, 62).
- [S39] Model card `fishaudio/s2-pro`@1de9996b6be38b745688de084d87a5633f714e4e (lines 115-126,
  140-142, 170).
- [S40] Model card `maya-research/maya1`@21c682a0afef8c13a89b2512733c8bf5f0c52eb7 (lines 20-21,
  138, 479-480).
- [S41] Model card `stepfun-ai/Step-Audio-EditX`@5fe2f8a05c2353301ad47d3c1747b262115da138
  (lines 26, 71-74); stepfun-ai/Step-Audio-EditX@a652e87052c109e26f616d60971376ff47a829d4
  `README.md` (lines 425-431, 585).
- [S42] Model card `mistralai/Voxtral-4B-TTS-2603`@b81be46c3777f88621676791b512bb01dc1cb970
  (lines 27, 40, 69-76, 119).
- [S43] Model card `nvidia/magpie_tts_multilingual_357m`@19806879b16d3f2ccf28fb112b1bcd16a3c7923e
  (lines 53, 79, 86).
- [S44] Model card `Supertone/supertonic-3`@3cadd1ee6394adea1bd021217a0e650ede09a323 (line 86);
  supertone-inc/supertonic@1e9799e964ea4c0dad7cde993b65c3c813a7b373 `README.md` (line 1:
  "This repository is archived").
- [S45] index-tts/index-tts@ee40fa7d6c6b8a2c7f06105f9f1e65775b74868c `README.md` (lines 38-44,
  242-298); Hugging Face metadata for `IndexTeam/IndexTTS-2.5` (licence `bilibili-model-license`).
- [S46] SparkAudio/Spark-TTS@2f1ea9082400547242641f5271b6f941c9f439d1 `README.md` (lines 45,
  150-156); Hugging Face metadata for `SparkAudio/Spark-TTS-0.5B` (licence cc-by-nc-sa-4.0).
- [S47] KittenML/KittenTTS@be5758500b731b8fc674acc62ea480d3022b7ebe `README.md` (lines 16,
  37-51).

Cues of concern, and older listeners
- [S48] P. N. Juslin, P. Laukka, "Communication of emotions in vocal expression and music
  performance: Different channels, same code?", Psychological Bulletin 129(5) (2003) 770–814,
  doi:10.1037/0033-2909.129.5.770. The fetched PDF holds 10 of the 45 pages, including Table 11
  (p. 802). https://www.brainmusic.org/EducationalActivities/Juslin_emotion2003.pdf
- [S49] M. McHenry, P. A. Parker, W. F. Baile, R. Lenzi, "Voice analysis during bad news
  discussion in oncology: reduced pitch, decreased speaking rate, and nonverbal communication
  of empathy", Supportive Care in Cancer (2012), doi:10.1007/s00520-011-1187-8, PMID 21573770.
  Abstract only, from the Europe PMC API.
  https://www.ebi.ac.uk/europepmc/webservices/rest/search?query=PMCID:PMC12969286&format=json&resultType=core
- [S50] J. James, B. T. Balamurali, C. Watson, H. Mixdorff, "Exploring Prosodic Features
  Modelling for Secondary Emotions Needed for Empathetic Speech Synthesis", Sensors 23(6)
  (2023) 2999, doi:10.3390/s23062999, PMC10053518. Full text from the Europe PMC API.
  https://www.ebi.ac.uk/europepmc/webservices/rest/PMC10053518/fullTextXML
- [S52] A. Braver, N. Dresher, S. Kawahara, "The Phonetics of Emphatic Vowel Lengthening in
  English", Proceedings of the Annual Meetings on Phonology 2 (Phonology 2014),
  doi:10.3765/amp.v2i0.3754.
  https://journals.linguisticsociety.org/proceedings/index.php/amphonology/article/download/3754/3473
- [S53] C. Gobl, A. Ní Chasaide, "The role of voice quality in communicating emotion, mood and
  attitude", Speech Communication 40 (2003) 189–212, doi:10.1016/S0167-6393(02)00082-1.
  http://www.cs.columbia.edu/~julia/papers/gobl03.pdf
- [S54] Z. Li, S. Nayak, M. Coler, "What Makes Synthetic Speech Sound Sarcastic? A
  Prosody-Controlled Perception Study", arXiv:2606.09717. https://arxiv.org/pdf/2606.09717
- [S55] S. Kemper, T. Harden, "Experimentally disentangling what's beneficial about elderspeak
  from what's not", Psychology and Aging 14(4) (1999) 656–670,
  doi:10.1037//0882-7974.14.4.656, PMID 10632152. Abstract only, from the Europe PMC API.
- [S56] R. W. Roring, F. G. Hines, N. Charness, "Age differences in identifying words in
  synthetic speech", Human Factors 49(1) (2007) 25–31, doi:10.1518/001872007779598055,
  PMID 17315840. Abstract only, from the Europe PMC API.
- [S57] M. Wolters, P. Campbell, C. DePlacido, A. Liddell, D. Owens, "Making Speech Synthesis
  More Accessible to Older People", 6th ISCA Workshop on Speech Synthesis (SSW6), 2007.
  https://www.isca-archive.org/ssw_2007/wolters07_ssw.pdf
- [S58] S. Mehri, A. W. Black, M. Eskenazi, "CMU GetGoing: An Understandable and Memorable
  Dialog System for Seniors", arXiv:1909.01322. https://arxiv.org/pdf/1909.01322
- [S59] D. Han, W. Chen, J. Kang, M. Cui, H. Meng, X. Wu, "Imitation Learning for Elder-Facing
  Speech Synthesis", arXiv:2606.21053. https://arxiv.org/pdf/2606.21053

(S23 and S51 are unused numbers.)

## Unfetched sources

- J. James, B. T. Balamurali, C. I. Watson, B. MacDonald, "Empathetic Speech Synthesis and
  Testing for Healthcare Robots", International Journal of Social Robotics 13 (2021)
  2119–2137, doi:10.1007/s12369-020-00691-4. The Springer page redirected to a login. Its
  findings are cited only as summarised in [S50].
- The full text of [S49]: the PMC page served a CAPTCHA. The sizes of the rate and pitch
  changes are therefore unknown.
- The full texts of [S55] and [S56]: abstracts only. The 150 against 210 words-per-minute
  detail of [S56] is as reported by [S57].
- The 35 pages of [S48] outside the fetched excerpt, including Table 7's per-study counts.
- TTS Arena V2 (Hugging Face Space `TTS-AGI/TTS-Arena-V2`): not fetched. Only [S37] is used for
  arena scores.
- A published speed for Kokoro on a desktop CPU, for Chatterbox-Turbo on any hardware, and for
  any candidate on an 8 GB laptop GPU: none found.
- Supertonic's CPU and GPU runtimes exist only as an image on its model card; not read.
- `onnx-community/chatterbox-ONNX` (an export of the original Chatterbox): seen in a Hugging
  Face search, not examined.

## Appendix: measurements

### M1. What espeak-ng gives piper-rs

A scratch crate depending on `espeak-rs = "=0.2.0"`, built offline from the local registry
(the same version piper-rs 0.2.0 uses), calling it exactly as `piper-rs/src/lib.rs:86` does:

```rust
let p = espeak_rs::text_to_phonemes(&text, "en-us", None).unwrap().join(" ");
```

Run with `PIPER_ESPEAKNG_DATA_DIRECTORY=models` (as `Makefile:49` sets it). Output, text then
phonemes:

```
"Oh no"                             ˈoʊ nˈoʊ
"Oh no, I'm sorry to hear that."    ˈoʊ nˈoʊaɪm sˈɑːɹi tə hˈɪɹ ðˈæt
"Oh, no."                           ˈoʊnˈoʊ
"Oh nooo"                           ˈoʊ nˈuːoʊ
"Oh noo, I'm sorry to hear that."   ˈoʊ nˈuːaɪm sˈɑːɹi tə hˈɪɹ ðˈæt
"I'm so sorry."                     aɪm sˌoʊ sˈɑːɹi
"test"                              tˈɛst
"Oh no, I'm sorry to hear that. Are you hurt?"
                                    ˈoʊ nˈoʊaɪm sˈɑːɹi tə hˈɪɹ ðˈætɑːɹ juː hˈɜːt
```

Phoneme map entries, from `en_US-hfc_female-medium.onnx.json` (159 entries):
`ː`→122, `ˈ`→120, `ˌ`→121, `o`→27, `ʊ`→100, `n`→26, `,`→8, `.`→10, `?`→13, ` `→3. Voice
defaults: `noise_scale` 0.667, `length_scale` 1, `noise_w` 0.8, 22 050 Hz, one speaker.

### M2. The voice's graph

`onnx` 1.23.0 in a scratch virtualenv, loading `models/en_US-hfc_female-medium.onnx` (63 MB)
without running it:

```
opset [('', 15)]   producer pytorch 2.0.0   nodes 2755
inputs [('input', ['batch_size', 'phonemes']), ('input_lengths', ['batch_size']), ('scales', [3])]
outputs ['output']
Ceil nodes [('/Ceil', ['/Mul_1_output_0'], ['/Ceil_output_0'])]
Ceil /Ceil
  Mul /Mul_1
    Mul /Mul
      Exp /Exp
        Split /dp/Split ...
      </enc_p/Cast_1_output_0>          (x_mask)
    Gather /Gather_1
      <scales>
scales consumers: /Gather (index 0), /Gather_1 (index 1), /Gather_2 (index 2)
```

### M3. The per-phoneme `length_scales` edit, checked but not run

In memory, never saved:
- Add input `length_scales` (float, `[1, 1, "phonemes"]`).
- Set `/Mul_1`'s second input to it.
- Append `/Ceil_output_0` as an output.

Then run `onnx.checker.check_model` and `onnx.shape_inference.infer_shapes`:

```
before: /Mul_1 inputs ['/Mul_output_0', '/Gather_1_output_0']
after:  /Mul_1 inputs ['/Mul_output_0', 'length_scales']
checker: ok
inputs: [('input', ['batch_size', 'phonemes']), ('input_lengths', ['batch_size']), ('scales', [3]), ('length_scales', [1, 1, 'phonemes'])]
outputs: ['output', '/Ceil_output_0']
other users of scales[1]: []
```

No inference was run on the edited graph. Whether it runs, and how it sounds, is untested.
