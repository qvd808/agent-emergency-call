# Demo: one call, every feature

A script for a live demo of about two minutes. Everything runs on the laptop; two softphones
stand in for the resident and the dispatcher. All names and details are made up.

## Setup

| Role | Extension | Device |
|---|---|---|
| Resident | 2000 | a softphone on the laptop, with a headset |
| Dispatcher | 1001 | Linphone on a phone, on the same Wi-Fi |

- `.env`: `DISPATCHER_EXTENSION=1001`, so escalations ring the phone.
- **Use a headset** on the laptop. Through speakers, the agent's voice can come back into the
  microphone and pause the agent. If it does, set `BARGE_IN=off` in `.env` and restart the
  agent.
- Ollama running (`ollama serve`), with the model from `.env` pulled.

Before recording:

1. `make asterisk-up`.
2. `make asterisk-cli`, then `pjsip show contacts`. 2000 must show `Avail`, and 1001 too if the
   transfer should ring the phone. Linphone on Android stops answering after a while; open it
   in the foreground if 1001 shows `Unavail`.
3. Optional: `RECORD_CALLS=on` in `.env` saves each call as `calls/<call>.wav`, you on the left
   and the agent on the right.
4. `make agent` in a terminal you keep on screen. It prints every utterance, reply, barge-in and
   escalation as it happens.

## The call

Dial **3100** from the laptop softphone, then:

| You say | What to show |
|---|---|
| (listen) | The agent says it is an automated check-in assistant and asks how you are. |
| "I'm doing alright, thanks." | It thanks you and asks whether you've eaten. |
| Talk over it while it asks: "Sorry, wait, what was that?" | It stops mid-sentence within a moment (`paused`, then `barge-in confirmed` in the log) and answers what you said. |
| "Yes, I had some toast this morning." | It moves on to falls. |
| "No… (count two seconds in silence) …I haven't fallen." | It waits through the pause instead of answering "No". The log may show `gave the turn back`. Then it asks about pain. |
| "Actually, I slipped in the bathroom and I can't get up." | `ESCALATING … trigger Keyword` in under a second. It says it is connecting you to a person and to call 911 yourself if you are in danger. |
| (wait) | The agent transfers the live call (`transfer to 1001 sent: Redirect successful`). If the phone is registered, it rings; answer it as the dispatcher. Whether anyone answers is beyond the agent. |

Afterwards, open the newest file in `calls/`: the transcript, each turn's latency and status,
the barge-in, and the escalation with its trigger, evidence and timing.

## The eval

`make eval` runs 16 scripted residents through the same call path, with no phone, and prints
the gates and latency table (about 17 minutes, so play a recorded run). The latest report is in
`calls/eval/<run>/report.md`.
