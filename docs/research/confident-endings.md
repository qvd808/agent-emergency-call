# What makes Poppy and Prudence sound confident, and what hfc_female can borrow

The maintainer listened to the samples from [style-voices.md](style-voices.md) on 2026-09-25
(**heard**):

- hfc_female stays the agent's voice. It sounds more comfortable and reassuring.
- semaine's Poppy and Prudence are the interesting ones, for specific scenarios. They have "some
  kind of tonal at the end of the word", which sounds confident: the word is clear and there is a
  stop after it.
- All the semaine voices sound like someone making an announcement.

The question is how those two voices were made, what controls them, and what makes them sound
different.

Each claim is marked by where it comes from:
- **Fetched**: stated in a source fetched on 2026-09-25, listed at the end as [C1]–[C5].
- **Measured**: produced by a script in `confident-endings/`, output in
  `confident-endings/output/`. Runs are on this cloud container's 4 CPU cores, not the laptop.
- **Heard**: the maintainer's verdict.
- **Inferred**: worked out here, not stated by any source.

## Answer (short)

1. **No setting makes a voice confident.** At synthesis time every speaker has the same three
   dials: `noise_scale`, `length_scale` and `noise_scale_w` [C1, models.py:779–781]. The speaker
   id only picks one row of a learned table: 512 numbers per speaker [C1, models.py:706 and 787;
   config.py:110]. That vector goes to three places:
   - the **duration predictor**, which sets how many frames each sound lasts, pauses included
     [C1, models.py:792];
   - the **flow** and the **decoder**, which turn phonemes into sound frames and then into the
     waveform [C1, models.py:812–813]. Pitch, loudness and timbre come out of these, since VITS
     has no separate pitch input (**inferred** from the code: no pitch appears in `infer`).

   The text encoder never sees the vector [C1, models.py:784]. So every speaker reads the same
   phonemes, and differs only in timing and in how they sound.

2. **The confidence came from the actors, not from any tuning.**
   - **The base.** semaine is lessac's medium voice, fine-tuned on DFKI's recordings of four
     actors [C3]. Each actor played one character: Poppy "outgoing and optimistic", Prudence
     "pragmatic and practical" [C2, README lines 11–13]. Training learned each actor's vector
     from their recordings.
   - **What they read (measured, from the prompt codes in [C2]):** most of it is
     encyclopaedia-style sentences.
     - Poppy: 737 of 992 utterances (107 min).
     - Prudence: 2109 of 2343 (226 min).

     Each also has about 150 in-character lines ("I'm the eternal optimist.") and about 80 topic
     questions.
   - **Accent.** The voice is phonemised as British Received Pronunciation (`en-gb-x-rp` [C3]).
     hfc_female is `en-us`.

   Reading aloud, in character, is the likely source of the "announcement" sound (**inferred**).
   The accent is part of what separates the two voices by ear (**inferred**).

3. **Piper copies each actor closely** (**measured**, `measure_endings.py`). The text was 48
   sentences both actors recorded with identical wording:

   | Source | Speech (s) | Pauses | Pause time (ms) | Median pitch (Hz) | Pitch spread (st) | Brightness (dB) |
   |---|---|---|---|---|---|---|
   | Poppy, recording | 2.58 | 5.0 | 640 | 250 | 13.1 | −6.6 |
   | Poppy, Piper | 2.59 | 5.0 | 618 | 255 | 12.3 | −6.3 |
   | Prudence, recording | 2.91 | 3.0 | 352 | 215 | 17.4 | −10.3 |
   | Prudence, Piper | 2.93 | 3.0 | 392 | 212 | 15.0 | −10.9 |
   | hfc_female, Piper | 2.81 | 2.5 | 262 | 208 | 10.2 | −11.5 |

   How to read the columns:
   - All values are medians over the 48 sentences. Each Piper sentence is the mean of 2 random
     draws.
   - "Pauses" counts silences of 60 ms or more inside a sentence.
   - "Pitch spread" is the 10th–90th percentile range, in semitones.
   - "Brightness" is the energy at 1–5 kHz relative to 50 Hz–1 kHz, in voiced frames. The
     recordings and Piper differ in microphone and room, so compare Piper with Piper on this
     column.

4. **What differs from hfc_female on the same sentence** (**measured**; the percentages are the
   share of sentences where it holds):
   - **Prudence moves her pitch more:** 15.0 against 10.2 semitones, in all 48 sentences. She also
     pauses longer, in 85% of them. On the six check-in lines (11 sentences, 8 draws each), she
     ends lower and falls faster:
     - 6.8 semitones below her median, against 4.7, in 73% of sentences;
     - the last 200 ms fall at 48 semitones a second, against 14.
   - **Poppy pauses about twice as much:** 5 pauses against 2.5, 618 ms against 262, in 98% of
     sentences. She is also higher (255 against 208 Hz, all 48), brighter (−6.3 against
     −11.5 dB, 96%), and quicker between pauses (81%).
   - **Not different:** the last sound does not stop more abruptly. hfc_female's final sound dies
     away faster: 144 ms from 3 dB to 25 dB below its last peak, against Prudence's 204 and
     Poppy's 154. The pitch just before a pause inside a sentence differs by about half a
     semitone or less between the voices.

   A guess at how these map onto what was heard (**inferred**; only listening can confirm it):
   - "a stop after the word" is the extra pausing, and Prudence's longer word endings (point 5);
   - "a tone at the end" is Prudence's wider pitch movement and lower final fall;
   - "announcement" is Poppy's brightness and both actors' read-aloud delivery.

