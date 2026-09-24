# agent-emergency-call

A voice check-in agent for people who live alone. A resident dials an extension on a local
Asterisk PBX, or the server calls them. An AI agent runs a short spoken check-in. A real
emergency is transferred to a human dispatcher extension. Everything runs on one laptop over
local Wi-Fi, using synthetic data only.

**Status: skeleton.** Nothing below has placed a call yet. The plan is tracked on the
[map issue](https://github.com/qvd808/agent-emergency-call/issues/1).

## Safety rules

- No extension for 911 or any real emergency number, and no dialplan pattern routes to an
  outside network.
- The system never contacts police or emergency services. Notices to the dispatcher are mocks.
- The agent says it is an automated check-in assistant and gives no medical advice.
- Synthetic data only: no real names, recordings or health information.

## Layout

```
asterisk/            Dockerfile (Asterisk 22.11.0 from source) and config/, mounted at /etc/asterisk
agent/               the live-call agent: one Rust process for the whole call
turn/                turn detection (VAD, end of turn, barge-in), shared with the prototype
eval/                the eval: scripted residents through a fake AudioSocket client
calls/               one JSON log per call (gitignored)
docker-compose.yml   Asterisk only; the agent and Ollama run natively in Ubuntu
.env.example         settings; copy to .env
```

## Running

`make` lists the targets. Setup steps and how to place a test call come with the first call.
