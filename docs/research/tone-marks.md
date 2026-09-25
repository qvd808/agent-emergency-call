# Tone marks the voice model renders itself

Research for the maintainer's question of 2026-09-25, after listening to the "Oh no" samples
from [expressive-voice-measurements.md](expressive-voice-measurements.md):

> We need a way to generate tone marking for the model when it reads the transcript, so we can
> guarantee a fast reply with tonal voice and emotion in play. The emotion has to match what
> it's saying. "Oh noo" in a joking situation and feeling sorry have completely different tones.

A second condition came from the same listening, on 2026-09-25: editing the audio after the
model has made it (stretching a vowel, reshaping the pitch) makes the voice worse. A mark has
to be an input the model was trained on, so that the model itself decides the timing, pitch
and loudness that go with it.

Each claim is marked by where it comes from:
- **Fetched**: stated in a source fetched on 2026-09-25, listed at the end as [T1]–[T12].
- **Measured**: produced by a command in this repository, named where it is used.
- **Heard**: the maintainer's verdict on audio in this session.
- **Inferred**: worked out here, not stated by any source.

## Answer (short)

1. **Editing the audio after synthesis is ruled out.** The maintainer (heard, 2026-09-25)
   rated six renderings of "Oh no, I'm sorry to hear that." from one Piper take:
   - Piper's own output was best.
   - The five pitch edits of "Oh no," (surprised, three attempts at apologetic, joking) were
     all worse.
   - Holding the vowel lengthens the low, flat pitch Piper gives "no". That reads as the speaker
     being upset herself.
   - A high peak on "no" (sample 08) reads as surprise.
   - What was wanted is apologetic: sorry for the caller.

   The numbers are in "What was tried and rejected" below. Stretching the "no" is dropped.