5. **Where the timing differs** (**measured**, `where_frames_go.py`). The four check-in lines
   were split into 7 sentences, and the same phonemes were timed by each voice at the duration
   predictor's mean (`noise_w` 0). Totals, in ms:

   | Kind of sound | hfc_female | Prudence | Poppy |
   |---|---|---|---|
   | Before the first word | 441 | 476 | 2380 |
   | Vowels | 2926 | 3100 | 3123 |
   | Other consonants | 2009 | 2310 | 2380 |
   | Last sound of a word | 1788 | 2380 | 2125 |
   | Word boundaries and stress marks | 2902 | 2868 | 3065 |
   | Punctuation | 1126 | 1428 | 1521 |

   - **Prudence** holds the last sound of each word 33% longer than hfc_female. Her vowels are only
     6% longer. In "hear that.", the final "t" and the silence before the full stop last 15 frames
     against hfc_female's 8, about 80 ms more.
   - **Poppy** mostly adds about a third of a second of silence before each sentence. On a call,
     that would be added delay.

6. **hfc_female can borrow the timing through its own inputs** (**measured**, `borrow_timing.py`).
   hfc_female and semaine both count time in frames of 256 samples at 22 050 Hz, and both have a
   `Ceil` node where the frame counts are fixed. So:
   1. semaine is asked, as Prudence or Poppy, how long each of hfc_female's phonemes should last;
   2. hfc_female is then made to speak with those counts.

   The pitch, timbre and loudness stay hfc_female's. The rewired graphs are exact: fed their own
   values, both give the original audio, with a maximum difference of 0.0.
   - **Cost (measured, `timing_cost.py`).** semaine cut down to its text encoder and duration
     predictor takes 10–19 ms per sentence. hfc_female's own synthesis takes 39–121 ms per
     sentence.
   - **Caveat.** semaine is timing American phonemes, a sequence it was never trained on
     (**inferred** risk).
   - **What can't be borrowed natively.** Prudence's pitch movement: hfc_female has no pitch
     input, and pitch edits after synthesis were rejected by ear (tone-marks.md, point 1).
     Borrowing pitch would take training (**inferred**).

7. **The voice changes smoothly between speakers** (**measured**: the blends render; none heard).
   A blend of two rows (for example, 25% Obadiah and 75% Poppy) is a valid input and produces
   speech. So the speaker table works like a space with directions, not a list of four
   separate voices (**inferred**). That points toward a strength dial for a style (**inferred**,
   untested).

## The samples

`confident-endings/samples/`, 8 kHz, with sentences joined by the agent's 0.2 s pause
(`agent/src/tts.rs:64`). None have been heard yet.

- `hfc_female-<tone>-<own|prudence|poppy>-timing.wav`: four check-in lines. Each is spoken by
  hfc_female three times, with its own timing, then Prudence's, then Poppy's. All three use the
  duration predictor's mean, so they differ only in timing.
- `semaine-glad-obadiah-to-poppy-<000…100>.wav` and
  `semaine-sympathetic-prudence-to-poppy-<000…100>.wav`: five blends each, from 0% to 100% of
  the second speaker.

## Sources

Fetched on 2026-09-25.

- [C1] OHF-Voice/piper1-gpl@5b355b110aecf3de8f4e000ede1ce06831acff35:
  `src/piper/train/vits/models.py`:
  - line 706, `emb_g`;
  - lines 774–815, `infer`: 779–781 the three scales, 784 the text encoder without the speaker
    vector, 787 the lookup, 792 the duration predictor, 796 `ceil`, 812 the flow, 813 the
    decoder.

  `src/piper/train/vits/config.py` line 110: 512 channels for a multi-speaker voice.
- [C2] marytts/dfki-semaine-data@cbeb97b9bb0deecf4355220fcfba280a7b30983a: `README.md` lines 7–14;
  `poppy/dfki-poppy-data.yaml` and `prudence/dfki-prudence-data.yaml` (prompt codes, texts,
  start and end times); `LICENSE.md`, CC BY-NC-SA 4.0; release `v0.1` assets
  `dfki-poppy-data.flac` and `dfki-prudence-data.flac` (44.1 kHz, 106.8 and 225.8 min). The
  recordings were used for measurement only and are not in this repository.
- [C3] k2-fsa/sherpa-onnx release `tts-models`, `vits-piper-en_GB-semaine-medium.tar.bz2`:
  `MODEL_CARD` ("Finetuned from U.S. English lessac voice") and `en_GB-semaine-medium.onnx.json`
  (`espeak.voice` `en-gb-x-rp`, `speaker_id_map`).
- [C4] The same release, `vits-piper-en_US-hfc_female-medium.tar.bz2`: `.onnx.json`
  (`espeak.voice` `en-us`).
- [C5] The two ONNX graphs themselves: `/emb_g/Gather` over `emb_g.weight` [4, 512] in semaine,
  and a `/Ceil` node in both.

### Unfetched

Wikipedia, the Praat manual, the SEMAINE project site, ToBI guidelines and Hi-Fi-CAPTAIN's page
(the source of hfc_female's recordings) all returned 403 from this container. So:
- there is no fetched source here on how listeners hear pitch falls or pauses;
- hfc_female's own recordings were not measured.
