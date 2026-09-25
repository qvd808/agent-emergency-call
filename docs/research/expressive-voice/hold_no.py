"""Which Piper tricks lengthen the "no" in "Oh no, I'm sorry to hear that."? Issue #38.

    python -m venv .venv && .venv/bin/pip install piper-tts onnx onnxruntime praat-parselmouth numpy soundfile
    .venv/bin/python hold_no.py <voice.onnx> <out dir> [runs] [noise_w]

Patches the voice in memory (the file on disk is not changed):
  - the per-phoneme frame counts (the `Ceil` tensor, w_ceil) become a second output, as
    piper-tts 1.8.0's piper/patch_voice_with_alignment.py does, so every variant's "no" is
    measured exactly from the model's own durations;
  - a new input `phoneme_length_scales` [1, 1, ids] multiplies the durations just before
    that Ceil, so one phoneme can be lengthened on its own;
  - a new input `phoneme_extra_frames` [1, 1, ids] is then added, so a phoneme can be held
    for a fixed extra time whatever Piper's random duration for it was.
Each variant is synthesised `runs` times at the Steady tone's settings from agent/src/tts.rs
(length 1.2, noise 0.6, noise_w 0.7). Piper's durations are random through noise_w; a
`noise_w` of 0 makes them the same on every run, so the variants differ only by the trick.
Prints one row per variant, and writes the first run of each as a WAV and all runs to
results.json in <out dir>.
"""
import json
import pathlib
import sys

import numpy as np
import onnx
import onnxruntime as ort
import parselmouth
from onnx import TensorProto, helper
from parselmouth.praat import call

from piper import PiperVoice

model_path, out_dir = pathlib.Path(sys.argv[1]), pathlib.Path(sys.argv[2])
runs = int(sys.argv[3]) if len(sys.argv) > 3 else 10
out_dir.mkdir(parents=True, exist_ok=True)
config = json.loads(pathlib.Path(f"{model_path}.json").read_text())
RATE = config["audio"]["sample_rate"]
HOP = 256  # piper-tts 1.8.0 DEFAULT_HOP_LENGTH; checked below against the audio length
LENGTH, NOISE, NOISE_W = 1.2, 0.6, 0.7  # Tone::Steady, agent/src/tts.rs
if len(sys.argv) > 4:
    NOISE_W = float(sys.argv[4])
SENTENCE_SILENCE = 0.2  # agent/src/tts.rs SENTENCE_SILENCE_S

# --- Patch the graph -------------------------------------------------------------------------
m = onnx.load(str(model_path))
ceil = next(n for n in m.graph.node if n.op_type == "Ceil")
before_ceil = ceil.input[0]  # exp(logw) * x_mask * length_scale
m.graph.input.append(
    helper.make_tensor_value_info("phoneme_length_scales", TensorProto.FLOAT, [1, 1, "phonemes"])
)
# The new Mul goes right before the Ceil, which now reads it (nodes must stay in
# topological order).
at = list(m.graph.node).index(ceil)
m.graph.node.insert(
    at, helper.make_node("Mul", [before_ceil, "phoneme_length_scales"], ["scaled_w"], name="per_phoneme_scale")
)
m.graph.input.append(
    helper.make_tensor_value_info("phoneme_extra_frames", TensorProto.FLOAT, [1, 1, "phonemes"])
)
m.graph.node.insert(
    at + 1, helper.make_node("Add", ["scaled_w", "phoneme_extra_frames"], ["held_w"], name="per_phoneme_add")
)
ceil.input[0] = "held_w"
m.graph.output.append(helper.make_tensor_value_info(ceil.output[0], TensorProto.FLOAT, [1, 1, "phonemes"]))
# The sherpa-onnx copy of hfc_female lists its metadata twice, which the checker rejects.
seen = {}
for prop in list(m.metadata_props):
    seen.setdefault(prop.key, prop.value)
del m.metadata_props[:]
for k, v in seen.items():
    m.metadata_props.add(key=k, value=v)
onnx.checker.check_model(m)
session = ort.InferenceSession(m.SerializeToString(), providers=["CPUExecutionProvider"])

voice = PiperVoice.load(str(model_path))  # only for phonemize() and phonemes_to_ids()


def ids_of(phonemes):
    """piper-tts ids, and for each id the index of the phoneme it came from (None: BOS/PAD/EOS)."""
    ids, owner = [], []
    idmap = config["phoneme_id_map"]
    ids += idmap["^"]; owner += [None]
    ids += idmap["_"]; owner += [None]
    for i, p in enumerate(phonemes):
        if p not in idmap:
            continue
        ids += idmap[p]; owner += [i]
        ids += idmap["_"]; owner += [None]
    ids += idmap["$"]; owner += [None]
    return ids, owner


