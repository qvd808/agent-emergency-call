# agent-emergency-call

A voice check-in agent for people who live alone. A resident dials an extension on a local
Asterisk PBX, or the server calls them. An AI agent runs a short spoken check-in. A real
emergency is transferred to a human dispatcher extension. Everything runs on one laptop over
local Wi-Fi, using synthetic data only.

**Status: two phones call each other through Asterisk.** There is no agent yet. The plan is tracked on the
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

`make` lists the targets. Asterisk runs in a container on Docker Desktop (WSL 2 backend); the
repo lives in the Ubuntu WSL distro.

### Placing a test call

1. `make asterisk-build` once, then `make asterisk-up`. The first run generates the two SIP
   passwords into `.env`. Every run reads the laptop's current Wi-Fi address and restarts
   Asterisk with it.
2. `make sip-accounts` prints what to type into each softphone.
3. Set up Linphone on two devices on the same Wi-Fi as the laptop. Choose the option for your
   own SIP account, not a linphone.org one.

   | | 1001, the resident | 2000, the dispatcher |
   |---|---|---|
   | Device used for the first call | Android phone | iPad |
   | Username, password | from `make sip-accounts` | from `make sip-accounts` |
   | Domain | the address `make sip-accounts` prints | the same address |
   | Transport | UDP | UDP |
   | Permissions | Microphone | Microphone, and Local Network (iOS asks on first use) |

   Turn off STUN, ICE and media encryption in both. Without the microphone permission, the call
   connects but that side sends no voice. Without Local Network, an iOS device can't reach the
   laptop at all.
4. `make asterisk-cli`, then `pjsip show contacts`: both phones show `Avail` once registered.
   Keep Linphone open on screen; a phone that stops answering Asterisk's checks shows
   `Unavail`, and calls to it fail until it registers again.
5. Dial 2000 from the phone, and 1001 from the iPad.
6. To reach the agent, run `make models` once, then start the agent in another terminal and
   dial 3100. `make agent` logs what you say, one line per utterance, with how long after you
   stopped its text was ready. `make agent-test-wav` first plays a synthetic 12 s test clip
   (beeps once a second, a steady tone, a rising sweep that fades out).
7. `make asterisk-down` afterwards. While Asterisk runs, anything that can reach the laptop can
   send it SIP.
