"""Two ways into a Piper voice's speaker vector, rendered for the maintainer to hear.

1. hfc_female with semaine's timing. A speaker id sets, among other things, how many frames
   each phoneme lasts, pauses included (the duration predictor takes the speaker vector,
   piper1-gpl src/piper/train/vits/models.py:792). hfc_female has one speaker and no vector,
   but both voices round their frame counts in the same Ceil node, 256 samples a frame at
   22 050 Hz. So semaine is asked, as Prudence and as Poppy, how long each of hfc_female's own
   phonemes should last, and hfc_female is then made to speak with those lengths. The pitch,
   timbre and loudness are hfc_female's; only the timing is borrowed. This is an input to the
   model, not an edit of its audio.
2. A walk between two semaine speakers. The speaker id picks one 512-number row of emb_g
   (models.py:706, 787). Feeding in a blend of two rows instead shows the voice changing
   continuously from one speaker to the other.

    python -m venv .venv && .venv/bin/pip install piper-tts onnx onnxruntime praat-parselmouth numpy soundfile
    .venv/bin/python borrow_timing.py <voices dir> <out dir>

<voices dir> as in ../style-voices/render_style_voices.py. Phoneme lengths use noise_w 0, the
predictor's mean, so every variant of a line is compared on the same take; noise_scale stays
at the voices' 0.667. Sentences are joined with the agent's 0.2 s pause
(agent/src/tts.rs:64). One gain per set, so its loudest clip peaks at -1 dBFS, then 8 kHz.
Prints each clip's length and, for the borrowed timing, how many frames moved. Lines are
invented (synthetic data).
"""
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

voices_dir = None  # set by open_voices
LINE_RATE, SENTENCE_SILENCE_S = 8000, 0.2
NOISE, LENGTH, NOISE_W = 0.667, 1.0, 0.0
LINES = {
    "sympathetic": "Oh no, I'm sorry to hear that. Are you hurt?",
    "glad": "That's wonderful news, I'm so happy for you!",
    "reassuring": "Don't worry, you're not alone. Someone will check on you tonight.",
    "serious": "Please stay where you are. I'm getting a person on the line now.",
}
SEMAINE = {"prudence": 0, "spike": 1, "obadiah": 2, "poppy": 3}


def voice_dir(name):
    return voices_dir / f"vits-piper-{name}"


def load(name):
    return PiperVoice.load(voice_dir(name) / f"{name}.onnx", voice_dir(name) / f"{name}.onnx.json")


def dedupe_metadata(m):
    seen = {}  # the sherpa-onnx copies list some metadata keys twice, which onnx rejects
    for prop in list(m.metadata_props):
        seen.setdefault(prop.key, prop.value)
    del m.metadata_props[:]
    for k, v in seen.items():
        m.metadata_props.add(key=k, value=v)


def session(m):
    dedupe_metadata(m)
    onnx.checker.check_model(m)
    return ort.InferenceSession(m.SerializeToString(), providers=["CPUExecutionProvider"])


def semaine_graph():
    """semaine with its frames per phoneme as an extra output, and the speaker vector as an
    input in place of the sid lookup."""
    m = onnx.load(str(voice_dir("en_GB-semaine-medium") / "en_GB-semaine-medium.onnx"))
    ceil = next(n for n in m.graph.node if n.op_type == "Ceil")
    m.graph.output.append(helper.make_tensor_value_info(ceil.output[0], TensorProto.FLOAT, [1, 1, "phonemes"]))
    gather = next(n for n in m.graph.node if n.name == "/emb_g/Gather")
    emb = onnx.numpy_helper.to_array(next(i for i in m.graph.initializer if i.name == "emb_g.weight"))
    m.graph.input.append(helper.make_tensor_value_info("g", TensorProto.FLOAT, [1, emb.shape[1]]))
    for n in m.graph.node:
        n.input[:] = ["g" if i == gather.output[0] else i for i in n.input]
    m.graph.node.remove(gather)
    m.graph.input.remove(next(i for i in m.graph.input if i.name == "sid"))
    return session(m), emb


def hfc_graph():
    """hfc_female that speaks with frame counts it is given instead of its own."""
    m = onnx.load(str(voice_dir("en_US-hfc_female-medium") / "en_US-hfc_female-medium.onnx"))
    ceil = next(n for n in m.graph.node if n.op_type == "Ceil")
    m.graph.input.append(helper.make_tensor_value_info("frames", TensorProto.FLOAT, [1, 1, "phonemes"]))
    m.graph.output.append(helper.make_tensor_value_info(ceil.output[0], TensorProto.FLOAT, [1, 1, "phonemes"]))
    for n in m.graph.node:
        if n is not ceil:
            n.input[:] = ["frames" if i == ceil.output[0] else i for i in n.input]
    return session(m)


