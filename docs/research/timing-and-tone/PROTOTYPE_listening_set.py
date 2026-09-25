"""PROTOTYPE, throwaway: the listening set for issue #43, "Does each tone keep its picked timing
across more lines, without Poppy's wait before speaking?"

By ear, one line per tone, hfc_female sounded right with Prudence's timing for sympathetic,
Poppy's for glad and reassuring, and its own for serious (../timing-and-tone.md). This renders
five lines for each of the six tone marks (../tone-marks/tag_eval.py TONES), each with four
timings, for the maintainer to hear and pick:
- own                hfc_female's own timing;
- prudence, poppy    semaine's, borrowed through the Ceil node (../confident-endings/borrow_timing.py);
- poppy-own-lead-in  Poppy's timing, but hfc_female's own frames for the lead-in (the BOS id and
                     its PAD, as ../confident-endings/where_frames_go.py classes them). Poppy puts
                     0.45-0.93 s there on the first four lines, which a call would hear as delay.

The four takes of a line are named A-D in an order shuffled per line with a fixed seed, so they
can be heard blind; key.json says which is which. The first line of each tone is the one
already heard (borrow_timing.py LINES, and ../style-voices/render_style_voices.py EN for neutral
and playful). Some are the agent's own fixed lines (agent/src/conversation.rs). The rest are
invented (synthetic data).

Rendering as borrow_timing.py: noise_w 0, so every take of a line times the same way on every
run; noise_scale 0.667, so the voice's grain differs a little between takes and runs; 0.2 s
between sentences; one gain for the whole set, so its loudest clip peaks at -1 dBFS; then 8 kHz.
Prints, per take, ms per kind of sound as ../timing-and-tone/timing_by_tone.py does, then the
same averaged per tone.

    python -m venv .venv && .venv/bin/pip install piper-tts onnx onnxruntime praat-parselmouth numpy soundfile
    .venv/bin/python PROTOTYPE_listening_set.py <voices dir> <out dir>
"""
import json
import pathlib
import random
import sys

import numpy as np
import parselmouth
import soundfile as sf
from parselmouth.praat import call

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent.parent / "confident-endings"))
import borrow_timing as bt  # noqa: E402
from where_frames_go import MS, classes  # noqa: E402

LINES = {
    "sympathetic": [
        bt.LINES["sympathetic"],
        "Oh no, I'm sorry to hear that. Were you able to get up by yourself?",  # agent/src/tts.rs:194
        "I'm sorry, that sounds lonely. Do you have anyone to talk to this week?",
        "That sounds painful. Is your knee any better when you rest it?",
        "I'm sorry your sister is unwell. How are you holding up?",
    ],
    "glad": [
        bt.LINES["glad"],
        "Oh, how lovely that your granddaughter visited! What did you two do?",
        "A clear scan, that's great news! You must be so relieved.",
        "Well done on the walk, that's a long way! How do your legs feel?",
        "A new great-grandson, congratulations! What's his name?",
    ],
    "reassuring": [
        bt.LINES["reassuring"],
        "It's all right to feel nervous about the storm. Do you have a torch nearby?",
        "Waiting for results is hard. You're doing everything right.",
        "You did the right thing by telling me. Let's take it one step at a time.",
        "It's normal to feel shaky after a fall. Are you sitting down now?",
    ],
    "serious": [
        bt.LINES["serious"],
        # ESCALATION, agent/src/conversation.rs:84-85
        "I'm connecting you to a person now. Please stay on the line. "
        "If you are in danger, call nine one one yourself as soon as you can.",
        "I can't hear you. If you don't answer, I'll get a person on the line.",  # conversation.rs:98
        "Don't try to get up yet. Can you tell me where it hurts?",
        "Please stay still if your head hurts. Is anyone else in the house with you?",
    ],
    "neutral": [
        "Thanks. Did you have breakfast today?",
        "Hello. This is the automated check-in assistant. How are you feeling today?",  # conversation.rs:82
        "Okay, thank you. Have you had any pain today?",
        "Got it. Do you need anything right now?",
        "Thanks for letting me know. Have you taken your morning tablets?",
    ],
    "playful": [
        "Ha, a smoke alarm for a morning alarm. That's one way to wake up!",
        "The little thief! Did you get your slipper back?",
        "Soup-flavoured phone, that's a new one! Does it still work?",
        "She cheats, does she? Sounds like you need a rematch!",
        "Well, at least the toast is extra crispy. Did you have another go?",
    ],
}
TIMINGS = ["own", "prudence", "poppy", "poppy-own-lead-in"]
SPEECH = ["vowels", "consonants", "word end", "spaces/stress"]
COLUMNS = ["lead-in", "speech", "word end", "pause marks"]


