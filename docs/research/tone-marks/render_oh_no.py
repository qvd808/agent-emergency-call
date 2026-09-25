"""Same words, different intent. One Piper take of "Oh no, I'm sorry to hear that." with the
vowel of "no" held, then only the pitch of "Oh no" (and, for some, its loudness) is changed
with Praat's PSOLA. Everything after "Oh no," is the same audio in every file, so the files
differ only where the intent does.

    python -m venv .venv && .venv/bin/pip install piper-tts onnx onnxruntime praat-parselmouth numpy soundfile
    .venv/bin/python render_oh_no.py <voice.onnx> <out dir> [hold frames]

The hold is the fixed-frames edit from ../expressive-voice/hold_no.py: extra frames added to
the vowel's duration just before Piper's Ceil node. Each intent's pitch targets are guesses
to listen to, not measured templates. Prints, for every file, what the pitch of "Oh" and "no"
came out as, measured with Praat, so each target can be checked against the audio.

Rejected by ear on 2026-09-25: Piper's own take (0) was best and every pitch edit was worse.
Kept only as the record of that result; see ../tone-marks.md.
"""
import json
import pathlib
import sys

import numpy as np
import onnx
import onnxruntime as ort
import parselmouth
import soundfile as sf
from onnx import TensorProto, helper
from parselmouth.praat import call

from piper import PiperVoice

model_path, out_dir = pathlib.Path(sys.argv[1]), pathlib.Path(sys.argv[2])
HOLD = int(sys.argv[3]) if len(sys.argv) > 3 else 26  # 26 frames = 302 ms at 22 050 Hz
out_dir.mkdir(parents=True, exist_ok=True)
config = json.loads(pathlib.Path(f"{model_path}.json").read_text())
RATE = config["audio"]["sample_rate"]
HOP = 256  # piper-tts 1.8.0 DEFAULT_HOP_LENGTH; checked below against the audio length
# Tone::Steady's length and noise (agent/src/tts.rs:57). noise_w 0 gives Piper's mean
# durations, so the hold lands on the same take every run.
LENGTH, NOISE, NOISE_W = 1.2, 0.6, 0.0
LINE_RATE = 8000  # what a softphone hears (agent/src/audio.rs LINE_RATE_HZ)

# --- Patch the graph: extra frames per phoneme, and the frame counts as an output ------------
m = onnx.load(str(model_path))
ceil = next(n for n in m.graph.node if n.op_type == "Ceil")
m.graph.input.append(
    helper.make_tensor_value_info("phoneme_extra_frames", TensorProto.FLOAT, [1, 1, "phonemes"])
)
m.graph.node.insert(
    list(m.graph.node).index(ceil),
    helper.make_node("Add", [ceil.input[0], "phoneme_extra_frames"], ["held_w"], name="per_phoneme_add"),
)
ceil.input[0] = "held_w"
m.graph.output.append(helper.make_tensor_value_info(ceil.output[0], TensorProto.FLOAT, [1, 1, "phonemes"]))
seen = {}  # the sherpa-onnx copy of hfc_female lists its metadata twice
for prop in list(m.metadata_props):
    seen.setdefault(prop.key, prop.value)
del m.metadata_props[:]
for k, v in seen.items():
    m.metadata_props.add(key=k, value=v)
onnx.checker.check_model(m)
session = ort.InferenceSession(m.SerializeToString(), providers=["CPUExecutionProvider"])
voice = PiperVoice.load(str(model_path))  # only for phonemize()

TEXT = "Oh no, I'm sorry to hear that."
ph = voice.phonemize(TEXT)[0]
print("phonemes:", "".join(ph))
idmap = config["phoneme_id_map"]
ids, owner = list(idmap["^"]) + list(idmap["_"]), [None, None]
for i, p in enumerate(ph):
    ids += idmap[p]
    owner += [i]
    ids += idmap["_"]
    owner += [None]
ids += idmap["$"]
owner += [None]

# "Oh" is ph[0..2] (ˈ o ʊ), then a space, then "no" (n ˈ o ʊ), then the comma.
oh_o, oh_u = ph.index("o"), ph.index("ʊ")
n = ph.index("n")
no_o = ph.index("o", n)
no_u = ph.index("ʊ", no_o)
extra = np.zeros((1, 1, len(ids)), dtype=np.float32)
extra[0, 0, owner.index(no_o)] = round(HOLD * 0.75)  # most of the hold on the vowel's start,
extra[0, 0, owner.index(no_u)] = HOLD - round(HOLD * 0.75)  # the rest on its glide

audio, w_ceil = session.run(
    None,
    {
        "input": np.array([ids], dtype=np.int64),
        "input_lengths": np.array([len(ids)], dtype=np.int64),
        "scales": np.array([NOISE, LENGTH, NOISE_W], dtype=np.float32),
        "phoneme_extra_frames": extra,
    },
)
audio = audio.squeeze().astype(np.float64)
samples = (w_ceil.squeeze() * HOP).astype(int)
assert samples.sum() == len(audio), (samples.sum(), len(audio))
edges = np.concatenate([[0], np.cumsum(samples)]) / RATE  # id k spans edges[k]..edges[k+1]


