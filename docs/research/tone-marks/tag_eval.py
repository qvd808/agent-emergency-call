"""Does the local LLM pick the right tone mark for its own reply, and what does the mark cost?

    python docs/research/tone-marks/tag_eval.py     # Ollama running, model from OLLAMA_MODEL

The same words can need opposite tones: "Oh no" to a fall is sorry for the resident, "Oh no" to
burnt toast is a joke shared with them. Only the LLM has the context to tell them apart, so the
mark is asked for in the same structured answer as the reply, before it (as `status` comes
before `reply` in agent/src/llm.rs). Each resident line has the tones a person would accept.

Prints, per line, the tone picked on each run and the reply, then the share of runs whose tone
was accepted, and output tokens and generation time with and without the mark. Each request
runs once to warm up; the next RUNS are counted. Resident lines are invented (synthetic data).
"""
import json
import os
import urllib.request

URL = os.environ.get("OLLAMA_URL", "http://127.0.0.1:11434") + "/api/chat"
MODEL = os.environ.get("OLLAMA_MODEL", "qwen3:4b-instruct-2507-q4_K_M")
RUNS = int(os.environ.get("RUNS", "3"))

BASE = """You are an automated check-in assistant on a phone call with an older person who
lives alone. React to what they just said in a few warm words, then ask one short question.
One or two short sentences. Plain everyday words. Write only words to be spoken."""

TONES = {
    "sympathetic": "they told you something sad, painful or worrying that happened to them. You are sorry for them, not sad yourself.",
    "glad": "they told you good news. You are happy for them.",
    "playful": "they are joking or laughing about something harmless. You share the joke lightly.",
    "reassuring": "they are worried or nervous about what might happen. You calm them.",
    "serious": "their safety is at stake right now. You are clear and steady.",
    "neutral": "anything else: routine answers and small talk.",
}
MARKED = BASE + "\n\nAlso give the tone your reply is spoken in, one of:\n" + "\n".join(
    f"- {name}: {when}" for name, when in TONES.items()
)
MARKED_SCHEMA = {
    "type": "object",
    "properties": {"tone": {"enum": list(TONES)}, "reply": {"type": "string"}},
    "required": ["tone", "reply"],
}
PLAIN_SCHEMA = {"type": "object", "properties": {"reply": {"type": "string"}}, "required": ["reply"]}

# (what the resident said, tones a person would accept)
CASES = [
    ("Not so good. I slipped in the bathroom this morning.", {"sympathetic"}),
    ("I'm alright I suppose. A bit lonely since my husband passed.", {"sympathetic"}),
    ("My knee has been hurting a lot more this week.", {"sympathetic", "reassuring"}),
    ("My sister is in hospital, they don't know what's wrong yet.", {"sympathetic", "reassuring"}),
    ("I'm doing great, my granddaughter visited yesterday.", {"glad"}),
    ("The doctor said my scan came back clear!", {"glad"}),
    ("Ha, I burnt the toast again. The smoke alarm is my morning alarm now.", {"playful"}),
    ("My cat stole my slipper and hid it under the bed, the little thief!", {"playful"}),
    ("I dropped my phone in the soup, can you believe it? Ha!", {"playful"}),
    ("I lost to my neighbour at cards again. She cheats, I swear, haha.", {"playful"}),
    ("I'm a bit nervous about the storm tonight, what if the power goes out?", {"reassuring", "sympathetic"}),
    ("Fine, fine. Same as always.", {"neutral", "glad"}),
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


right = runs = 0
cost = {"plain": [0, 0.0], "marked": [0, 0.0]}
for said, accepted in CASES:
    print(f"\nRESIDENT: {said}   (accepted: {', '.join(sorted(accepted))})")
    for name, system, schema in [("plain", BASE, PLAIN_SCHEMA), ("marked", MARKED, MARKED_SCHEMA)]:
        ask(system, schema, said)  # warm-up
        for _ in range(RUNS):
            out, tokens, ms = ask(system, schema, said)
            cost[name][0] += tokens
            cost[name][1] += ms
            if name == "marked":
                ok = out["tone"] in accepted
                right += ok
                runs += 1
                print(f"  {'ok ' if ok else 'BAD'} {out['tone']:11} {tokens:3d} tok {ms:5.0f} ms  {out['reply']}")
print(f"\n{MODEL}: tone accepted in {right} of {runs} runs")
n = len(CASES) * RUNS
for name, (tokens, ms) in cost.items():
    print(f"mean over {n}: {name:6} {tokens / n:4.1f} output tokens {ms / n:5.0f} ms")
