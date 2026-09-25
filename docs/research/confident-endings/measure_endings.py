"""What is different about how semaine's Poppy and Prudence end a sentence, next to hfc_female?

The maintainer heard, on 2026-09-25, a clear stop and a confident tone at the end of each word
in Poppy and Prudence, and kept hfc_female as the more comfortable voice. This measures the
cues that could carry that, on the same text in every source:

- the two actors' own recordings, from the data the semaine voice was trained on;
- Piper's semaine voice as Poppy and as Prudence, and hfc_female.

The text is 48 sentences both actors read with identical wording (DFKI's "w" prompts), plus
the six check-in lines from ../style-voices/render_style_voices.py, which only Piper speaks.

    python -m venv .venv && .venv/bin/pip install piper-tts onnxruntime praat-parselmouth numpy soundfile pyyaml
    .venv/bin/python measure_endings.py <voices dir> <dfki-semaine-data clone> <flac dir>

<voices dir> as in ../style-voices/render_style_voices.py. The FLAC files are the release
assets https://github.com/marytts/dfki-semaine-data/releases/download/v0.1/dfki-<name>-data.flac

Per sentence, with the leading and trailing silence cut off (Praat, -25 dB below the peak):
- speech_s   seconds of sound, internal pauses excluded
- pauses     internal pauses of 60 ms or more, and their total in ms
- f0         median pitch (Hz) and 10th-90th percentile spread (semitones)
- pre_pause_st  pitch over the last 100 ms before each internal pause, in semitones from the
             sentence's median, averaged over the sentence's pauses (a phrase ending inside it)
- end_st     pitch over the last 100 ms of voicing, in semitones from the sentence's median
- end_slope  pitch slope over the last 200 ms of voicing, semitones per second
- decay_ms   time the sound takes to fall from 3 dB to 25 dB below the loudest point of the
             last 300 ms: how sharply it stops
- tilt_db    energy at 1-5 kHz relative to 50 Hz-1 kHz in voiced frames: higher is brighter,
             more effort (recordings and Piper differ in microphone and room, so compare
             Piper with Piper)
Each Piper sentence is drawn at the voice's default scales (noise_w 0.8 draws random phoneme
lengths), twice for the DFKI sentences and 8 times for the check-in lines. Prints medians per source, then how often each voice sits on each side of
hfc_female for the same sentence. Sentences are DFKI's; the check-in lines are invented.
"""
import pathlib
import re
import sys

import numpy as np
import parselmouth
import soundfile as sf
import yaml
from parselmouth.praat import call

from piper import PiperVoice, SynthesisConfig

voices_dir, data_dir, flac_dir = (pathlib.Path(a) for a in sys.argv[1:4])
N_SENTENCES, DRAWS, CHECK_IN_DRAWS = 48, 2, 8
CHECK_IN = [
    "Oh no, I'm sorry to hear that. Are you hurt?",
    "That's wonderful news, I'm so happy for you!",
    "Ha, a smoke alarm for a morning alarm. That's one way to wake up!",
    "Don't worry, you're not alone. Someone will check on you tonight.",
    "Please stay where you are. I'm getting a person on the line now.",
    "Thanks. Did you have breakfast today?",
]


def features(x, rate):
    snd = parselmouth.Sound(x.astype(np.float64), sampling_frequency=rate)
    grid = call(snd, "To TextGrid (silences)", 100, 0.0, -25.0, 0.06, 0.05, "silent", "sounding")
    spans = []
    for i in range(1, int(call(grid, "Get number of intervals", 1)) + 1):
        spans.append((call(grid, "Get label of interval", 1, i),
                      call(grid, "Get start time of interval", 1, i),
                      call(grid, "Get end time of interval", 1, i)))
    sounding = [(a, b) for label, a, b in spans if label == "sounding"]
    t0, t1 = sounding[0][0], sounding[-1][1]
    pauses = [b - a for label, a, b in spans if label == "silent" and a > t0 and b < t1]
    # How sharply the sound stops is measured on the untrimmed clip, which has the silence after
    # the last word to fall into.
    inten = snd.to_intensity(minimum_pitch=70, time_step=0.005)
    db, it = inten.values[0], inten.xs()
    tail = (it > t1 - 0.3) & (it <= t1)
    peak_i = np.flatnonzero(tail)[np.argmax(db[tail])]
    after = db[peak_i:]
    a = np.flatnonzero(after < db[peak_i] - 3)
    b = np.flatnonzero(after < db[peak_i] - 25)
    decay = (b[0] - a[0]) * 5.0 if len(a) and len(b) else np.nan

    snd = snd.extract_part(t0, t1, parselmouth.WindowShape.RECTANGULAR, 1.0, False)
    dur = t1 - t0

    pitch = snd.to_pitch(time_step=0.01, pitch_floor=70, pitch_ceiling=600)
    f0 = pitch.selected_array["frequency"]
    times = pitch.xs()
    voiced = f0 > 0
    med = np.median(f0[voiced])
    st = 12 * np.log2(np.where(voiced, f0, np.nan) / med)
    last = times[voiced][-1]
    end_st = np.nanmedian(st[voiced & (times > last - 0.1)])
    w = voiced & (times > last - 0.2)
    end_slope = np.polyfit(times[w], st[w], 1)[0] if w.sum() >= 3 else np.nan
    lo, hi = np.percentile(f0[voiced], [10, 90])
    # Pitch just before each internal pause: where a phrase ends inside the sentence.
    before = [np.nanmedian(st[voiced & (times > a - t0 - 0.1) & (times <= a - t0)])
              for label, a, b in spans if label == "silent" and a > t0 and b < t1]
    before = [v for v in before if not np.isnan(v)]
    pre_pause_st = np.mean(before) if before else np.nan

    y = snd.values[0]
    hop, n = int(0.01 * rate), int(0.03 * rate)
    frames = [y[i:i + n] * np.hanning(n) for i in range(0, len(y) - n, hop)]
    spec = np.abs(np.fft.rfft(frames, axis=1)) ** 2
    freqs = np.fft.rfftfreq(n, 1 / rate)
    ft = np.arange(len(frames)) * hop / rate + n / 2 / rate
    v = np.interp(ft, times, voiced.astype(float)) > 0.5
    low = spec[v][:, (freqs >= 50) & (freqs < 1000)].sum()
    high = spec[v][:, (freqs >= 1000) & (freqs < 5000)].sum()
    return dict(speech_s=dur - sum(pauses), pauses=len(pauses), pause_ms=1000 * sum(pauses), f0=med,
                spread=12 * np.log2(hi / lo), pre_pause_st=pre_pause_st, end_st=end_st, end_slope=end_slope, decay_ms=decay,
                tilt_db=10 * np.log10(high / low))