def synth(phonemes, length=LENGTH, scale_phonemes=None, extra_frames=None):
    """Audio (float), samples per id, and the owner of each id."""
    ids, owner = ids_of(phonemes)
    scales = np.ones((1, 1, len(ids)), dtype=np.float32)
    extra = np.zeros((1, 1, len(ids)), dtype=np.float32)
    for i, n in (extra_frames or {}).items():
        extra[0, 0, owner.index(i)] = n  # the phoneme's own id only, not its pad
    for i, f in (scale_phonemes or {}).items():
        for k, o in enumerate(owner):
            # The phoneme's own id and the pad after it.
            if o == i or (k > 0 and owner[k - 1] == i and o is None):
                scales[0, 0, k] = f
    audio, w_ceil = session.run(
        None,
        {
            "input": np.array([ids], dtype=np.int64),
            "input_lengths": np.array([len(ids)], dtype=np.int64),
            "scales": np.array([NOISE, length, NOISE_W], dtype=np.float32),
            "phoneme_length_scales": scales,
            "phoneme_extra_frames": extra,
        },
    )
    audio = audio.squeeze()
    samples = (w_ceil.squeeze() * HOP).astype(int)
    assert samples.sum() == len(audio), (samples.sum(), len(audio))
    return audio, samples, owner


def span(samples, owner, first, last):
    """(start, end) sample of phonemes first..last, pads between them included."""
    start = end = None
    pos = 0
    for k, n in enumerate(samples):
        o = owner[k]
        if o == first and start is None:
            start = pos
        if o is not None and first <= o <= last:
            end = pos + n
        if end is not None and o is None and k > 0 and owner[k - 1] == last:
            end = pos + n  # the pad right after the last phoneme
        pos += n
    return start, end


def pitch_of(audio, start, end):
    """Praat pitch over [start, end): median Hz, and first/last voiced Hz, in the segment."""
    snd = parselmouth.Sound(audio.astype(np.float64), sampling_frequency=RATE)
    pitch = snd.to_pitch_ac(time_step=0.005, pitch_floor=75, pitch_ceiling=500)
    t = pitch.xs()
    f = pitch.selected_array["frequency"]
    sel = (t >= start / RATE) & (t < end / RATE) & (f > 0)
    if sel.sum() < 2:
        return None, None, None
    v = f[sel]
    return float(np.median(v)), float(v[0]), float(v[-1])


def stretch(audio, start, end, factor):
    """Lengthen [start, end) by `factor` with Praat's PSOLA (Manipulation + DurationTier)."""
    snd = parselmouth.Sound(audio.astype(np.float64), sampling_frequency=RATE)
    manip = call(snd, "To Manipulation", 0.01, 75, 500)
    tier = call(manip, "Extract duration tier")
    s, e = start / RATE, end / RATE
    call(tier, "Add point", s - 0.001, 1.0)
    call(tier, "Add point", s, factor)
    call(tier, "Add point", e, factor)
    call(tier, "Add point", e + 0.001, 1.0)
    call([manip, tier], "Replace duration tier")
    return call(manip, "Get resynthesis (overlap-add)").values[0]


def write(name, audio):
    import soundfile as sf
    sf.write(out_dir / f"{name}.wav", audio, RATE, subtype="PCM_16")


TEXT = "Oh no, I'm sorry to hear that."
base_ph = voice.phonemize(TEXT)[0]
print("espeak phonemes:", "".join(base_ph))


def locate(ph, end_mark=","):
    """Index range of the word "no": its n through the last phoneme before the mark after it."""
    n = ph.index("n")
    end = ph.index(end_mark, n)
    return n, end - 1


def vowel_start(ph, n):
    return next(i for i in range(n + 1, len(ph)) if ph[i] not in "ˈˌ")


def with_no(no_phonemes):
    """The base phonemes with the word "no" replaced."""
    n, last = locate(base_ph)
    return base_ph[:n] + list(no_phonemes) + base_ph[last + 1 :]


variants = []  # (name, how, function returning (audio, (start, end) sample of "no"))


def plain(name, how, ph):
    def run():
        audio, samples, owner = synth(ph)
        n, last = locate(ph)
        return audio, span(samples, owner, n, last)
    variants.append((name, how, run))


def text_variant(name, text):
    ph = voice.phonemize(text)[0]
    plain(name, f'text "{text}" -> {"".join(ph)}', ph)


plain("base", "as espeak phonemizes it", base_ph)
text_variant("text-nooo", "Oh nooo, I'm sorry to hear that.")
text_variant("text-noooo", "Oh noooo, I'm sorry to hear that.")
text_variant("text-oh-comma-no", "Oh, no, I'm sorry to hear that.")
for no in ["nˈoːʊ", "nˈoʊː", "nˈoːʊː", "nˈoʊoʊ", "nˈooʊ", "nˈoːoːʊ"]:
    plain(f"phonemes-{no}", f"phonemes, no = {no}", with_no(no))


