# Timing and tone: what the picks show, and what is already known

Research for
[How does timing alone carry a reply's emotional tone, and is choosing a voice's timing per tone known work?](https://github.com/qvd808/agent-emergency-call/issues/42).

hfc_female spoke four check-in lines three times each. Its voice, pitch and loudness stayed the
same, and only the timing changed: its own, or timing borrowed from semaine's Prudence or Poppy
through Piper's `Ceil` node ([confident-endings.md](confident-endings.md), point 6). The
maintainer picked one per line (**heard**, 2026-09-25):

| Tone | Line | Picked |
|---|---|---|
| sympathetic | Oh no, I'm sorry to hear that. Are you hurt? | Prudence's timing |
| glad | That's wonderful news, I'm so happy for you! | Poppy's timing |
| reassuring | Don't worry, you're not alone. Someone will check on you tonight. | Poppy's timing |
| serious | Please stay where you are. I'm getting a person on the line now. | hfc_female's own |

Each claim is marked by where it comes from:
- **Fetched**: stated in a source fetched on 2026-09-25, listed at the end as [L1]–[L6].
- **Measured**: produced by `timing-and-tone/timing_by_tone.py`; output in
  `timing-and-tone/output/timing_by_tone.txt`.
- **Heard**: the maintainer's verdict.
- **Inferred**: worked out here, not stated by any source.

Paper hosts (arXiv, ISCA, ACL Anthology, PubMed Central, Frontiers, Springer, ScienceDirect,
Semantic Scholar, ResearchGate, university copies) all returned 403 from this container. So no
paper was read. The published work below is known from its authors' code and READMEs on
GitHub, and papers found only by search are listed as unfetched.

## Answer (short)

1. **Timing alone changed which tone a line carried** (**heard**). The picked timing differs from
   the others in a pattern that fits each tone (**measured**; ms over the whole line):

   | Line | Timing | Sounds per second | Last sound of each word | Punctuation | Before the first word |
   |---|---|---|---|---|---|
   | sympathetic | **Prudence** | **11.1** | **673** | 348 | 93 |
   | | own | 12.5 | 476 | 325 | 128 |
   | | Poppy | 10.9 | 604 | 418 | 929 |
   | glad | **Poppy** | **13.2** | 372 | **325** | 522 |
   | | own | 12.7 | 348 | 221 | 93 |
   | | Prudence | 12.5 | 476 | 163 | 23 |
   | reassuring | **Poppy** | **14.7** | 604 | **430** | 476 |
   | | own | 16.4 | 499 | 290 | 139 |
   | | Prudence | 14.4 | 615 | 383 | 290 |
   | serious | **own** | **16.1** | **464** | **290** | 81 |
   | | Prudence | 14.0 | 615 | 534 | 70 |
   | | Poppy | 13.3 | 546 | 348 | 453 |

   Read as a pattern (**inferred**, from one line per tone and one listener):
   - **sympathetic:** slower, with the end of each word held. Prudence holds word ends 41%
     longer than hfc_female's own timing, without Poppy's long wait before speaking.
   - **glad:** quick, with real stops at the punctuation. Poppy's is the fastest of the three
     and stops longest at the comma and the exclamation mark.
   - **reassuring:** slower, with the longest stops at punctuation.
   - **serious:** brisk and clipped. hfc_female's own timing is the fastest, with the shortest
     word ends and short pauses.

   Poppy's timing also puts 0.45–0.93 s of silence before each line. Whether that helped glad
   and reassuring, or was only tolerated, is not known. On a call it would add to the reply delay
   (**inferred**).

2. **One voice trained on emotions times each one differently** (**measured**).
   `thorsten_emotional` is one speaker whose 8 speaker ids are emotions. Its duration predictor
   was asked to time 8 German sentences under each emotion. Against `neutral`:

   | Emotion | Speech time | Vowels | Consonants | Last sound of a word | Punctuation |
   |---|---|---|---|---|---|
   | amused | −1% | +3% | +0% | −10% | −1% |
   | angry | +17% | +36% | +10% | +5% | +7% |
   | disgusted | +27% | +27% | +30% | +7% | +9% |
   | drunk | +29% | +27% | +33% | +45% | +30% |
   | sleepy | +67% | +85% | +63% | +65% | +65% |
   | surprised | +2% | −5% | +13% | +5% | +8% |
   | whisper | +12% | +14% | +26% | +9% | +151% |

   So the same words take between −1% and +67% more time, and each emotion stretches different
   sounds: angry the vowels, disgusted the consonants, drunk the ends of words, whisper the
   pauses. The actor recorded every emotion on the same 300 sentences, "even if the phrase
   context does not match that emotion" (style-voices.md [V4], lines 116–121). So the timing
   differences come from the emotion, not the words (**inferred**).

3. **That timing carries emotion is established work, not a new finding.**
   - **Emotion features.** openSMILE's GeMAPS set, a standard acoustic set for emotion research,
     includes timing among its features [L4]:
     - the rate and mean length of voiced stretches (lines 40–58);
     - unvoiced stretches, its stand-in for pauses (lines 60–74);
     - loudness peaks per second (lines 83–91).
   - **Emotional synthesisers.** EmoSpeech adds the emotion's vector to the encoded text before
     its duration predictor reads it, so how long each sound lasts depends on the emotion [L2].
     A Korean FastSpeech 2-based model is conditioned on emotion vectors that carry an
     emotion's category and strength [L5].
   - **Control.** FastSpeech 2's reference implementation exposes duration, alongside pitch and
     energy, as a ratio to scale when synthesising [L3].

   The psychology papers that the search results name as the classic sources could not be read:
   Banse and Scherer (1996), and Juslin and Laukka's 2003 meta-analysis. The search engine's
   summaries say they tie slow speech to sadness and fast speech to happiness and anger. That is
   unverified here, and nothing above rests on it.

4. **Moving timing from one voice to another is a published research topic.** Daft-Exprt
   transfers prosody between speakers: pitch, loudness and duration, plus higher-level
   prosody. It trains so that the transferred prosody carries no speaker identity, and speaks
   in the target voice [L1, README line 6]. Its script also offers local prosody control
   [L1, line 220].

   What was done here is a much smaller thing. It borrows only the phoneme lengths, at run time,
   from a second, separately trained Piper voice, with no training at all (**inferred**
   comparison). Whether anyone has published that exact shortcut is unknown. The paper hosts
   are blocked, so novelty can't be checked.

5. **What it suggests for the agent** (**inferred**; a decision for the map, not made here):
   - A tone mark could pick a timing source as well as, or instead of, today's two `Tone`
     settings.
   - That rests on one line per tone and one listener. More lines per tone would show whether
     the picks hold. So would removing Poppy's silence before speaking (using hfc_female's own
     lead-in) and seeing whether glad and reassuring still sound right.
   - The run-time cost is known: 10–19 ms a sentence (confident-endings.md, point 6).

