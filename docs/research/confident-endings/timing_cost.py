"""What borrowing semaine's timing would cost per reply, on this machine's CPU.

Only semaine's frame counts are needed, not its audio, so its graph is cut down to what
computes them: the text encoder and the duration predictor, from the phoneme ids and the
speaker vector to the Ceil node (onnx.utils.extract_model). Times, per sentence of
borrow_timing.py's lines, over 20 runs after one warm-up:
- hfc_female alone, as the agent runs it today;
- the cut-down semaine graph;
- hfc_female speaking with the frame counts it is given.

    .venv/bin/python timing_cost.py <voices dir>
"""
import sys
import tempfile
import time

import numpy as np
import onnx
import onnxruntime as ort
import onnx.utils

import borrow_timing as bt

hfc, semaine, s_sess, emb, h_sess = bt.open_voices(sys.argv[1])
full = onnx.load(str(bt.voice_dir("en_GB-semaine-medium") / "en_GB-semaine-medium.onnx"))
ceil = next(n for n in full.graph.node if n.op_type == "Ceil").output[0]
with tempfile.TemporaryDirectory() as tmp:
    path = f"{tmp}/semaine.onnx"
    bt.dedupe_metadata(full)
    onnx.save(full, path)
    onnx.utils.extract_model(path, f"{tmp}/durations.onnx", ["input", "input_lengths", "scales", "sid"], [ceil])
    d_sess = ort.InferenceSession(f"{tmp}/durations.onnx", providers=["CPUExecutionProvider"])
    plain = ort.InferenceSession(str(bt.voice_dir("en_US-hfc_female-medium") / "en_US-hfc_female-medium.onnx"),
                                 providers=["CPUExecutionProvider"])
RUNS = 20


def timed(f):
    f()
    t = time.perf_counter()
    for _ in range(RUNS):
        f()
    return (time.perf_counter() - t) / RUNS * 1000


def feed(ids, **extra):
    return {"input": np.array([ids], dtype=np.int64), "input_lengths": np.array([len(ids)], dtype=np.int64),
            "scales": np.array([bt.NOISE, bt.LENGTH, bt.NOISE_W], dtype=np.float32), **extra}


print(f"{'sentence':48s} {'hfc ms':>7s} {'durations ms':>12s} {'hfc+frames ms':>13s}")
for text in bt.LINES.values():
    for phonemes in hfc.phonemize(text):
        ids = hfc.phonemes_to_ids(phonemes)
        sid = {"sid": np.array([bt.SEMAINE["prudence"]], dtype=np.int64)}
        frames = d_sess.run(None, feed(ids, **sid))[0]
        a = timed(lambda: plain.run(None, feed(ids)))
        b = timed(lambda: d_sess.run(None, feed(ids, **sid)))
        c = timed(lambda: h_sess.run(None, feed(ids, frames=frames)))
        print(f"{''.join(phonemes)[:46]:48s} {a:7.1f} {b:12.1f} {c:13.1f}")
