"""PROTOTYPE, throwaway: tallies the maintainer's picks from the listening set of issue #43.

Reads listening-set/picks.txt (one pick per line, "<tone> <n>: <letter> (<timing>)", or
"None fits" / "Can't tell"), checks each letter against listening-set/key.json, and takes each
take's speech rate from output/listening_set.txt, the run that rendered the set. Prints:
- per tone, how many lines went to each timing;
- line 1 of the four tones heard before (../timing-and-tone.md), against the earlier pick;
- how often Poppy's timing was picked with its wait before speaking, and without it;
- per tone, where the picked timing's speech rate ranks among the three timings (1 = fastest).
  Poppy's two takes share one speech rate, so they count as one timing here.

    python PROTOTYPE_tally_picks.py
"""
import collections
import json
import pathlib
import re

HERE = pathlib.Path(__file__).resolve().parent
EARLIER = {"sympathetic": "prudence", "glad": "poppy", "reassuring": "poppy", "serious": "own"}
COLUMNS = ["own", "prudence", "poppy", "poppy-own-lead-in", "None fits", "Can't tell"]

key = json.loads((HERE / "listening-set/key.json").read_text())
rate = {}
for line in (HERE / "output/listening_set.txt").read_text().splitlines():
    m = re.match(r"^(\w+)\s+(\d+)\s+[ABCD]\s+(\S+)\s+\d+\s+\d+\s+\d+\s+\d+\s+([\d.]+)\s+[\d.]+$", line)
    if m:
        rate[(m[1], int(m[2]), m[3])] = float(m[4])
picks = {}
for line in (HERE / "listening-set/picks.txt").read_text().splitlines():
    m = re.match(r"^(\w+) (\d+): (?:([ABCD]) \((\S+)\)|(None fits|Can't tell))$", line)
    if not m:
        continue
    tone, n, letter, timing, other = m[1], int(m[2]), m[3], m[4], m[5]
    if letter:
        assert key[tone][n - 1]["takes"][letter] == timing, f"{tone} {n}: {letter} is not {timing}"
    picks[(tone, n)] = timing or other
assert len(picks) == sum(len(lines) for lines in key.values()), "a line has no pick"

print("== Lines per timing (5 lines a tone)")
print(f"{'tone':12s} " + " ".join(f"{c:>17s}" for c in COLUMNS))
for tone in key:
    count = collections.Counter(p for (t, _), p in picks.items() if t == tone)
    print(f"{tone:12s} " + " ".join(f"{count[c]:17d}" for c in COLUMNS))
total = collections.Counter(picks.values())
print(f"{'all':12s} " + " ".join(f"{total[c]:17d}" for c in COLUMNS))

print("\n== Line 1, heard again blind")
for tone, before in EARLIER.items():
    now = picks[(tone, 1)]
    print(f"{tone:12s} earlier {before:9s} now {now:18s} {'held' if now == before else 'changed'}")

print(f"\n== Poppy's timing: picked with its wait {total['poppy']} times, without it {total['poppy-own-lead-in']}")

print("\n== Speech rate of the picked timing, ranked among the three (1 = fastest)")
for tone in key:
    ranks = []
    for (t, n), p in sorted(picks.items()):
        if t != tone or p in ("None fits", "Can't tell"):
            continue
        base = "poppy" if p.startswith("poppy") else p
        order = sorted(["own", "prudence", "poppy"], key=lambda k: -rate[(t, n, k)])
        ranks.append(order.index(base) + 1)
    print(f"{tone:12s} " + " ".join(map(str, ranks)))
