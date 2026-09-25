"""PROTOTYPE, throwaway: can the local LLM mark up how its reply is said (pace, pitch, volume,
pause, holding a word), and what does that cost in output tokens and time?

    python agent/examples/PROTOTYPE_delivery_llm.py     # Ollama running, model from OLLAMA_MODEL

For each resident line, asks for (a) the reply as plain text, (b) the reply plus one mood word,
and (c) the reply as phrases with a delivery each, both with Ollama's structured output, and prints the phrases, the output
tokens and the generation time. Each request runs twice; the second (warm) one is reported.
"""
import json
import os
import urllib.request

URL = os.environ.get("OLLAMA_URL", "http://127.0.0.1:11434") + "/api/chat"
MODEL = os.environ.get("OLLAMA_MODEL", "qwen3:4b-instruct-2507-q4_K_M")

BASE = """You are an automated check-in assistant on a phone call with an older person who
lives alone. React to what they just said in a few warm words, then ask whether they have had
any falls lately. One or two short sentences. Plain everyday words. If they mention a problem,
say you're sorry to hear it, and ask one question about it instead. Write only words to be
spoken."""

DELIVERY = BASE + """

Your words are spoken by a speech synthesiser that you direct. Split what you say into short
phrases, at the commas and full stops, and give each phrase how it is said:
- pace: "slow" for sympathy or for something important, "normal" otherwise, "brisk" for light
  small talk.
- pitch: "low" for sympathy and concern, "high" for good news and cheerful words, "normal"
  otherwise.
- volume: "soft" for sympathy, "normal" otherwise.
- pause_after: "long" after bad news before you go on, "short" between thoughts, "none"
  inside a sentence.
- hold: true to draw out the last word, like a sincere "Oh noo" or "Oh, that's lovely", at
  most once per reply. Otherwise false.
A real person varies these all the time: sympathy is slower, lower and softer; good news is
brighter and quicker. Don't make every phrase the same."""

MOOD = BASE + """

Also give the mood your reply should be spoken in: "sympathetic" when they told you something
sad, painful or worrying; "cheerful" when they told you good news; "neutral" otherwise."""
MOOD_SCHEMA = {
    "type": "object",
    "properties": {"reply": {"type": "string"}, "mood": {"enum": ["sympathetic", "cheerful", "neutral"]}},
    "required": ["reply", "mood"],
}

PLAIN_SCHEMA = {"type": "object", "properties": {"reply": {"type": "string"}}, "required": ["reply"]}
PHRASE = {
    "type": "object",
    "properties": {
        "text": {"type": "string"},
        "pace": {"enum": ["slow", "normal", "brisk"]},
        "pitch": {"enum": ["low", "normal", "high"]},
        "volume": {"enum": ["soft", "normal"]},
        "pause_after": {"enum": ["none", "short", "long"]},
        "hold": {"type": "boolean"},
    },
    "required": ["text", "pace", "pitch", "volume", "pause_after", "hold"],
}
DELIVERY_SCHEMA = {
    "type": "object",
    "properties": {"phrases": {"type": "array", "items": PHRASE}},
    "required": ["phrases"],
}

RESIDENTS = [
    "I'm doing great, my granddaughter visited yesterday.",
    "Not so good. I slipped in the bathroom this morning.",
    "I'm alright I suppose. A bit lonely since my husband passed.",
    "Fine, fine. Same as always.",
    "My knee has been hurting a lot more this week.",
    "Oh wonderful, I just got back from a walk in the park.",
]


def ask(system, schema, said):
    body = {
        "model": MODEL,
        "messages": [{"role": "system", "content": system}, {"role": "user", "content": said}],
        "format": schema,
        "stream": False,
        "keep_alive": "1h",
        "options": {"temperature": 0.7},
    }
    request = urllib.request.Request(URL, json.dumps(body).encode(), {"Content-Type": "application/json"})
    with urllib.request.urlopen(request) as response:
        r = json.load(response)
    return json.loads(r["message"]["content"]), r["eval_count"], r["total_duration"] / 1e6


totals = {"plain": [0, 0.0], "mood": [0, 0.0], "delivery": [0, 0.0]}
for said in RESIDENTS:
    print(f"\nRESIDENT: {said}")
    for name, system, schema in [("plain", BASE, PLAIN_SCHEMA), ("mood", MOOD, MOOD_SCHEMA), ("delivery", DELIVERY, DELIVERY_SCHEMA)]:
        ask(system, schema, said)  # warm-up
        out, tokens, ms = ask(system, schema, said)
        totals[name][0] += tokens
        totals[name][1] += ms
        if name == "plain":
            print(f"  plain    {tokens:4d} tokens {ms:6.0f} ms  {out['reply']}")
        elif name == "mood":
            print(f"  mood     {tokens:4d} tokens {ms:6.0f} ms  [{out['mood']}] {out['reply']}")
        else:
            print(f"  delivery {tokens:4d} tokens {ms:6.0f} ms")
            for p in out["phrases"]:
                print(f"      {p['text']!r:45} {p['pace']:6} {p['pitch']:6} {p['volume']:6} "
                      f"pause={p['pause_after']:5} hold={p['hold']}")
n = len(RESIDENTS)
for name, (tokens, ms) in totals.items():
    print(f"mean over {n}: {name:8} {tokens / n:4.0f} tokens {ms / n:5.0f} ms")