def takes(hfc, semaine, s_sess, emb, h_sess, names, text):
    """The four takes of a line, each as (audio per sentence, ms per kind of sound, sounds)."""
    audio = {t: [] for t in TIMINGS}
    ms = {t: {} for t in TIMINGS}
    sounds = 0
    for phonemes in hfc.phonemize(text):
        ids = hfc.phonemes_to_ids(phonemes)
        assert semaine.phonemes_to_ids(phonemes) == ids, "the two voices number these phonemes differently"
        symbols = [names[i] for i in ids]
        cls = classes(symbols)
        sounds += sum(c in {"vowels", "consonants", "word end"} and s not in {"_", "ː"} for c, s in zip(cls, symbols))
        own = bt.own_frames(h_sess, ids)
        prudence = bt.run(s_sess, ids, g=emb[[bt.SEMAINE["prudence"]]])[1]
        poppy = bt.run(s_sess, ids, g=emb[[bt.SEMAINE["poppy"]]])[1]
        lead = np.array([c == "lead-in" for c in cls])
        frames = {"own": own, "prudence": prudence, "poppy": poppy,
                  "poppy-own-lead-in": np.where(lead[None, None, :], own, poppy)}
        for t, f in frames.items():
            audio[t].append(bt.run(h_sess, ids, frames=f)[0])
            for c, n in zip(cls, f[0, 0]):
                ms[t][c] = ms[t].get(c, 0.0) + n * MS
    return audio, ms, sounds


def row(ms, sounds):
    speech = sum(ms.get(c, 0.0) for c in SPEECH)
    return [ms.get("lead-in", 0.0), speech, ms.get("word end", 0.0), ms.get("pause marks", 0.0), sounds / speech * 1000]


def main():
    out_dir = pathlib.Path(sys.argv[2])
    out_dir.mkdir(parents=True, exist_ok=True)
    hfc, semaine, s_sess, emb, h_sess = bt.open_voices(sys.argv[1])
    rate = hfc.config.sample_rate
    names = {i[0]: p for p, i in hfc.config.phoneme_id_map.items()}

    clips, key, table = [], {}, []
    for tone, lines in LINES.items():
        key[tone] = []
        for n, text in enumerate(lines, 1):
            audio, ms, sounds = takes(hfc, semaine, s_sess, emb, h_sess, names, text)
            order = TIMINGS[:]
            random.Random(f"{tone}-{n}").shuffle(order)
            letters = dict(zip("ABCD", order))
            key[tone].append({"n": n, "text": text, "takes": letters})
            for letter, t in letters.items():
                x = bt.join(audio[t], rate)
                clips.append((f"{tone}-{n}-{letter}", x))
                table.append((tone, n, letter, t, *row(ms[t], sounds), len(x) / rate))

    gain = 10 ** (-1 / 20) / max(np.max(np.abs(x)) for _, x in clips)
    for name, x in clips:
        line = call(parselmouth.Sound(x * gain, sampling_frequency=rate), "Resample", bt.LINE_RATE, 50).values[0]
        sf.write(out_dir / f"{name}.wav", np.clip(line, -1, 1), bt.LINE_RATE, subtype="PCM_16")
    (out_dir / "key.json").write_text(json.dumps(key, indent=1) + "\n")

    head = f"{'lead-in':>8s} {'speech':>7s} {'word end':>9s} {'punct.':>7s} {'sounds/s':>9s}"
    print(f"== Per take: ms per kind of sound (noise_w 0), and the clip's length\n"
          f"{'tone':12s} {'n':>2s} {'take':4s} {'timing':18s} {head} {'clip s':>7s}")
    for tone, n, letter, t, *v, secs in table:
        print(f"{tone:12s} {n:2d} {letter:4s} {t:18s} {v[0]:8.0f} {v[1]:7.0f} {v[2]:9.0f} {v[3]:7.0f} {v[4]:9.1f} {secs:7.2f}")
    print(f"\n== Per tone, mean over its {len(LINES['glad'])} lines\n{'tone':12s} {'timing':18s} {head}")
    for tone in LINES:
        for t in TIMINGS:
            v = np.mean([r[4:9] for r in table if r[0] == tone and r[3] == t], axis=0)
            print(f"{tone:12s} {t:18s} {v[0]:8.0f} {v[1]:7.0f} {v[2]:9.0f} {v[3]:7.0f} {v[4]:9.1f}")


if __name__ == "__main__":
    main()