# --- The text: sentences both actors read with the same wording -----------------------------
data = {s: {u["prompt"]: u for u in yaml.safe_load(open(data_dir / s / f"dfki-{s}-data.yaml")) if u.get("segments")}
        for s in ["poppy", "prudence"]}
prompts = [k for k in sorted(data["poppy"])
           if k.startswith("w") and k in data["prudence"] and data["poppy"][k]["text"] == data["prudence"][k]["text"]
           and 5 <= len(data["poppy"][k]["text"].split()) <= 16 and data["poppy"][k]["text"].endswith(".")
           and len(re.findall(r"[.!?]", data["poppy"][k]["text"])) == 1][:N_SENTENCES]
texts = [data["poppy"][k]["text"] for k in prompts]

rows = {}  # (source, sentence key) -> list of feature dicts
for s in ["poppy", "prudence"]:
    audio, rate = sf.read(flac_dir / f"dfki-{s}-data.flac")
    for k in prompts:
        u = data[s][k]
        x = audio[int(u["start"] * rate):int(u["end"] * rate)]
        rows[(f"{s} (recording)", k)] = [features(x, rate)]

voices = {
    "hfc_female": ("en_US-hfc_female-medium", None),
    "prudence (Piper)": ("en_GB-semaine-medium", 0),
    "poppy (Piper)": ("en_GB-semaine-medium", 3),
}
loaded = {}
for source, (name, sid) in voices.items():
    if name not in loaded:
        d = voices_dir / f"vits-piper-{name}"
        loaded[name] = PiperVoice.load(d / f"{name}.onnx", d / f"{name}.onnx.json")
    voice = loaded[name]
    items = list(zip(prompts, texts)) + [(f"check-in {i + 1}", t) for i, t in enumerate(CHECK_IN)]
    for k, text in items:
        for _ in range(CHECK_IN_DRAWS if k.startswith("check-in") else DRAWS):
            for j, chunk in enumerate(voice.synthesize(text, SynthesisConfig(speaker_id=sid, normalize_audio=False))):
                key = k if not k.startswith("check-in") else f"{k}.{j + 1}"
                rows.setdefault((source, key), []).append(features(chunk.audio_float_array, chunk.sample_rate))

COLS = ["speech_s", "pauses", "pause_ms", "f0", "spread", "pre_pause_st", "end_st", "end_slope", "decay_ms", "tilt_db"]
sources = ["poppy (recording)", "prudence (recording)"] + list(voices)


def mean_of(source, key, col):
    vals = [r[col] for r in rows.get((source, key), [])]
    return np.nanmean(vals) if vals else np.nan


for title, keys in [("48 DFKI sentences", prompts),
                    ("check-in sentences (Piper only)", sorted({k for s, k in rows if k.startswith("check-in")}))]:
    print(f"\n== {title}: median over sentences (each Piper sentence the mean of its draws)")
    print(f"{'source':22s}" + "".join(f"{c:>13s}" for c in COLS))
    for s in sources:
        if not any((s, k) in rows for k in keys):
            continue
        print(f"{s:22s}" + "".join(f"{np.nanmedian([mean_of(s, k, c) for k in keys]):13.2f}" for c in COLS)
              + f"   (decay measured in {sum(not np.isnan(mean_of(s, k, 'decay_ms')) for k in keys)} of {len(keys)})")
    print(f"\nSame sentence, voice against hfc_female: share of sentences where the voice is higher")
    print(f"{'source':22s}" + "".join(f"{c:>13s}" for c in COLS))
    for s in ["prudence (Piper)", "poppy (Piper)"]:
        cells = []
        for c in COLS:
            pairs = [(mean_of(s, k, c), mean_of("hfc_female", k, c)) for k in keys]
            pairs = [(a, b) for a, b in pairs if not (np.isnan(a) or np.isnan(b))]
            cells.append(f"{np.mean([a > b for a, b in pairs]):13.0%}")
        print(f"{s:22s}" + "".join(cells) + f"   ({len(keys)} sentences)")
