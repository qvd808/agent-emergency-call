"""Time one synthesis of the reply with the stock model and with the patched one (all scales 1,
and the vowel of "no" x2), and the Praat PSOLA stretch. This container, 4 vCPUs: relative only."""
import time, numpy as np, onnx, onnxruntime as ort, parselmouth, json, sys
from onnx import TensorProto, helper
from parselmouth.praat import call
from piper import PiperVoice
path = sys.argv[1]
cfg = json.load(open(path + ".json")); RATE = cfg["audio"]["sample_rate"]
voice = PiperVoice.load(path)
ph = voice.phonemize("Oh no, I'm sorry to hear that. Were you able to get up by yourself?")
ids = [voice.phonemes_to_ids(p) for p in ph]
stock = ort.InferenceSession(path, providers=["CPUExecutionProvider"])
m = onnx.load(path)
ceil = next(n for n in m.graph.node if n.op_type == "Ceil")
m.graph.input.append(helper.make_tensor_value_info("phoneme_length_scales", TensorProto.FLOAT, [1, 1, "phonemes"]))
m.graph.node.insert(list(m.graph.node).index(ceil), helper.make_node("Mul", [ceil.input[0], "phoneme_length_scales"], ["scaled_w"]))
ceil.input[0] = "scaled_w"
m.graph.output.append(helper.make_tensor_value_info(ceil.output[0], TensorProto.FLOAT, [1, 1, "phonemes"]))
patched = ort.InferenceSession(m.SerializeToString(), providers=["CPUExecutionProvider"])
def feed(i, extra):
    d = {"input": np.array([i], np.int64), "input_lengths": np.array([len(i)], np.int64), "scales": np.array([0.6, 1.2, 0.7], np.float32)}
    if extra: d["phoneme_length_scales"] = np.ones((1, 1, len(i)), np.float32)
    return d
def run(sess, extra, n=30):
    t = []
    for _ in range(n):
        s = time.perf_counter(); audio = [sess.run(None, feed(i, extra))[0] for i in ids]; t.append(time.perf_counter() - s)
    secs = sum(a.size for a in audio) / RATE
    return np.median(t) * 1000, secs
for name, sess, extra in [("stock", stock, False), ("patched", patched, True), ("stock", stock, False), ("patched", patched, True)]:
    ms, secs = run(sess, extra)
    print(f"{name:8} median {ms:6.1f} ms for {secs:.2f} s of speech")
audio = np.concatenate([stock.run(None, feed(i, False))[0].squeeze() for i in ids])
snd = parselmouth.Sound(audio.astype(np.float64), sampling_frequency=RATE)
t = []
for _ in range(10):
    s = time.perf_counter()
    manip = call(snd, "To Manipulation", 0.01, 75, 500); tier = call(manip, "Extract duration tier")
    for tt, f in [(0.30, 1.0), (0.301, 2.0), (0.45, 2.0), (0.451, 1.0)]: call(tier, "Add point", tt, f)
    call([manip, tier], "Replace duration tier"); call(manip, "Get resynthesis (overlap-add)")
    t.append(time.perf_counter() - s)
print(f"Praat PSOLA stretch of one vowel, whole reply re-synthesised: median {np.median(t)*1000:.1f} ms")