def per_phoneme(name, factor, whole_word=False):
    def run():
        n, last = locate(base_ph)
        first = n if whole_word else vowel_start(base_ph, n)
        scale = {i: factor for i in range(first, last + 1)}
        audio, samples, owner = synth(base_ph, scale_phonemes=scale)
        return audio, span(samples, owner, n, last)
    what = "n and the vowel" if whole_word else "the vowel oʊ only"
    variants.append((name, f"per-phoneme length x{factor} on {what} (patched model)", run))


for f in [1.5, 2.0, 2.5]:
    per_phoneme(f"per-phoneme-vowel-x{f}", f)
per_phoneme("per-phoneme-word-x2.0", 2.0, whole_word=True)


def hold(name, frames):
    """A fixed number of frames added to the vowel of "no", split over o and ʊ. The Ceil rounds
    each share up, so the total can come out a frame long."""
    def run():
        n, last = locate(base_ph)
        v0 = vowel_start(base_ph, n)
        vowel = list(range(v0, last + 1))
        extra = {i: frames / len(vowel) for i in vowel}
        audio, samples, owner = synth(base_ph, extra_frames=extra)
        return audio, span(samples, owner, n, last)
    ms = frames * HOP / RATE * 1000
    variants.append((name, f"vowel of no held {frames} frames longer (+{ms:.0f} ms, patched model)", run))


hold("hold-vowel-+13-frames", 13)
hold("hold-vowel-+20-frames", 20)


def clause(name, factor):
    """ "Oh no," on its own at length x factor, then the rest at the tone's length."""
    def run():
        comma = base_ph.index(",")
        head, tail = base_ph[: comma + 1], base_ph[comma + 1 :]
        while tail and tail[0] == " ":
            tail = tail[1:]
        a1, s1, o1 = synth(head, length=LENGTH * factor)
        a2, _, _ = synth(tail)
        n, last = locate(head)
        return np.concatenate([a1, a2]), span(s1, o1, n, last)
    variants.append((name, f'"Oh no," synthesised alone at length_scale {LENGTH}x{factor}', run))


for f in [1.5, 2.0]:
    clause(f"clause-x{f}", f)


def psola(name, factor):
    def run():
        audio, samples, owner = synth(base_ph)
        n, last = locate(base_ph)
        v0 = vowel_start(base_ph, n)
        vs, ve = span(samples, owner, v0, last)
        ns, ne = span(samples, owner, n, last)
        out = stretch(audio, vs, ve, factor)
        grow = len(out) - len(audio)
        return out, (ns, ne + grow)
    variants.append((name, f"base audio, vowel of no stretched x{factor} afterwards with PSOLA (Praat)", run))


def sentence(name, head_text, factor=1.0):
    """ "Oh no." as its own sentence, as tts.rs synthesises each sentence alone, 0.2 s between."""
    def run():
        head = voice.phonemize(head_text)[0]
        tail = voice.phonemize("I'm sorry to hear that.")[0]
        a1, s1, o1 = synth(head, length=LENGTH * factor)
        a2, _, _ = synth(tail)
        n, last = locate(head, ".")
        gap = np.zeros(int(SENTENCE_SILENCE * RATE), dtype=a1.dtype)
        return np.concatenate([a1, gap, a2]), span(s1, o1, n, last)
    variants.append((name, f'"{head_text}" as its own sentence at length_scale {LENGTH}x{factor}, 0.2 s, then the rest', run))


sentence("sentence-oh-no", "Oh no.")
sentence("sentence-oh-no-x1.3", "Oh no.", 1.3)


for f in [1.5, 2.0]:
    psola(f"psola-vowel-x{f}", f)

print(f"\n{runs} runs each, Steady tone (length {LENGTH}, noise {NOISE}, noise_w {NOISE_W}), {model_path.name}")
print(f"{'variant':26} {'no ms':>13} {'total ms':>14} {'no F0 med':>9} {'no F0 move st':>13}  how")
rows = {}
for name, how, run in variants:
    no_ms, tot_ms, med, se = [], [], [], []
    for r in range(runs):
        audio, (s, e) = run()
        if r == 0:
            write(name, audio)
        no_ms.append((e - s) / RATE * 1000)
        tot_ms.append(len(audio) / RATE * 1000)
        p = pitch_of(audio, s, e)
        if p[0] is not None:
            med.append(p[0]); se.append((p[1], p[2]))
    rows[name] = dict(how=how, no_ms=no_ms, total_ms=tot_ms, f0_median=med, f0_start_end=se)
    move = np.mean([12 * np.log2(b / a) for a, b in se]) if se else float("nan")
    rows[name]["f0_move_st"] = move
    print(f"{name:26} {np.mean(no_ms):5.0f} ({min(no_ms):3.0f}-{max(no_ms):3.0f}) "
          f"{np.mean(tot_ms):6.0f} ({np.std(tot_ms):3.0f}) {np.mean(med) if med else float('nan'):9.0f} "
          f"{move:+13.1f}  {how}")
(out_dir / "results.json").write_text(json.dumps(rows, indent=1))
