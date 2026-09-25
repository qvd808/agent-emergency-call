"""PROTOTYPE, throwaway: measures the pitch of PROTOTYPE_prosody_samples' per-line WAVs with
Praat, through parselmouth, so the reshaping is judged by a tracker other than its own.

    python -m venv .venv && .venv/bin/pip install praat-parselmouth numpy
    .venv/bin/python agent/examples/PROTOTYPE_prosody_measure.py <out dir>

Per variant, averaged over the lines: median pitch; the 10th-90th percentile span and the
standard deviation of the pitch in semitones (how far the voice rises and falls); the
fraction of voiced frames; and the harmonics-to-noise ratio, which drops if the reshaping
roughens the voice.
"""
import collections
import pathlib
import sys

import numpy as np
import parselmouth

rows = collections.defaultdict(list)
for wav in sorted(pathlib.Path(sys.argv[1]).glob("*--*.wav")):
    variant, line = wav.stem.split("--")
    sound = parselmouth.Sound(str(wav))
    f0 = sound.to_pitch_ac(time_step=0.01, pitch_floor=75, pitch_ceiling=500).selected_array["frequency"]
    voiced = f0[f0 > 0]
    st = 12 * np.log2(voiced / np.median(voiced))
    hnr = sound.to_harmonicity_cc(time_step=0.01, minimum_pitch=75).values[0]
    hnr = hnr[hnr > -200].mean()  # Praat marks unvoiced frames -200 dB
    span = np.percentile(st, 90) - np.percentile(st, 10)
    rows[variant].append((line, np.median(voiced), span, st.std(), len(voiced) / len(f0), hnr))

print(f"{'variant':30} {'median Hz':>9} {'span st':>8} {'sd st':>6} {'voiced':>6} {'HNR dB':>6}")
for variant, r in rows.items():
    median, span, sd, voiced, hnr = (np.mean([x[i] for x in r]) for i in range(1, 6))
    print(f"{variant:30} {median:9.0f} {span:8.2f} {sd:6.2f} {voiced:6.2f} {hnr:6.1f}")