def spans(first, last):
    """Seconds from the start of phoneme `first` to the end of phoneme `last` (not its pad)."""
    return edges[owner.index(first)], edges[owner.index(last) + 1]


OH = spans(oh_o, oh_u)
NO = spans(no_o, no_u)
print(f'"Oh" vowel {OH[0]:.3f}-{OH[1]:.3f} s, "no" vowel {NO[0]:.3f}-{NO[1]:.3f} s '
      f"({(NO[1] - NO[0]) * 1000:.0f} ms, {HOLD} frames held)")

# --- Intents -----------------------------------------------------------------------------------
# Pitch targets in Hz at fractions of each vowel; Praat interpolates between them. None keeps
# Piper's own pitch. gain_db changes the loudness of "Oh no," only.
INTENTS = [
    ("0-as-piper", "Piper's own pitch with the hold: a peak on Oh, then a low flat no", None, 0.0),
    ("1-surprised", "high peak inside the no, like sample 08",
     {"oh": [(0, 250), (1, 265)], "no": [(0, 235), (0.35, 310), (1, 200)]}, 0.0),
    ("2-sorry-glide", "Oh lower, no glides down 238 to 182 Hz, never flat, softer",
     {"oh": [(0, 215), (1, 212)], "no": [(0, 238), (1, 182)]}, -3.0),
    ("3-sorry-rise-fall", "Oh lower, a small rise then fall inside the no (peak 238 Hz), softer",
     {"oh": [(0, 205), (1, 200)], "no": [(0, 198), (0.4, 238), (1, 188)]}, -3.0),
    ("4-sorry-fall-lift", "Oh lower, no falls to 182 Hz then lifts to 205 into I'm sorry, softer",
     {"oh": [(0, 215), (1, 212)], "no": [(0, 232), (0.65, 182), (1, 205)]}, -3.0),
    ("5-joking", "high sing-song no, louder: for contrast only",
     {"oh": [(0, 275), (1, 285)], "no": [(0, 250), (0.3, 340), (0.6, 245), (0.85, 300), (1, 270)]}, 2.0),
]


def render(targets, gain_db):
    snd = parselmouth.Sound(audio, sampling_frequency=RATE)
    out = audio
    if targets:
        manip = call(snd, "To Manipulation", 0.01, 75, 500)
        tier = call(manip, "Extract pitch tier")
        call(tier, "Remove points between", OH[0], NO[1])
        for word, (a, b) in (("oh", OH), ("no", NO)):
            for frac, hz in targets[word]:
                call(tier, "Add point", a + frac * (b - a), hz)
        call([manip, tier], "Replace pitch tier")
        out = call(manip, "Get resynthesis (overlap-add)").values[0]
    if gain_db:
        t = np.arange(len(out)) / RATE
        ramp = 0.03
        start, end = OH[0] - 0.05, NO[1] + 0.02
        inside = np.clip(np.minimum((t - start) / ramp, (end - t) / ramp), 0, 1)
        out = out * 10 ** (gain_db * inside / 20)
    return out


def measure(x, rate):
    """Median, max and end (last 30 ms) pitch of each vowel, and the dB of "Oh no" vs the rest."""
    snd = parselmouth.Sound(x, sampling_frequency=rate)
    p = snd.to_pitch_ac(time_step=0.005, pitch_floor=75, pitch_ceiling=500)
    t, f = p.xs(), p.selected_array["frequency"]
    res = {}
    for word, (a, b) in (("oh", OH), ("no", NO)):
        sel = (t >= a) & (t < b) & (f > 0)
        v, tv = f[sel], t[sel]
        res[word] = (np.median(v), v.max(), np.median(v[tv >= b - 0.03]))
    rms = lambda a, b: np.sqrt(np.mean(x[int(a * rate):int(b * rate)] ** 2))
    res["db"] = 20 * np.log10(rms(OH[0], NO[1]) / rms(NO[1] + 0.2, len(x) / rate))
    return res


print(f"\n{'file':22s} {'Oh: median/max/end Hz':>22s} {'no: median/max/end Hz':>22s} {'Oh no vs rest':>14s}")
for name, how, targets, gain_db in INTENTS:
    x = np.clip(render(targets, gain_db), -1, 1)
    line = call(parselmouth.Sound(x, sampling_frequency=RATE), "Resample", LINE_RATE, 50).values[0]
    sf.write(out_dir / f"{name}.wav", np.clip(line, -1, 1), LINE_RATE, subtype="PCM_16")
    r = measure(line, LINE_RATE)
    fmt = lambda w: "/".join(f"{v:.0f}" for v in r[w])
    print(f"{name:22s} {fmt('oh'):>22s} {fmt('no'):>22s} {r['db']:+11.1f} dB   {how}")