## Sources

Fetched on 2026-09-25 from GitHub, at the commit named.

- [L1] ubisoft/ubisoft-laforge-daft-exprt@a576691c8c42988f813183efcea43c1677abe17a, `README.md`
  line 6 (method and results as the authors state them), line 10 (the released model is trained
  on LJ Speech and ESD, not the paper's data), line 220 (`--control`).
- [L2] deepvk/emospeech@3eee16d3861b2b6ec6129fd9c347c0210fb317b5:
  - `README.md` lines 1–3 and 35–39;
  - `src/models/acoustic_model/fastspeech/fastspeech.py` lines 60–76: speaker and emotion
    vectors added to the encoder output, then the variance adaptor;
  - `src/models/acoustic_model/fastspeech/modules.py` lines 14 and 132: the duration predictor
    reads that output.
- [L3] ming024/FastSpeech2@d4e79eb52e8b01d24703b2dfc0385544092958f3, `README.md` lines 61–66:
  pitch, energy and duration control ratios.
- [L4] audeering/opensmile@055dda577c82383f8239663e5ac6c58d25b504db:
  - `config/gemaps/v01b/GeMAPSv01b_core.func.conf.inc` lines 40–58, 60–74 and 83–91;
  - `config/egemaps/v02/eGeMAPSv02.conf` line 32, where the temporal set is included in
    eGeMAPS.
- [L5] hs-oh-prml/EmotionControllableTextToSpeech@5dcf8afe6a0c1b8d612d6f1d8de315cf419fe594,
  `README.md` line 5.
- [L6] Earlier notes in this repository: [confident-endings.md](confident-endings.md) for the
  borrowed timing and its cost, and [style-voices.md](style-voices.md) [V4] and [V5] for
  thorsten_emotional's data and speaker ids.

### Unfetched

Found by web search on 2026-09-25. Their hosts returned 403, so none was read and nothing above
rests on them.

- Banse, R. and Scherer, K. R. (1996), "Acoustic profiles in vocal emotion expression", Journal
  of Personality and Social Psychology.
- Juslin, P. N. and Laukka, P. (2003), "Communication of emotions in vocal expression and music
  performance: different channels, same code?", Psychological Bulletin 129, 770–814.
- Schröder, M. et al. (2001), "Acoustic correlates of emotion dimensions in view of speech
  synthesis", Eurospeech (isca-archive.org).
- Zaïdi, J. et al., "Daft-Exprt: Cross-Speaker Prosody Transfer on Any Text for Expressive
  Speech Synthesis", arXiv 2108.02271 / Interspeech 2022.
- Diatlova, D. and Shutov, V., "EmoSpeech: Guiding FastSpeech2 Towards Emotional Text to
  Speech", arXiv 2307.00024.
- "Enhancing In-the-Wild Speech Emotion Conversion with Resynthesis-based Duration Modeling",
  arXiv 2508.11535.
- "A Human-in-the-Loop Approach to Improving Cross-Text Prosody Transfer", arXiv 2406.06601
  (code: lordzuko/cross-text-PT, whose README is Daft-Exprt's).
