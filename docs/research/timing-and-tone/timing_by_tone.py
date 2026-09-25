"""How timing differs with tone: in the timings the maintainer picked, and in one emotional voice.

1. The picks. hfc_female spoke four check-in lines with three timings each: its own, Prudence's
   and Poppy's (../confident-endings/borrow_timing.py). The maintainer picked one per line on
   2026-09-25 (PICKED below). For each line and timing, this prints how long each kind of sound
   lasts, so the picked timing can be read against the others.
2. One voice, eight emotions. de_DE-thorsten_emotional-medium is one speaker whose speaker ids
   are emotions. The same eight German sentences are timed under each emotion. This prints how
   each kind of sound changes against `neutral`.

Kinds of sound are those of ../confident-endings/where_frames_go.py: lead-in, vowels,
consonants, the last sound of a word, word boundaries and stress marks, and punctuation.
"Speech" is everything but the lead-in and punctuation. All timings are the duration
predictor's mean (noise_w 0), so they don't vary between runs.

    .venv/bin/python timing_by_tone.py <voices dir>

<voices dir> as in ../style-voices/render_style_voices.py. The German sentences are invented
(synthetic data).
"""
import pathlib
import sys

import numpy as np
import onnx
import onnxruntime as ort
from onnx import TensorProto, helper

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent.parent / "confident-endings"))
import borrow_timing as bt  # noqa: E402
from where_frames_go import CLASSES, MS, classes  # noqa: E402

PICKED = {"sympathetic": "prudence", "glad": "poppy", "reassuring": "poppy", "serious": "own"}
GERMAN = [
    "Guten Morgen, wie geht es Ihnen heute?",
    "Oh nein, das tut mir leid. Sind Sie verletzt?",
    "Das sind wunderbare Neuigkeiten, ich freue mich für Sie!",
    "Keine Sorge, Sie sind nicht allein.",
    "Bitte bleiben Sie, wo Sie sind.",
    "Haben Sie heute schon gefrühstückt?",
    "Ich rufe jetzt eine Person an, die Ihnen hilft.",
    "Ha, ein Rauchmelder als Wecker. So kann man auch aufwachen!",
]
SPEECH = ["vowels", "consonants", "word end", "spaces/stress"]


def ms_by_class(symbols, frames, into):
    for cls, f in zip(classes(symbols), frames):
        into[cls] = into.get(cls, 0.0) + f * MS
    return into


def speech_ms(t):
    return sum(t.get(c, 0.0) for c in SPEECH)


# --- 1. The picks ------------------------------------------------------------------------------
hfc, semaine, s_sess, emb, h_sess = bt.open_voices(sys.argv[1])
names = {i[0]: p for p, i in hfc.config.phoneme_id_map.items()}
print("== hfc_female's four lines: ms per kind of sound under each timing (* = picked by ear)")
print(f"{'line':12s} {'timing':9s} {'lead-in':>8s} {'speech':>7s} {'word end':>9s} {'vowels':>7s} {'punct.':>7s} {'sounds/s':>9s}")
for tone, text in bt.LINES.items():
    totals = {"own": {}, "prudence": {}, "poppy": {}}
    sounds = 0
    for phonemes in hfc.phonemize(text):
        ids = hfc.phonemes_to_ids(phonemes)
        symbols = [names[i] for i in ids]
        sounds += sum(c in {"vowels", "consonants", "word end"} and s not in {"_", "ː"} for c, s in zip(classes(symbols), symbols))
        ms_by_class(symbols, bt.own_frames(h_sess, ids)[0, 0], totals["own"])
        for who in ["prudence", "poppy"]:
            ms_by_class(symbols, bt.run(s_sess, ids, g=emb[[bt.SEMAINE[who]]])[1][0, 0], totals[who])
    for who, t in totals.items():
        mark = "*" if PICKED[tone] == who else " "
        print(f"{tone:12s} {who + mark:9s} {t['lead-in']:8.0f} {speech_ms(t):7.0f} {t['word end']:9.0f} "
              f"{t['vowels']:7.0f} {t['pause marks']:7.0f} {sounds / speech_ms(t) * 1000:9.1f}")

# --- 2. One voice, eight emotions --------------------------------------------------------------
name = "de_DE-thorsten_emotional-medium"
thorsten = bt.PiperVoice.load(bt.voice_dir(name) / f"{name}.onnx", bt.voice_dir(name) / f"{name}.onnx.json")
m = onnx.load(str(bt.voice_dir(name) / f"{name}.onnx"))
ceil = next(n for n in m.graph.node if n.op_type == "Ceil")
m.graph.output.append(helper.make_tensor_value_info(ceil.output[0], TensorProto.FLOAT, [1, 1, "phonemes"]))
t_sess = bt.session(m)
t_names = {i[0]: p for p, i in thorsten.config.phoneme_id_map.items()}
by_emotion = {}
for emotion, sid in thorsten.config.speaker_id_map.items():
    t = {}
    for text in GERMAN:
        for phonemes in thorsten.phonemize(text):
            ids = thorsten.phonemes_to_ids(phonemes)
            frames = bt.run(t_sess, ids, sid=np.array([sid], dtype=np.int64))[1][0, 0]
            ms_by_class([t_names[i] for i in ids], frames, t)
    by_emotion[emotion] = t
base = by_emotion["neutral"]
print(f"\n== thorsten_emotional, {len(GERMAN)} German sentences: change against neutral, per kind of sound")
print(f"{'emotion':10s} {'speech ms':>9s} {'speech':>7s} " + " ".join(f"{c:>13s}" for c in CLASSES))
for emotion, t in by_emotion.items():
    print(f"{emotion:10s} {speech_ms(t):9.0f} {speech_ms(t) / speech_ms(base) - 1:+7.0%} "
          + " ".join(f"{t.get(c, 0) / base[c] - 1:+13.0%}" for c in CLASSES))
