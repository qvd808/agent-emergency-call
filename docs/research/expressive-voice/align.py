"""Per-phoneme durations of the base line, from piper-tts's own alignment patch
(PiperVoice.load(include_alignments=True)), at the Steady tone's settings.

    venv/bin/python align.py <voice.onnx>

Each phoneme's time includes the pad id after it, as piper-tts groups them.
"""
import sys

import numpy as np
from piper import PiperVoice, SynthesisConfig

voice = PiperVoice.load(sys.argv[1], include_alignments=True)
rate = voice.config.sample_rate
for noise_w, runs in ((0.0, 1), (0.7, 30)):
    config = SynthesisConfig(length_scale=1.2, noise_scale=0.6, noise_w_scale=noise_w)
    table = []
    for _ in range(runs):
        chunk = next(iter(voice.synthesize("Oh no, I'm sorry to hear that.", config, include_alignments=True)))
        table.append([(a.phoneme, a.num_samples / rate * 1000) for a in chunk.phoneme_alignments])
    names = [p for p, _ in table[0]]
    ms = np.array([[m for _, m in row] for row in table])
    print(f"noise_w {noise_w}, {runs} run(s), mean ms per phoneme:")
    print("  " + "  ".join(f"{p!r}:{m:.0f}" for p, m in zip(names, ms.mean(0))))
    comma = names.index(",")
    after = ms[:, comma] + ms[:, comma + 1]
    between = ms[:, names.index(" ")]
    print(f"  ',' plus the space after it: mean {after.mean():.0f} ms (min {after.min():.0f}, max {after.max():.0f});"
          f" the space between 'Oh' and 'no': mean {between.mean():.0f} ms")