def run(sess, ids, **extra):
    feed = {"input": np.array([ids], dtype=np.int64), "input_lengths": np.array([len(ids)], dtype=np.int64),
            "scales": np.array([NOISE, LENGTH, NOISE_W], dtype=np.float32), **extra}
    outs = sess.run(None, feed)
    return outs[0].squeeze(), (outs[1] if len(outs) > 1 else None)


def join(parts, rate):
    gap = np.zeros(int(SENTENCE_SILENCE_S * rate), dtype=np.float32)
    out = []
    for p in parts:
        out += [p, gap]
    return np.concatenate(out[:-1])


def write_set(clips, rate, out_dir):
    gain = 10 ** (-1 / 20) / max(np.max(np.abs(x)) for _, x in clips)
    for name, x in clips:
        line = call(parselmouth.Sound(x * gain, sampling_frequency=rate), "Resample", LINE_RATE, 50).values[0]
        sf.write(out_dir / f"{name}.wav", np.clip(line, -1, 1), LINE_RATE, subtype="PCM_16")
        print(f"{name + '.wav':46s} {len(x) / rate:5.2f} s")


def open_voices(path):
    """Both voices, semaine's rewired graph with its speaker vectors, and hfc_female's."""
    global voices_dir
    voices_dir = pathlib.Path(path)
    hfc, semaine = load("en_US-hfc_female-medium"), load("en_GB-semaine-medium")
    assert hfc.config.sample_rate == semaine.config.sample_rate
    s_sess, emb = semaine_graph()
    return hfc, semaine, s_sess, emb, hfc_graph()


def own_frames(h_sess, ids):
    """hfc_female's own frame counts; the audio made from the zeros fed in is thrown away."""
    return run(h_sess, ids, frames=np.zeros((1, 1, len(ids)), dtype=np.float32))[1]


def main():
    out_dir = pathlib.Path(sys.argv[2])
    out_dir.mkdir(parents=True, exist_ok=True)
    hfc, semaine, s_sess, emb, h_sess = open_voices(sys.argv[1])
    rate = hfc.config.sample_rate

    # --- 1. hfc_female with semaine's timing ----------------------------------------------------
    print("== hfc_female speaking with semaine's frame counts (noise_w 0)")
    clips = []
    for tone, text in LINES.items():
        variants = {"own": [], "prudence": [], "poppy": []}
        moved = {"prudence": 0, "poppy": 0}
        for phonemes in hfc.phonemize(text):
            ids = hfc.phonemes_to_ids(phonemes)
            assert semaine.phonemes_to_ids(phonemes) == ids, "the two voices number these phonemes differently"
            own = own_frames(h_sess, ids)
            audio, _ = run(h_sess, ids, frames=own)
            variants["own"].append(audio)
            for who in moved:
                _, borrowed = run(s_sess, ids, g=emb[[SEMAINE[who]]])
                audio, _ = run(h_sess, ids, frames=borrowed)
                variants[who].append(audio)
                moved[who] += int(np.abs(borrowed - own).sum())
        for who, parts in variants.items():
            clips.append((f"hfc_female-{tone}-{who}-timing", join(parts, rate)))
        print(f"  {tone}: frames moved, prudence {moved['prudence']}, poppy {moved['poppy']}")
    write_set(clips, rate, out_dir)

    # --- 2. A walk between two semaine speakers -------------------------------------------------
    print("\n== semaine, blends of two speaker vectors (noise_w 0)")
    for a, b, tone in [("obadiah", "poppy", "glad"), ("prudence", "poppy", "sympathetic")]:
        clips = []
        for w in [0.0, 0.25, 0.5, 0.75, 1.0]:
            g = ((1 - w) * emb[SEMAINE[a]] + w * emb[SEMAINE[b]])[None, :].astype(np.float32)
            parts = [run(s_sess, semaine.phonemes_to_ids(p), g=g)[0] for p in semaine.phonemize(LINES[tone])]
            clips.append((f"semaine-{tone}-{a}-to-{b}-{int(w * 100):03d}", join(parts, rate)))
        write_set(clips, rate, out_dir)


if __name__ == "__main__":
    main()
