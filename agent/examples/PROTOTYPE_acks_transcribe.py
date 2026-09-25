"""PROTOTYPE, throwaway: transcribes the takes `PROTOTYPE_acks` wrote, one by one, and counts
per candidate how many a speech recogniser heard as the intended word. The recogniser stands
in for a listener: a take it can't make out is one a resident may not either (inferred).

Whisper small.en through sherpa-onnx, from the k2-fsa/sherpa-onnx release `asr-models`,
`sherpa-onnx-whisper-small.en.tar.bz2`. It is larger than the agent's tiny.en, so a miss here
is less likely to be the recogniser's fault (inferred).

    python -m venv .venv && .venv/bin/pip install sherpa-onnx soundfile numpy
    .venv/bin/python PROTOTYPE_acks_transcribe.py <takes dir> <sherpa-onnx-whisper-small.en dir>
"""
import collections
import pathlib
import re
import sys

import sherpa_onnx
import soundfile as sf

# What counts as heard right: the words, ignoring case and punctuation.
EXPECTED = {
    "mm-hm": {"mm hm", "mhm", "mmhm", "mm hmm", "mmhmm"},
    "i-see": {"i see"},
    "okay": {"okay", "ok"},
    "right": {"right"},
    "got-it": {"got it"},
    "uh-huh": {"uh huh", "uh-huh"},
    "alright": {"alright", "all right"},
}
for phonemes in ("ph-syllabic", "ph-long", "ph-schwa", "ph-strut"):
    EXPECTED[phonemes] = EXPECTED["mm-hm"]
for setting in ("warm-flat", "steady", "default"):
    for word in ("i-see", "okay"):
        EXPECTED[f"{setting}_{word}"] = EXPECTED[word]

takes, model = pathlib.Path(sys.argv[1]), pathlib.Path(sys.argv[2])
rec = sherpa_onnx.OfflineRecognizer.from_whisper(
    encoder=str(model / "small.en-encoder.onnx"),
    decoder=str(model / "small.en-decoder.onnx"),
    tokens=str(model / "small.en-tokens.txt"),
    num_threads=4,
)


def words(text):
    return re.sub(r"[^a-z' ]+", " ", text.lower().replace("-", " ")).split()


heard = collections.defaultdict(list)
for path in sorted(takes.glob("*.wav")):
    slug = path.stem.rsplit("-", 1)[0]
    audio, rate = sf.read(path, dtype="float32")
    stream = rec.create_stream()
    stream.accept_waveform(rate, audio)
    rec.decode_stream(stream)
    heard[slug].append((stream.result.text.strip(), len(audio) / rate))

for slug, results in heard.items():
    right = sum(" ".join(words(t)) in {" ".join(words(e)) for e in EXPECTED[slug]} for t, _ in results)
    ms = sorted(round(s * 1000) for _, s in results)
    print(f"{slug:8} {right:2}/{len(results)} heard right, {ms[0]}-{ms[-1]} ms long")
    for text, count in collections.Counter(t for t, _ in results).most_common():
        print(f"           {count:2} x {text!r}")
