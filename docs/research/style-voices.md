# Style voices: what issue #41 costs, and a way to the same voice without Expresso

Research for the maintainer's request of 2026-09-25 on
[Would a Piper voice trained on Expresso, one speaker id per style, be an acceptable agent voice?](https://github.com/qvd808/agent-emergency-call/issues/41):

> Research ticket 41 whether it is costly. I don't wanna waste money. I would rather we come up
> with some method or modify the output of Piper [...] Still I do think we need some model
> that can output some tonal mark first so the other model can then infer it and have emotion
> when speaking. [...] Just use some free API on Hugging Face first, then we generate some
> sample voice with emotion for me to hear and pick if it's good.

It continues [tone-marks.md](tone-marks.md). There, editing Piper's audio after synthesis was
rejected by ear, and the tone mark was put in the reply LLM's answer. So "modify Piper" here
means changing the model, meaning which speaker ids it has, not the audio it outputs.

Each claim is marked by where it comes from:
- **Fetched**: stated in a source fetched on 2026-09-25, listed at the end as [V1]–[V12].
- **Measured**: produced by `style-voices/render_style_voices.py`; output in
  `style-voices/output/render_style_voices.txt`.
- **Inferred**: worked out here, not stated by any source.

Nothing here has been heard yet. The samples are in `style-voices/samples/` for the maintainer
to judge.

## Blocked in this session

Hugging Face (`huggingface.co`, `router.huggingface.co`, `hf.space`), arXiv, Kaggle, Colab and
`ollama.com` all returned 403 from this container's egress proxy on 2026-09-25. That rules out
the free Hugging Face API step. No expressive model could be run here, so every sample below
comes from Piper voices on GitHub releases. The teacher samples (point 4) need Hugging Face
opened in the environment's network settings, or the laptop.

## Answer (short)

1. **In money, the route in issue #41 costs nothing** (**inferred**). Every step runs on the
   laptop with open-source tools. What it costs instead is disk and laptop time:
   - **Download.** Expresso is one 36 GB tar file [V3, line 51].
   - **Data preparation.** `sympathetic` exists only as improvised dialogue: 100 minutes, no
     read speech [V3, line 39]. Dialogues are stereo, one channel per actor [V3, line 46], and
     only read speech has transcriptions [V3, line 5]. So one actor's channel has to be cut into
     utterances and transcribed, for example with whisper (**inferred**).
   - **Training.** Fine-tuning from a `medium` checkpoint "will speed up training a lot" [V1,
     lines 49–73]. A multi-speaker voice fine-tuned from a single-speaker checkpoint is "*much*
     faster" than training from scratch [V2, line 188]. Piper's maintainer found about 1000
     extra epochs enough when fine-tuning [V2, line 218]. Users report 8 GB of VRAM working
     [V1, lines 156–158].
   - **Unknown:** hours per epoch on the laptop's GPU. No fetched source gives a time. One epoch
     on the real data, timed, would settle it (**inferred**).
   - Paid cloud GPUs are not needed. A free, offline kit for fine-tuning Piper voices exists
     (TextyMcSpeechy: an NVIDIA GPU and about 50 GB of disk [V12]). Free notebook tiers (Kaggle,
     Colab) could not be checked from here.

2. **It costs nothing at run time.** Synthesis time is the same with or without style speakers
   (**measured**, one run of each clip, 4 CPU cores in this container, not the laptop):

   | Voice | Speaker ids | Synthesis time per second of audio (RTF) |
   |---|---|---|
   | `en_US-hfc_female-medium` (today's voice) | 1 | 0.04–0.06 |
   | `en_GB-semaine-medium` | 4 personas | 0.04–0.05 |
   | `de_DE-thorsten_emotional-medium` | 8 emotions | 0.04–0.05 |

   A speaker id is one row of an embedding table the voice already has, which is why the speed
   doesn't change (**inferred**). piper-rs already passes the id, and `agent/src/tts.rs:90` passes
   `None` today (tone-marks.md, point 4).

3. **The approach already works in two published Piper voices [V7].** Both can be heard now:
   - `de_DE-thorsten_emotional-medium` is the exact shape issue #41 proposes. One man, with 8
     speaker ids that are emotions (`amused`, `angry`, `disgusted`, `drunk`, `neutral`,
     `sleepy`, `surprised`, `whisper`). It was fine-tuned from the neutral thorsten voice [V5].
     Its data was 300 sentences per emotion, 2400 recordings, all by the same speaker, acted
     "even if the phrase context does not match that emotion" [V4, lines 116–121]. It is German,
     so it shows the mechanism, not a voice for this agent.
   - `en_GB-semaine-medium` has 4 speaker ids, each a different actor playing one character:
     Poppy "outgoing and optimistic", Obadiah "gloomy and depressed", Prudence "pragmatic and
     practical", Spike "angry and argumentative" [V6, lines 11–13]. Style and timbre change
     together here, which is what issue #41 warns the Expresso voice would do.

   So about 300 sentences per style were enough for a published Piper voice to learn a style
   (**inferred** from thorsten_emotional). That is about 25 minutes of speech per style, if the average sentence
   runs 5 s (**inferred**; thorsten's sentences are 59–148 characters [V4, line 126]).

4. **The same voice can be built without Expresso by distilling a larger expressive model into
   Piper.** A large text-to-speech model (the teacher) speaks about 300 lines in each of the six
   tone marks, offline. Piper is then fine-tuned on that audio, one speaker id per mark, like
   thorsten_emotional (**inferred**; this is how the pieces combine, not a recipe found in a
   source). Compared with issue #41:

   | | Expresso (issue #41) | Distilled from a teacher |
   |---|---|---|
   | Money | none | none on the laptop; the listening step can use a free Hugging Face Space |
   | Styles | Expresso's 26, with `sympathetic` and `happy` but no `playful` or `reassuring` [V3, lines 13–41] | exactly the six marks in `tone-marks/tag_eval.py` |
   | Text | transcribe improvised dialogue | written by us, so no transcribing |
   | Data | recordings of 4 real actors, CC BY-NC 4.0 [V3, line 133] | synthetic, which fits the synthetic-data-only safety rule |
   | Voice | becomes an Expresso actor | a teacher that clones a voice could stay close to hfc_female |
   | Main risk | the dialogue audio: crosstalk, laughter, overlapping turns (**inferred**) | Piper learns the teacher's flaws, and the teacher's "sympathetic" may be wrong |

   Two teachers fit, each with a licence limit:
   - **IndexTTS2.** It takes the speaker from one reference clip and the emotion separately:
     from a second reference clip, an 8-value emotion vector, or the text itself [V9, README
     lines 242–318]. So hfc_female's timbre with a sympathetic delivery is at least expressible.
     Its licence treats model outputs as derivative works [V9, LICENSE line 10], and it allows
     them to improve other models only if those are non-commercial [V9, LICENSE line 28].
   - **Qwen3-TTS 1.7B CustomVoice.** It takes a free-text style instruction ("Very happy.") over
     9 fixed timbres, of which Ryan and Aiden are English-native [V8, README lines 73–79 and
     150–200]. The voice-cloning models take no instruction [V8, lines 77 and 79], so this teacher can't
     keep hfc_female. The repository is Apache-2.0 [V8, LICENSE]. The weights' own licence is
     on Hugging Face and was not fetched.

   Earlier notes ruled these models out for live use, because they don't fit next to the LLM
   (tone-marks.md, point 3). Distilling moves them offline, where the laptop's GPU runs one
   model at a time (**inferred**). Piper stays the only voice model on a call. The step that
   must come first is listening to the teacher's six tones. If the teacher's "sympathetic"
   sounds wrong, distilling it is wasted. That step is blocked here (see above).

5. **The tone mark needs no second model yet.** The LLM already running in Ollama
   (`qwen3:4b-instruct-2507-q4_K_M`, `.env.example:7`) writes the mark in the same answer as
   the reply. At the measured 84 tokens/s (`agent/src/llm.rs:33`), a 5-token mark is about 60 ms
   (tone-marks.md, point 5). A separate tagger would be a second model, loaded and run every
   turn (**inferred**). It is worth building only if `tone-marks/tag_eval.py`, run on the
   laptop, shows the LLM picks wrong tones. Then the "distilled version" is a small classifier:
   a large model labels many (resident line, reply) pairs, and a small one learns the labels
   (**inferred**). An off-the-shelf emotion classifier is the wrong shape. GoEmotions, for
   example, labels the emotion a comment expresses, among 27 categories including `caring`,
   `remorse` and `amusement` [V10, lines 3–19]. The mark is the agent's stance toward the
   resident, which is a different thing (**inferred**).

## The samples

`style-voices/samples/`, 46 clips at 8 kHz, what a softphone hears. The file name is
`<voice>-<tone the line was written for>-<speaker id>.wav`. There is one line per tone mark:

| Tone | Line |
|---|---|
| sympathetic | Oh no, I'm sorry to hear that. Are you hurt? |
| glad | That's wonderful news, I'm so happy for you! |
| playful | Ha, a smoke alarm for a morning alarm. That's one way to wake up! |
| reassuring | Don't worry, you're not alone. Someone will check on you tonight. |
| serious | Please stay where you are. I'm getting a person on the line now. |
| neutral | Thanks. Did you have breakfast today? |

- **hfc_female and semaine** speak all six lines. semaine speaks each one in all four personas.
- **thorsten_emotional** speaks two German lines, the sympathetic one ("Oh nein, das tut mir
  leid. Sind Sie verletzt?") and the playful one, in all eight emotions. It is there to show one
  voice changing emotion by speaker id.

What the numbers show (**measured**):
- The same line changes pitch with the speaker id. Across semaine, the sympathetic line's median
  ranged from 97 Hz (Spike) to 320 Hz (Poppy).
- Within thorsten_emotional, the sympathetic line in `neutral` sat at 136 Hz, and in `angry` at
  218 Hz.
- Both whisper clips are 4–7% voiced, so their pitch figures are meaningless.
- Piper draws random phoneme lengths at its default `noise_w` 0.8, so lengths and pitch shift a
  little from run to run.

No voice renders an apologetic "sympathetic". semaine and thorsten_emotional have no such
speaker (**fetched**, [V5] [V6]). Their clips show the mechanism and the voice quality a Piper
style voice reaches. They do not show the target tone.

## What was tried and not reachable

- The Hugging Face Spaces and Inference API for Qwen3-TTS, IndexTTS2 and Parler Expresso:
  proxy 403 (see above).
- OpenVoice V1, whose base speaker had style control, is MIT-licensed [V11, README line 36].
  Its checkpoint link [V11, USAGE line 51] returned `NoSuchBucket` on 2026-09-25.
- The Expresso tar at `dl.fbaipublicfiles.com`: proxy 403, so no Expresso clip could be heard.

## Sources

Fetched on 2026-09-25, raw from GitHub at the commit named, unless stated otherwise.

- [V1] OHF-Voice/piper1-gpl@5b355b110aecf3de8f4e000ede1ce06831acff35, `docs/TRAINING.md`:
  lines 49–73, fine-tuning with `--ckpt_path` "will speed up training a lot", medium only;
  lines 81–91, multiple speakers; lines 156–158, hardware, "as little as 8GB of VRAM".
- [V2] rhasspy/piper@73c04d81d5590ecc46e522de3601ce7fb29fc2be, `TRAINING.md`: line 183, batch 32
  with `--max-phoneme-ids 400` for 24 GB; lines 186–188,
  `--resume_from_single_speaker_checkpoint`; line 218, "2000 epochs [...] from scratch, and an
  additional 1000 epochs when fine-tuning".
- [V3] facebookresearch/textlesslib@ba33d669d8284b4f7bfe81e7384e83ab799fe384,
  `examples/expresso/dataset/README.md`: line 5, 4 speakers, 40 h, transcriptions for read speech
  only; lines 13–41, minutes per style; line 46, stereo dialogues; line 51, `expresso.tar`
  (36GB); line 133, CC BY-NC 4.0.
- [V4] thorstenMueller/Thorsten-Voice@29d0c153c55d8ffaa0ebf8ed1ecc879bff9fdf27, `README.md`
  lines 116–126: the emotional dataset, 300 sentences × 8 emotions, 59–148 characters.
- [V5] k2-fsa/sherpa-onnx GitHub release `tts-models`, assets
  `vits-piper-de_DE-thorsten_emotional-medium.tar.bz2`,
  `vits-piper-en_GB-semaine-medium.tar.bz2` and `vits-piper-en_US-hfc_female-medium.tar.bz2`,
  downloaded 2026-09-25: each `MODEL_CARD` (dataset, licence, "Finetuned from") and each
  `.onnx.json` `speaker_id_map`.
- [V6] marytts/dfki-semaine-data@cbeb97b9bb0deecf4355220fcfba280a7b30983a, `README.md`
  lines 7–14: the four characters and their recording durations.
- [V7] rhasspy/piper@73c04d81d5590ecc46e522de3601ce7fb29fc2be, `VOICES.md` lines 42–43 and
  62–63: both voices are in Piper's official list.
- [V8] QwenLM/Qwen3-TTS@022e286b98fbec7e1e916cb940cdf532cd9f488e, `README.md` lines 73–79 (models
  and which take instructions) and 150–200 (`generate_custom_voice`, `instruct`, speakers);
  `LICENSE`, Apache 2.0.
- [V9] index-tts/index-tts@ee40fa7d6c6b8a2c7f06105f9f1e65775b74868c: `README.md` lines 242–318
  (`spk_audio_prompt`, `emo_audio_prompt`, `emo_alpha`, `emo_vector`, `use_emo_text`);
  `LICENSE` line 10 (outputs are derivative works) and line 28 (improving other models only if
  non-commercial).
- [V10] google-research/google-research@d36068b845da4c2b24927fee2cea1e6ef98dadda,
  `goemotions/README.md` lines 3–19.
- [V11] myshell-ai/OpenVoice@74a1d147b17a8c3092dd5430504bd83ef6c7eb23, `README.md` line 36 and
  `docs/USAGE.md` line 51.
- [V12] domesticatedviking/TextyMcSpeechy@413a6edc7ca11cc1aa05eea2f7c5e7db3b834c6a, `README.md`
  lines 13 and 77–78: a free, offline Piper fine-tuning kit; an NVIDIA GPU required, 50 GB of
  free disk suggested.

### Unfetched

Blocked from this container (see above). Nothing above rests on them.

- The Expresso paper (arXiv 2308.05725) and its demo page.
- `rhasspy/piper-checkpoints` on Hugging Face: whether hfc_female's training checkpoint is
  published. If it is not, fine-tuning would start from lessac, which hfc_female was itself
  fine-tuned from [V5].
- The weight licences and sizes of Qwen3-TTS and IndexTTS2 on Hugging Face.
- Free GPU quotas on Kaggle and Colab.
