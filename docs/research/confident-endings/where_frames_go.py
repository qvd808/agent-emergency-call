"""Where semaine's timing differs from hfc_female's, by kind of phoneme.

For every sentence of borrow_timing.py's lines, the frames per phoneme id that hfc_female, and
semaine as Prudence and as Poppy, give the same phoneme ids (noise_w 0, the predictor's mean).
Each id is put in one class, and each id's following PAD goes with it:
- lead-in        BOS, the start of the sentence
- word end       the last sound of a word, just before a space or a punctuation mark
- pause marks    punctuation and EOS
- vowels         the other vowel symbols (IPA vowels and the length mark)
- consonants     the other consonants
- spaces/stress  word boundaries and stress marks
A frame is 256 samples at 22 050 Hz, 11.6 ms.

    .venv/bin/python where_frames_go.py <voices dir>
"""
import sys

from borrow_timing import LINES, SEMAINE, open_voices, own_frames, run

VOWELS = set("aeiouæɑɒɔəɛɜɪʊʌɐɚɝɵʉyøœɤɯɨʏː")
PUNCT = set(",.;:!?")
CLASSES = ["lead-in", "vowels", "consonants", "word end", "spaces/stress", "pause marks"]
MS = 256 / 22050 * 1000


def classes(symbols):
    """The class of each phoneme id in a sentence, given as symbols; a PAD takes the class of
    the id before it."""
    out, cls = [], None
    for k, sym in enumerate(symbols):
        if sym == "_":
            pass
        elif sym == "^":
            cls = "lead-in"
        elif sym in PUNCT or sym == "$":
            cls = "pause marks"
        elif sym in {" ", "ˈ", "ˌ"}:
            cls = "spaces/stress"
        else:
            nxt = next((s for s in symbols[k + 1:] if s not in {"_", "ˈ", "ˌ"}), "$")
            if nxt == " " or nxt in PUNCT or nxt == "$":
                cls = "word end"
            else:
                cls = "vowels" if sym in VOWELS else "consonants"
        out.append(cls)
    return out


def main():
    hfc, semaine, s_sess, emb, h_sess = open_voices(sys.argv[1])
    names = {i[0]: p for p, i in hfc.config.phoneme_id_map.items()}
    totals = {}
    for text in LINES.values():
        for phonemes in hfc.phonemize(text):
            ids = hfc.phonemes_to_ids(phonemes)
            got = {"hfc_female": own_frames(h_sess, ids)[0, 0]}
            for who in ["prudence", "poppy"]:
                got[who] = run(s_sess, ids, g=emb[[SEMAINE[who]]])[1][0, 0]
            for k, cls in enumerate(classes([names[i] for i in ids])):
                for who, f in got.items():
                    totals.setdefault(cls, {}).setdefault(who, 0.0)
                    totals[cls][who] += f[k]
    print(f"{'class':14s} {'hfc_female':>10s} {'prudence':>10s} {'poppy':>10s}   (ms, all sentences of the four lines)")
    for cls in CLASSES:
        t = totals[cls]
        print(f"{cls:14s} " + " ".join(f"{t[w] * MS:10.0f}" for w in ["hfc_female", "prudence", "poppy"]))


if __name__ == "__main__":
    main()
