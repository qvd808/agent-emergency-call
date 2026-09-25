"""What a Piper voice whose speakers are styles sounds like, before anyone trains one (issue #41).

Two published Piper voices already use speaker ids for emotions or personas:
- en_GB-semaine-medium: 4 speakers, each an actor playing a character with one expressive
  style (poppy outgoing, obadiah gloomy, prudence pragmatic, spike angry).
- de_DE-thorsten_emotional-medium: one man, 8 speaker ids that are emotions, fine-tuned from
  the neutral thorsten voice on 300 sentences per emotion. This is the shape issue #41 asks
  about, only in German.
hfc_female, the agent's voice today, is rendered alongside as the reference.

    python -m venv .venv && .venv/bin/pip install piper-tts onnxruntime praat-parselmouth numpy soundfile
    .venv/bin/python render_style_voices.py <voices dir> <out dir>

<voices dir> holds the sherpa-onnx copies, unpacked from
https://github.com/k2-fsa/sherpa-onnx/releases/download/tts-models/vits-piper-<voice>.tar.bz2

Each line is written for one of the six tone marks in ../tone-marks/tag_eval.py. Every clip is
synthesised with the voice's own default scales and no per-clip normalisation, then resampled
to 8 kHz, what a softphone hears. Each voice gets one gain, so its loudest clip peaks at -1
dBFS and its styles keep their loudness relative to each other. Prints, per clip, the
synthesis time on this machine's CPU, the audio length, the share of frames Praat finds
voiced, and the pitch median and spread of those frames (meaningless when little is voiced,
as in a whisper). Lines are invented (synthetic data).
"""
import pathlib
import sys
import time

import numpy as np
import parselmouth
import soundfile as sf
from parselmouth.praat import call

from piper import PiperVoice, SynthesisConfig

voices_dir, out_dir = pathlib.Path(sys.argv[1]), pathlib.Path(sys.argv[2])
out_dir.mkdir(parents=True, exist_ok=True)
LINE_RATE = 8000  # what a softphone hears (agent/src/audio.rs LINE_RATE_HZ)

EN = [
    ("sympathetic", "Oh no, I'm sorry to hear that. Are you hurt?"),
    ("glad", "That's wonderful news, I'm so happy for you!"),
    ("playful", "Ha, a smoke alarm for a morning alarm. That's one way to wake up!"),
    ("reassuring", "Don't worry, you're not alone. Someone will check on you tonight."),
    ("serious", "Please stay where you are. I'm getting a person on the line now."),
    ("neutral", "Thanks. Did you have breakfast today?"),
]
DE = [
    ("sympathetic", "Oh nein, das tut mir leid. Sind Sie verletzt?"),
    ("playful", "Ha, ein Rauchmelder als Wecker. So kann man auch aufwachen!"),
]
VOICES = [("en_US-hfc_female-medium", EN), ("en_GB-semaine-medium", EN), ("de_DE-thorsten_emotional-medium", DE)]


def pitch(x, rate):
    """Share of frames voiced, median pitch in Hz, and the 10th-90th percentile spread in
    semitones."""
    f0 = parselmouth.Sound(x, sampling_frequency=rate).to_pitch(pitch_floor=60, pitch_ceiling=500)
    f0 = f0.selected_array["frequency"]
    voiced = f0[f0 > 0]
    lo, mid, hi = np.percentile(voiced, [10, 50, 90])
    return len(voiced) / len(f0), mid, 12 * np.log2(hi / lo)


print(f"{'file':46s} {'synth ms':>8s} {'audio s':>7s} {'RTF':>5s} {'voiced':>6s} {'F0 Hz':>6s} {'spread st':>9s} {'peak dBFS':>9s}")
for name, lines in VOICES:
    d = voices_dir / f"vits-piper-{name}"
    voice = PiperVoice.load(d / f"{name}.onnx", d / f"{name}.onnx.json")
    speakers = voice.config.speaker_id_map or {"only": None}
    list(voice.synthesize("Warm up.", SynthesisConfig(speaker_id=next(iter(speakers.values())))))
    clips = []
    for mark, text in lines:
        for speaker, sid in speakers.items():
            config = SynthesisConfig(speaker_id=sid, normalize_audio=False)
            start = time.perf_counter()
            chunks = list(voice.synthesize(text, config))
            ms = (time.perf_counter() - start) * 1000
            rate = chunks[0].sample_rate
            x = np.concatenate([c.audio_float_array for c in chunks])
            line = call(parselmouth.Sound(x, sampling_frequency=rate), "Resample", LINE_RATE, 50).values[0]
            clips.append((f"{name.split('-')[1]}-{mark}-{speaker}.wav", ms, x, rate, line))
    gain = 10 ** (-1 / 20) / max(np.max(np.abs(c[4])) for c in clips)
    for file, ms, x, rate, line in clips:
        line = line * gain
        sf.write(out_dir / file, line, LINE_RATE, subtype="PCM_16")
        voiced, f0, spread = pitch(x, rate)
        peak = 20 * np.log10(np.max(np.abs(line)))
        secs = len(x) / rate
        print(f"{file:46s} {ms:8.0f} {secs:7.2f} {ms / 1000 / secs:5.2f} {voiced:6.0%} {f0:6.0f} {spread:9.1f} {peak:9.1f}")