2. **Piper takes no emotion input.** The only inputs it has are:
   - the text, with the six punctuation marks its phonemizer keeps (`agent/src/tts.rs:122-124`);
   - a speaker id, for voices trained with several speakers (piper-rs `src/model.rs:89-90`
     [T12]);
   - three sampling scales, set per sentence (`agent/src/tts.rs:33-40`).

   The scales barely move the pitch: Warm and Steady were never more than 0.34 semitones apart
   in pitch variation (`agent/src/tts.rs:37-40`, measured for issue #37). With the current
   single-speaker voice, a mark can only choose punctuation, wording, speed and variation, not
   an emotion (**inferred** from the three points above).

3. **The label matters as much as the model: "sad" is not "sympathetic".**
   - Microsoft's commercial voices list `sympathetic`, `empathetic`, `concerned`, `reassuring`,
     `joking`, `serious` and `urgent` as styles separate from `sad` [T1].
   - The Expresso dataset has 100 minutes of speech recorded in a `sympathetic` style, apart
     from `sad` (101 min) and `calm` (93 min) [T2].

   The open models that fit this laptop offer only basic emotions:

   | Model | Emotion input | Source |
   |---|---|---|
   | NeuTTS-2E | 7 labels: angry, disgusted, fearful, happy, neutral, sad, surprised | expressive-voice-measurements.md, R4 |
   | IndexTTS2 | 8 intensities: happy, angry, sad, afraid, disgusted, melancholic, surprised, calm | [T4] |
   | CosyVoice | fixed instructions: very happy, very sad, very angry, very soft, loud, slow, fast | [T5] |
   | Chatterbox-Turbo | sound tags such as `[sigh]` and `[laugh]` | [T6] |
   | Parler Mini Expresso | text description with Expresso's read styles: happy, confused, laughing, sad, whisper, emphasis | expressive-voice.md, S22 |

   None of these lists has an apologetic or sympathetic entry. Mapping "sympathetic" onto any
   of them lands on `sad`, `calm` or `soft`, the direction the maintainer heard as wrong
   (**inferred**). Only models that take a free-text instruction can be asked for "sympathetic"
   in words. Of those, Qwen3-TTS 1.7B needs about 4.5 GB and does not fit next to the LLM
   (expressive-voice.md, section 2).

4. **One way to get a native "sympathetic" at Piper speed: a Piper voice whose speakers are
   styles.** This is one of the two routes Piper's maintainer suggested for emotion
   (expressive-voice.md, S8).
   - **Training.** Piper's training code takes a `file|speaker|text` CSV and maps each speaker
     name to an id stored in the voice config [T3]. Training from an existing `medium` checkpoint
     "will speed up training a lot" [T3]. Users report training with as little as 8 GB of GPU
     memory [T3].
   - **Data.** Expresso has four speakers, each in several styles, including `sympathetic`,
     `happy`, `calm`, `sad`, `laughing`, `whisper` and `default` [T2].
   - **At run time.** The tone mark becomes a speaker id. piper-rs 0.2.0 already passes a
     speaker id to multi-speaker voices [T12]. `tts.rs` passes `None` today
     (`agent/src/tts.rs:90`).
   - **Cost.** A medium voice with more speakers has the same architecture, so the cost per
     reply should stay near Piper's 67–176 ms (**inferred**).
   - **What the maintainer would have to accept:**
     - The voice becomes an Expresso speaker, not `hfc_female` (**inferred**: a speaker id
       carries timbre and style together).
     - Expresso is recordings of real people under CC BY-NC 4.0 [T2]. `hfc_female`'s dataset is
       already CC BY-NC-SA 4.0 (expressive-voice-measurements.md, R1).
     - The improvised styles, `sympathetic` among them, come without transcripts. Only the read
       speech is transcribed [T2]. So they would need transcribing offline, for example with
       whisper (**inferred**).
     - Training time on the laptop's 8 GB GPU is unknown.

   Piper's training code also takes custom phonemes [T3], which is the second route: a style
   symbol inside the phoneme string. That could mark a single phrase rather than a whole
   sentence, but it needs the same labelled data (**inferred**).

5. **The mark should be written by the reply LLM, in the same answer as the reply.**
   - Only the LLM has read the call, so only it can tell burnt toast from a fall (**inferred**).
   - The turn is one structured answer, and the agent starts speaking once the fields before
     `summary` are complete (`agent/src/llm.rs:30-33`). A `tone` field placed before `reply`
     is therefore ready before the words are spoken (**inferred**).
   - A one-word enum costs a few output tokens. At the measured 84 tokens/s
     (`agent/src/llm.rs:33`), 5 tokens are about 60 ms (**inferred** arithmetic).
   - Microsoft's styles use the same shape: a plain-text marker such as `[whispering]` before a
     sentence, applying until reset [T1]. Its docs add that the model "adapts style application
     based on the semantic meaning of the text" [T1].
   - Tag accuracy is untested. Whether `qwen3:4b` picks the right tone is the open fact.
     `tone-marks/tag_eval.py` measures it on twelve invented resident lines, four of them jokes.
     It ran only against a mock server here, because no model registry is reachable from this
     container.

## How the pieces fit (**inferred**)

- **The mark set is small and fixed:** `sympathetic`, `glad`, `playful`, `reassuring`,
  `serious`, `neutral` (the set in `tag_eval.py`). Each is a stance toward the resident, not the
  agent's own feeling. That is the difference between "sorry for you" and "sad".
- **Each mark needs a native rendering in the voice.** Until a voice can render all six, the
  marks fall back to what Piper has natively: punctuation, and today's two `Tone`s, which are
  chosen now from `status` (`agent/src/conversation.rs:823-829`). No audio is edited after
  synthesis.
- **Order:**
  1. The mark in the turn schema, and `tag_eval.py` on the laptop. This settles whether the
     LLM half works before any voice is trained.
  2. A style-as-speaker Piper voice from Expresso, if the maintainer accepts its voice and
     licence.
  3. A free-text-instruction model, once the hardware allows one.

## What was tried and rejected

`tone-marks/render_oh_no.py`, output in `tone-marks/output/render_oh_no.txt` (measured on
2026-09-25). This was one take of `hfc_female` (sherpa-onnx copy) at Steady's length and noise,
with `noise_w` 0 and 26 frames added to the vowel of "no" inside the graph. The pitch of "Oh no,"
was then replaced with Praat's overlap-add, and three variants were made 3 dB softer.

| File | "Oh" median/max/end, Hz | "no" median/max/end, Hz | Verdict (heard) |
|---|---|---|---|
| 0-as-piper | 216/255/198 | 171/178/170 | best |
| 1-surprised | 258/264/262 | 263/308/206 | worse |
| 2-sorry-glide | 213/217/212 | 210/237/184 | worse |
| 3-sorry-rise-fall | 202/205/201 | 216/237/191 | worse |
| 4-sorry-fall-lift | 213/217/212 | 200/231/202 | worse |
| 5-joking | 280/284/283 | 285/337/278 | worse |

The same low, flat "no" (160–167 Hz, 10.57–10.75 s) is in the maintainer's `2-after-fix.wav`,
made by the agent's own pipeline (measured with Praat in this session). The audio was not kept.
The script regenerates it.

## Sources

Fetched on 2026-09-25, raw from GitHub at the commit named.

- [T1] MicrosoftDocs/azure-ai-docs@b9ebc1d217e11b5cc567ef93d618d74c7474483f,
  `articles/ai-services/speech-service/speech-synthesis-markup-voice.md`:
  - lines 128–131: style markers in plain text, "A style marker applies to all subsequent
    sentences until you reset the style";
  - lines 141 and 153: the HD voices' styles, including `sympathetic`, `concerned`, `joking`,
    `reassuring`, `sad`, `serious` and `urgent`;
  - line 145: "the model adapts style application based on the semantic meaning of the text";
  - line 171: `styledegree`, 0.01–2;
  - lines 190, 195 and 204: `empathetic`, `gentle` and `sad` described separately.
  Paid; listed only as evidence of what a style vocabulary distinguishes.
- [T2] facebookresearch/textlesslib@ba33d669d8284b4f7bfe81e7384e83ab799fe384,
  `examples/expresso/dataset/README.md`:
  - line 5: 4 speakers, 8 read styles, 26 improvised styles, transcriptions for read speech
    only;
  - lines 13–41: minutes per style (`sympathetic` 100, `sad` 101, `calm` 93);
  - lines 132–133: CC BY-NC 4.0.
- [T3] OHF-Voice/piper1-gpl@5b355b110aecf3de8f4e000ede1ce06831acff35, `docs/TRAINING.md`:
  - lines 49–73: training command, fine-tuning from a checkpoint;
  - lines 81–91: multiple speakers;
  - lines 93–107: custom phonemes;
  - lines 156–158: hardware.
- [T4] index-tts/index-tts@ee40fa7d6c6b8a2c7f06105f9f1e65775b74868c, `README.md` lines
  281–306: emotion vector order, `use_emo_text`.
- [T5] FunAudioLLM/CosyVoice@074ca6dc9e80a2f424f1f74b48bdd7d3fea531cc,
  `cosyvoice/utils/common.py` lines 28–53 (`instruct_list`), `example.py` line 30.
- [T6] resemble-ai/chatterbox@5de7a54aa4e5e2baadb0182dde554908b48b85c2, `README.md` lines
  30–32 and 44–45.
- [T7] canopyai/Orpheus-TTS@e64661fe6d02c414fc77c53578c9d64082614861, `README.md` line 113:
  `<laugh>`, `<chuckle>`, `<sigh>`, `<cough>`, `<sniffle>`, `<groan>`, `<yawn>`, `<gasp>`.
- [T8] hi-paris/Prosody-Control-French-TTS@43c394998eccec80e34ca7534f0292df38801dfd:
  - `README.md` describes LLMs annotating text with SSML pitch, volume, rate and pauses;
  - `Code/ssml_models/fewshot/model.py` lines 165–173 and `config.yaml` run it through Ollama
    with models including `qwen3:8b` and `qwen2.5:7b`.
  Its results are in a paper that could not be fetched.
- [T9] walker-hyf/ECSS@c1545fdb12ca56482de4ff3550dc47b81d9fe794, `README.md`: conversational
  TTS whose training data labels each sentence with an emotion and an intensity
  (`sentence ID|speaker|phonemes|text|emotion|emotion intensity`).
- [T10] Helsinki-NLP/prosody@75a56c5790ca487e2bb52380bd98722ff0c147b7, `README.md` lines 16–18
  and 62–65: prominence (0/1/2) and boundary labels predicted from text alone; BERT best.
- [T11] AI-S2-Lab/Chain-Talker@197b096f2d547bbc99a8c42a6d51df2be5940299, `README.md`: title
  only ("Chain Understanding and Rendering for Empathetic Conversational Speech Synthesis",
  ACL 2025 Findings).
- [T12] thewh1teagle/piper-rs@d70b0970a87453f3476b5fd6cf9edf329be8b445:
  - `src/lib.rs:74-82`: `create` takes `speaker_id`;
  - `src/lib.rs:105-110`: speaker name to id map;
  - `src/model.rs:89-90`: the id is passed when `num_speakers > 1`.

### Unfetched

Web search found these, but arXiv, ACL Anthology, ISCA, W3C and Hugging Face are all blocked
from this container. Nothing above rests on them.

- Chain-Talker (arXiv 2505.12597): emotion descriptors derived from the dialogue history.
- ECSS (arXiv 2312.11947).
- The French SSML paper (ICNLSP 2025, aclanthology `2025.icnlsp-1.30`; arXiv 2508.17494).
- EmoVoice (arXiv 2504.12867).
- Kyutai's voice repository, for which Expresso clips it holds (expressive-voice.md cites it
  as S24).
