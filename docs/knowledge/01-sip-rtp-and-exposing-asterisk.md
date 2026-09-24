# SIP, RTP, and what opening them to the Wi-Fi exposes

*Background for [Should SIP and RTP accept packets only from the local subnet?](https://github.com/qvd808/agent-emergency-call/issues/30).
It teaches the phone protocols from zero, then follows a packet from the Wi-Fi into Asterisk
and on through the laptop, so that the ticket's firewall and hardening choices can be
reviewed. §2 to §6 are protocol basics. The security argument starts at §7. Several facts
about this laptop come from tests run on 2026-09-24, and they are described where they are
used.*

---

## 1. The goal

*Needs: nothing before it.*

**What exists at the end.** Not code: a property of this laptop that can be checked, in
three parts, plus a statement of what stays open on purpose.

- Asterisk's SIP port (UDP 5060) and RTP ports (UDP 10000–10099) are open to the Wi-Fi only
  while Asterisk runs, and it runs only when started by hand, for a test at home.
- Asterisk serves a request only from a configured phone that proves a long random
  password, and that phone can only reach the project's internal extensions.
- If Asterisk itself were broken into, the attacker would not be root inside its container,
  and using AMI would take a long random secret that grants no system commands.

What stays open, on purpose: whatever the host firewall admits to Docker Desktop reaches the
published ports, and a broken-into container can reach services in Ubuntu (§7, §8). Both
belong to how the host itself is set up, which is outside this effort (§9).

For each part, the reader should be able to say which layer provides it, and what would be
possible without it.

**Why now.** The first real call,
[Phone 1001 calls softphone 2000 with audio both ways](https://github.com/qvd808/agent-emergency-call/issues/14),
is the first time this project opens ports to the Wi-Fi. Until then nothing here listens on
the network. Without this note, the ticket's choices are a list of settings with no reason
attached.

**What this note does not cover.** How SIP and RTP cross NAT (`external_media_address`,
one-way audio), which belongs to the first-call task. Codecs and sample rates, beyond naming
them. AudioSocket, the link between Asterisk and the agent. Encryption, beyond saying what
it would be. The rest of the laptop's firewall, beyond the parts a broken-into Asterisk
could use.

---

## 2. A phone call is two conversations

*Needs: §1.*

Take the first call this project places: the phone app registered as extension 1001 rings
the dispatcher's softphone, extension 2000. Two separate things have to happen.

1. **Setting up the call.** 1001 says "I want to talk to 2000". Something finds where 2000
   is, 2000 rings and picks up, and at the end one side hangs up. This is **signalling**: a
   handful of messages per call.
2. **Carrying the voices.** Once 2000 picks up, both sides send sound to each other,
   continuously, until someone hangs up. This is **media**: a steady stream of small packets,
   dozens per second, for the whole call.

The two have opposite needs (inferred from the protocols in §3 and §5; no single source
says it this way):

- A signalling message has to arrive, intact. Half a second late, nobody notices.
- A media packet that arrives late is useless. 20 ms of sound that turns up after it should
  have been played can't be played, so skipping it is better than waiting for it.

That is why telephony over IP uses one protocol for each: **SIP** for signalling (§3) and
**RTP** for media (§5). **SDP** (§4) connects them: it is how SIP tells each side where to
send the media.

Between the phones sits a **PBX** (private branch exchange): the phone exchange of a
building or a company. Phones sign in to it, and it connects calls between them. In this
project the PBX is **Asterisk** (§6). The phones are **softphones**, apps that act as a phone:
Linphone on Android as 1001, and a softphone on the laptop as 2000. An **extension** is a
number dialled on the PBX: 1001, 2000, or 3100 for the agent.

---

## 3. SIP: setting up the call

*Needs: §2.*

A simplified exchange. It follows the shape of RFC 3261's examples, but it was not captured
from a real call: the addresses are made up and most headers are dropped.

```
phone 1001 → Asterisk    REGISTER sip:asterisk      "1001 can be reached at 192.0.2.10:5060"
Asterisk   → phone 1001  SIP/2.0 200 OK
   ... later ...
phone 1001 → Asterisk    INVITE sip:2000@asterisk   "I want to call 2000", plus a body saying
                                                    where to send my audio (§4)
Asterisk   → phone 2000  INVITE ...                 "1001 is calling you"
phone 2000 → Asterisk    SIP/2.0 180 Ringing
phone 2000 → Asterisk    SIP/2.0 200 OK             (picked up)
   ... the voices flow (§5) ...
phone 1001 → Asterisk    BYE                        (hung up)
```

What the trace shows, stated in general:

- **SIP is requests and responses, written as text, like HTTP.** RFC 3261 §7: "SIP is a
  text-based protocol and uses the UTF-8 charset", and "much of SIP's message and header
  field syntax is identical to HTTP/1.1" [S1]. A request names a **method** (`REGISTER`,
  `INVITE`, `BYE`). A response carries a three-digit **status code** (`200 OK`,
  `180 Ringing`, `401 Unauthorized`), the same idea as in HTTP.
- **Addresses look like email addresses.** RFC 3261 §4: a SIP URI "has a similar form to
  an email address, typically containing a username and a host name", for example
  `sip:bob@biloxi.com` [S1]. So the comparison with SMTP holds for addressing: in both, you
  name a user at a server, and the server finds them. The comparison stops there. SMTP hands
  over a message to be delivered later, while SIP sets up something live (inferred).
- **`REGISTER` tells the PBX where a phone is.** A phone's address changes (new Wi-Fi, new
  DHCP lease), so the phone keeps telling the PBX "1001 is at this address now". The PBX
  uses that to deliver an `INVITE` to it later.
- **`INVITE` starts a call.** Its body describes the media the caller wants (§4).
- **Where SIP travels.** SIP's default port "is 5060 for UDP, TCP and SCTP, 5061 for TLS"
  (RFC 3261 §18.1.1) [S1]. This project uses UDP 5060 (`docker-compose.yml:9`).
- **SIP does not carry the voice.** RFC 3261 §2: "SIP is not a vertically integrated
  communications system". It is used with "the Real-time Transport Protocol (RTP) ... for
  transporting real-time data" and "the Session Description Protocol (SDP) ... for
  describing multimedia sessions" [S1].

That last point corrects a common picture, in which SIP "connects a port to a phone". SIP
opens and connects nothing for the audio. Inside its messages it carries a *description* of
where each side wants its audio sent, and the audio then travels on its own.

---

## 4. SDP: where the audio should go

*Needs: §3.*

The body of an `INVITE` is an SDP description. RFC 3264 §10.1 gives this offer, cut here
to its audio lines [S3]:

```
c=IN IP4 host.anywhere.com
m=audio 49170 RTP/AVP 0
a=rtpmap:0 PCMU/8000
```

- `c=` is the address to send media to.
- `m=audio 49170 RTP/AVP 0` means: an audio stream, sent to port 49170, carried by RTP,
  in format 0.
- `a=rtpmap:0 PCMU/8000` names format 0: the **codec** PCMU, at 8000 samples per second. A
  codec is a way of turning sound into bytes and back.

The other side replies with its own description, in its `200 OK`. From then on each side
sends RTP to the address and port the other one named. RFC 3264 calls this the
**offer/answer** model [S3].

Two consequences matter later:

- **Audio ports are chosen per call** from a range the PBX is given. For this project the
  range is 10000–10099 (`docker-compose.yml:10-11`, which notes it must match `rtpstart` and
  `rtpend` in `asterisk/config/rtp.conf`). That is why a whole range of UDP ports has to be
  open, not one port.
- **The address in the SDP is the one the sender believes it has.** If something along the
  way rewrites addresses, the SDP and the packet's real source disagree. §7 shows this
  happens on this laptop.

---

## 5. RTP: carrying the voice

*Needs: §2, §4.*

A concrete stream first. At 20 ms of audio per packet, a call sends 50 packets a second in
each direction: 1000 ms divided by 20 ms (arithmetic, inferred). RFC 3551 §4.2: "For
packetized audio, the default packetization interval SHOULD have a duration of 20 ms or one
frame, whichever is longer" [S5]. Each packet is a small header followed by the sound.

RFC 3550 §1 describes RTP as providing "end-to-end delivery services for data with
real-time characteristics, such as interactive audio and video. Those services include
payload type identification, sequence numbering, timestamping and delivery monitoring.
Applications typically run RTP on top of UDP" [S4]. Two header fields matter here
(RFC 3550 §5.1) [S4]:

- the **sequence number**, which "increments by one for each RTP data packet sent, and may
  be used by the receiver to detect packet loss and to restore packet sequence";
- the **timestamp**, which "reflects the sampling instant of the first octet in the RTP data
  packet".

And what RTP leaves out: "RTP itself does not provide any mechanism to ensure timely
delivery ... It does not guarantee delivery or prevent out-of-order delivery"
(RFC 3550 §1) [S4]. A lost packet is not sent again. That is the trade-off from §2: a resent
voice packet would arrive too late to play.

**How RTP differs from HTTP.** In HTTP a client asks and a server answers, over TCP, which
resends anything lost. RTP has no requests and no answers. Once the call is up, each side
streams packets to the other over UDP, and nothing is resent (inferred comparison, from the
RTP text above). In a call, the protocol that looks like HTTP is SIP (§3), not RTP.

**How RTP relates to WebRTC.** WebRTC, the calling stack built into browsers, runs on RTP;
RTP does not run on it. The combination rules WebRTC uses are called a **profile**. That's
a term of RTP's own, unrelated to the firewall profiles in §7. RFC 8834 §4.2: "WebRTC
endpoints MUST NOT send packets using the basic RTP/AVP profile ... implementations MUST use
SRTP and Secure RTCP (SRTCP)" [S6]. **SRTP** is RTP plus what plain RTP lacks: "media
encryption, integrity protection, replay protection, and a limited form of source
authentication" (same section) [S6]. The `RTP/AVP` in §4's SDP is the plain profile. It has
no encryption and no check on who sent a packet.

---

## 6. Asterisk, PJSIP and AMI

*Needs: §2, §3, §5.*

**Asterisk.** Its README: "Asterisk is an Open Source PBX and telephony toolkit. It is, in a
sense, middleware between Internet and telephony channels on the bottom, and Internet and
telephony applications at the top" [S7]. So Asterisk is not a phone app. It is the exchange
that phone apps sign in to. In this project it runs in a Docker container
(`asterisk/Dockerfile`) and does three things:
- accepts SIP from the phones;
- relays RTP between them;
- runs the **dialplan**.

**Dialplan and context.** The dialplan is Asterisk's list of what happens when someone
dials an extension. For 2000 it rings the dispatcher's phone. For 3100 it connects the call
to the agent. The dialplan is split into named groups called **contexts**. Each phone is
given one context, which the endpoint option `context` describes as "Dialplan context for
inbound sessions" (`pjsip.conf.sample:662`) [S9]. The same file warns: "It's easy to
accidentally provide access to internal or outbound dialing extensions which could cost you
severely. The "context=" line in endpoint configuration determines which dialplan context
inbound calls will enter into" (`pjsip.conf.sample:56-59`) [S9]. So a call from a phone
reaches only the extensions in that phone's context, and a number outside it can't be
dialled from that phone (inferred from those two passages).

**PJSIP.** pjproject is an open-source SIP library, and Asterisk's SIP support is built on
it. The Asterisk tarball bundles pjproject by default (`asterisk/Dockerfile:26`, citing the
tarball's `configure.ac:473-478`). The Asterisk modules around it are configured in
`pjsip.conf`, which describes each phone with a few kinds of section [S9]:

- `type=endpoint`: the phone as Asterisk sees it, meaning its context, its codecs, and
  which `auth` it must pass.
- `type=auth`: the username and password it must prove.
- `type=aor`: its **address of record**. "Endpoints use one or more AOR sections to store
  their contact details" (`pjsip.conf.sample:264`), and a `REGISTER` is matched to an AOR
  by "the username in the "To" header" (`pjsip.conf.sample:104-105`) [S9]. So the AOR is
  where the phone's current address is kept, and its `REGISTER` updates it (inferred from
  those two lines).
- `type=identify`: optional. Recognises a phone by the IP address its packets come from.

So "PJSIP is the library that handles SIP for Asterisk" is right, with one refinement. In
Asterisk's configuration, "PJSIP" means Asterisk's modules and their `pjsip.conf`, not the
library itself.

**AMI.** The Asterisk Manager Interface is a TCP port, 5038 by default
(`manager.conf.sample:25`) [S10]. A program logs in to it with a username and a secret, then
sends commands. This project's agent will use AMI to place outbound check-in calls
(`Originate`) and to transfer a call to the dispatcher (`Redirect`). That was decided in
[How does the agent place, transfer and track calls through Asterisk's manager interface?](https://github.com/qvd808/agent-emergency-call/issues/5).
Each AMI user is granted **classes** of permission. "Write authorization permits you to send
commands and get back responses". The classes include (`manager.conf.sample:295-331`) [S10]:
- `system`: "ability to run system management commands, such as Shutdown, Restart, and
  Reload";
- `command`: "Permission to run CLI commands";
- `call`;
- `originate`: "Permission to originate new calls".

---

## 7. How a packet from the Wi-Fi reaches Asterisk

*Needs: §3, §5, §6.*

This is the path on this laptop, which runs:
- Windows;
- Docker Desktop, running containers in its own WSL 2 VM;
- the agent, natively in the Ubuntu WSL distro. See
  [How do Asterisk under Docker Desktop and the agent in Ubuntu reach each other?](https://github.com/qvd808/agent-emergency-call/issues/27).

### Windows Firewall

Windows Firewall blocks inbound traffic by default. A packet gets through only if an
**allow rule** matches it. Microsoft's rule precedence [S14]:

1. "Explicitly defined allow rules take precedence over the default block setting."
2. "Explicit block rules take precedence over any conflicting allow rules."
3. "More specific rules take precedence over less specific rules, except if there are
   explicit block rules".

A rule can be limited by program, protocol, local port, remote address, and **profile**.
Windows classes every network it joins as **Public** or **Private** (or Domain, for a company
network), and a rule applies only on the profiles it names. For apps used only on a home
network, Microsoft recommends limiting the remote address "to specify Local Subnet only",
and enabling the rules only "on the private profile" [S14].

Many rules were never written on purpose. When a program first listens and has no rule,
Windows shows a prompt. An admin who allows it gets allow rules. "If they respond No or
cancel the prompt, block rules are created", and "once the rules are added, they must be
deleted to generate the prompt again" [S14].

### Docker Desktop's port publishing

A container's port is not directly on the Wi-Fi. Docker's docs [S12]:
1. "Docker Desktop's backend process listens on the specified host port".
2. It "forwards the connection into the Linux VM where the container is running".
3. There the connection is "routed to the container's internal IP address and port".

On Windows the backend process is `com.docker.backend.exe` [S12]. A published port by
default "listens on all network interfaces (`0.0.0.0`), but you can restrict it to a
specific address, such as `127.0.0.1`" [S12]. And "Host firewalls can permit or deny inbound
connections by filtering on `com.docker.backend`" [S12].

So to Windows Firewall, a packet for Asterisk is a packet for `com.docker.backend.exe`. A
rule that allows that program allows every port that any container publishes.

### What Asterisk sees of the sender

A test on this laptop on 2026-09-24:
- A `python:3.11` container listened on a published UDP port (`-p 15070:15070/udp`) and
  printed the source address of each datagram it received.
- Three datagrams were sent to it: from Windows to the laptop's Wi-Fi address, from Windows
  to `127.0.0.1`, and from Ubuntu to `127.0.0.1`.
- All three arrived from `172.17.0.1`, the Docker bridge's gateway. `docker inspect` showed
  the container at `172.17.0.2` with gateway `172.17.0.1`.

So inside the container, every packet seems to come from the same address, whoever sent
it. The test didn't include a packet from the phone. That packet takes the same path through
`com.docker.backend.exe`, so it is expected to look the same (inferred, untested).

### What a container can reach

Two tests on this laptop:

- **Ubuntu's loopback is reachable from containers.** In the ticket linked at the top of
  this section, a container dialled `host.docker.internal`, and a listener bound to
  `127.0.0.1` in Ubuntu accepted the connection. It arrived from `127.0.0.1`.
- **So are Ubuntu's services.** On 2026-09-24, a `busybox` container ran
  `nc -z host.docker.internal <port>` against ports that services in Ubuntu were listening
  on. They were open. Ports with nothing listening were closed.

So any container can open a connection to any service listening in Ubuntu, including one
bound only to `127.0.0.1`.

### What Ubuntu can reach

WSL's **interop** lets Linux run Windows programs. Tested on 2026-09-24:
`powershell.exe`, started from Ubuntu,
reported the Windows user who runs WSL, and `IsInRole(Administrator)` returned `False`. So a
Windows program started from Ubuntu runs as that user, **not elevated**. It can read and
write whatever the user can. Anything that needs admin rights needs a UAC prompt first.

---

## 8. Where the difficulty is

*Needs: §6, §7.*

Each layer in §6 and §7 is simple on its own. The difficulty is what they do together,
against two kinds of attacker: a stranger who can send packets to the laptop, and someone
who has already broken into Asterisk.

### Guessing a password

First, the check Asterisk runs:
1. **It identifies the phone.** PJSIP matches a request to a phone by the username in the
   request, or by its source IP address (`identify_by`) [S9].
2. **It refuses what it can't identify.** "By default anonymous inbound calls via PJSIP are
   not allowed" (`pjsip.conf.sample:41`) [S9].
3. **It demands proof.** RFC 3261 §22.2, where a **UAS** (user agent server) is the side
   receiving a request, here Asterisk: "If no credentials (in the Authorization header field)
   are provided in the request, the UAS can challenge the originator to provide credentials
   by rejecting the request with a 401 (Unauthorized) status code" [S1].

The challenge includes a **nonce**, a value the server picks fresh for each challenge. The
phone retries with a response computed from "the username, the password, the given nonce
value, the HTTP method, and the requested URI. In this way, the password is never sent in
the clear". That wording is RFC 2617 §3.1.2 [S2]; SIP follows HTTP's digest scheme
(RFC 3261 §22.4) [S1].

```
phone    → Asterisk   REGISTER                                  (no credentials)
Asterisk → phone      401 Unauthorized, nonce="…"               (the challenge)
phone    → Asterisk   REGISTER, Authorization: response=hash(user, password, nonce, …)
Asterisk → phone      200 OK                                    (same hash computed)
```

An attacker has two ways at this. One is trying passwords online, one `REGISTER` each. The
other is overhearing an exchange, then testing "any overheard nonce/response pairs against a
list of common words" (RFC 2617 §4.7) [S2]. Asterisk's guide: "Secure passwords limit your
risk to brute force attacks" (`README-SERIOUSLY.bestpractices.md:13`) [S8]. A long random
password is on no list of common words, and there are too many possibilities to try them one
guess at a time (inferred).

### What a correct password would buy

- **Registration hijacking.** RFC 3261 §26.1.1: registration lets a user agent identify
  itself "to a registrar as a device at which a user (designated by an address of record)
  is located" [S1]. The **registrar** is the server that accepts `REGISTER`, here Asterisk.
  Whoever registers as 1001 gets 1001's calls (inferred from the same section).
- **Toll fraud.** This is the usual reason to attack a PBX: "If your PBX system is
  accessible via the internet, then your system will be vulnerable to expensive
  international calls" (`README-SERIOUSLY.bestpractices.md:215-217`) [S8]. It needs a route
  from the dialplan to the outside. This project has none: its contexts hold only internal
  extensions, a safety rule on the
  [map](https://github.com/qvd808/agent-emergency-call/issues/1). A stolen password reaches
  1001, 2000 and 3100, and nothing else.

### A bug before the password check

Asterisk has to read a SIP message before it can find the username in it. So a bug in how
Asterisk reads SIP is reachable by anyone who can send a packet to UDP 5060, with no
password at all (inferred). A password can't protect against that. What can is limiting
who is able to send packets, and limiting what a broken-into Asterisk holds.

### The chain from Asterisk to Windows

Putting §7 together, on this setup:

1. A stranger on the Wi-Fi sends packets through `com.docker.backend.exe` into the Asterisk
   container.
2. If they break into Asterisk, they run as root inside the container. `asterisk/Dockerfile`
   has no `USER` line, and its `CMD` (`asterisk/Dockerfile:40`) passes no `-U`. That is
   inferred from the file, not checked in a running container. Asterisk's guide: "not
   running Asterisk as root, can prevent serious problems"
   (`README-SERIOUSLY.bestpractices.md:358-359`) [S8].
3. From the container they can reach any service listening in Ubuntu (§7). One that
   accepts a guessable password, such as an SSH server with password logins, would give
   them a shell in Ubuntu.
4. From Ubuntu, interop runs Windows programs as the Windows user, unelevated (§7). That
   reaches the user's files, browser profiles and saved logins.

The chain can be cut at any link.

### Blind spots from the address rewrite

§7 showed every packet arriving from `172.17.0.1`. Three Asterisk features rely on the real
source address:

- **`type=identify`** matches a phone by its IP address [S9]. Here every sender has the same
  one.
- **An address list in Asterisk** (an **ACL**, access control list) would allow or deny
  every sender at once.
- **strictrtp** "will drop RTP packets that do not come from the recognized source of the
  RTP stream". It learns that source at the start of a call (`rtp.conf.sample:24-31`)
  [S11]. With one address for every sender, it can't tell the phone from anyone else while it
  learns (inferred).

Only Windows Firewall sees the real sender. It is the only place where "local subnet only"
can work.

### AMI is close to a shell

Asterisk's guide: "you should treat the Manager class authorization 'originate' the same as
the class authorization 'system'" (`README-SERIOUSLY.bestpractices.md:357-358`) [S8]. Its
first example is an `Originate` action with `Application: System` and
`Data: echo hello world!`, which "will attempt to execute an Asterisk application, System".
Its second is a dialplan that runs whatever is in `${EXEC_COMMAND}`. That can be abused
because "the Manager action Originate allows for channel variables to be set by the account
initiating the new call" (lines 321-355) [S8]. This project needs `originate` (§6), so its
AMI secret protects something close to running commands inside the container.

### Open for how long

Asterisk's ports are open only while its container runs, since the backend is what listens
on a published port (§7). Docker's restart policies decide when a container runs without
being asked [S13]:

- `no`: "Don't automatically restart the container. (Default)"
- `unless-stopped`: "Similar to `always`, except that when the container is stopped
  (manually or otherwise), it isn't restarted even after Docker daemon restarts."

The repo sets `unless-stopped` (`docker-compose.yml:16`). So if Asterisk is still running
when Docker Desktop quits, it comes back the next time Docker Desktop starts, on whatever
network the laptop is on by then.

---

## 9. What this project picked, and why

*Needs: §7, §8.*

### The problem

Opening SIP and RTP to the phone opens a way into the laptop. Some layers have to stop a
stranger on the Wi-Fi from getting in, and some have to stop someone who got into Asterisk
from going further. The question is which.

### The options in the wild

- **Leave the program-wide rule and rely on SIP passwords.** Clicking allow on Windows'
  prompt creates exactly such a rule [S14]. At home, the router already keeps the internet
  out (inferred). This setup takes no effort, and PJSIP does refuse requests it can't
  identify or authenticate (§8).
- **Narrow the rule by port, protocol, remote address and profile.** This is Microsoft's own
  recommendation for apps used on a home network: remote address "Local Subnet only", with
  the rules enabled on the private profile [S14]. A packet gets through if any allow rule
  matches it (§7), so narrowing works only once the broad rule stops matching. That means
  editing it, disabling it, or deleting it. Deleting brings the prompt back [S14].
- **Add block rules.** "Explicit block rules take precedence over any conflicting allow
  rules" [S14]. So blocking everything outside the home subnet would beat Docker's rule
  without touching it. The catch: "everything except one subnet" has to be written as a list
  of ranges.
- **Run Asterisk where it sees real addresses.** That means Docker Engine inside Ubuntu with
  host networking, the fallback already named in
  [Phone 1001 calls softphone 2000 with audio both ways](https://github.com/qvd808/agent-emergency-call/issues/14).
  Asterisk's own address checks and strictrtp would work again (inferred).
- **Encrypt.** SIP over TLS (port 5061, RFC 3261 §18.1.1) [S1], and SRTP, which WebRTC
  requires [S6]. This guards against eavesdropping and against forged audio on the Wi-Fi.
- **Shrink what a broken-into Asterisk holds.** Run it non-root inside the container [S8],
  load fewer modules, give the AMI user fewer classes [S10], and leave nothing guessable to
  log in to next.

### The choice

These choices were made on
[Should SIP and RTP accept packets only from the local subnet?](https://github.com/qvd808/agent-emergency-call/issues/30).
Its resolution comment holds the final list.

- **Where Asterisk is served:** on the home Wi-Fi only, and only while testing. A demo for
  other people would run on a separate server.
- **Firewall and host:** unchanged by this repo. What the host firewall admits, and which
  services the host runs, are host setup, which is ruled outside this effort.
- **When it runs:** only when started by hand (restart policy `no`).
- **Secrets:** each SIP phone and the AMI user get a long random secret, generated into the
  gitignored `.env`. A lost secret is regenerated, never remembered.
- **Inside Asterisk:**
  - the AMI user gets `call` and `originate` only, never `system` or `command`;
  - Asterisk loads an explicit list of modules;
  - it runs as a non-root user in its container, with its capabilities dropped.
- **Encryption:** none. Calls stay unencrypted.

### What it costs, and what pays for it

- **While Asterisk runs, anything the host firewall admits can send to it** (§7). What
  pays for it: Asterisk runs only during a test at home, where the
  router keeps the internet out (inferred), and every account behind it has a long random
  secret.
- **A broken-into Asterisk could still reach services in Ubuntu** (§8). What pays for it:
  getting there first takes a bug in Asterisk, and even then the attacker isn't root inside
  the container.
- **Asterisk has to be started by hand** after each reboot or restart of Docker Desktop.
- **An explicit module list can miss a module**, and the first sign is a failed call. The
  error names what is missing.
- **A lost secret can't be recovered, only replaced:** regenerate it, restart Asterisk, and
  enter it again in the softphones.
- **Calls on the Wi-Fi can be overheard** by anyone on the same network, because plain RTP
  has no encryption (§5). That is acceptable only because every call uses synthetic data.

### What it does not buy

- **SIP passwords do not protect the laptop.** They protect the phone system from misuse
  (§8). A bug in how Asterisk reads SIP is reachable without one.
- **Restart policy `no` is not a firewall.** It limits *when* Asterisk listens, not *who* can
  reach it while it does.
- **A "local subnet only" firewall rule would not mean "trusted" either.** On a café's
  Wi-Fi, the local subnet is strangers.
- **An address check inside Asterisk would add no layer here.** Every sender looks like
  `172.17.0.1` (§7).
- **Binding a service in Ubuntu to `127.0.0.1` would not keep containers out.** Container
  connections arrive on Ubuntu's loopback (§7).

### What it actually buys

- **From the Wi-Fi:** SIP and RTP exist only while Asterisk runs, which is only during a
  test at home.
- **The chain in §8** loses its worst first step: root inside the container.
- **The AMI secret**, which guards something close to running commands, is long, random,
  and kept in one gitignored file, and the AMI user has no `system` or `command` class.
- **Nothing for the laptop's owner to do by hand.** Every piece is a line in this repo,
  written in the ticket that writes that file.

---

## 10. The shape in this repo

*Needs: §9.*

Where each piece lives today, or will live once the ticket's work lands.

| Piece | Where |
|---|---|
| SIP port published | `docker-compose.yml:9` |
| RTP range published; must match `rtp.conf` | `docker-compose.yml:10-11` |
| Restart policy (today `unless-stopped`) | `docker-compose.yml:16` |
| Asterisk's user (today root, inferred) | `asterisk/Dockerfile:40` |
| AMI address and credentials (empty today) | `.env.example:12-16` |
| `.env` kept out of git | `.gitignore:2` |
| `pjsip.conf`, `manager.conf`, `modules.conf`, `rtp.conf`, `extensions.conf` | `asterisk/config/`, to be written in [Phone 1001 calls softphone 2000 with audio both ways](https://github.com/qvd808/agent-emergency-call/issues/14) (file set inferred) |
| Secrets generated into `.env` | a `make` target, to be written in [Phone 1001 calls softphone 2000 with audio both ways](https://github.com/qvd808/agent-emergency-call/issues/14) |
| The firewall | unchanged by this repo: host setup (§9) |

---

## 11. Checking yourself

*Needs: everything above.*

1. Which protocol carries "I want to call 2000", and which carries the sound? Why doesn't
   one protocol do both?
   *SIP (§3) and RTP (§5). Signalling has to arrive; media has to arrive on time or not at
   all (§2).*
2. Your phone sends an `INVITE`. How does Asterisk know which UDP port to send your audio
   to?
   *From the SDP body of the `INVITE`: the `c=` address, and the port in `m=audio` (§4).*
3. Is RTP "like HTTP"? How does WebRTC relate to it?
   *No. RTP has no requests or responses, and resends nothing (§5). WebRTC carries its media
   over SRTP, which is RTP plus encryption and integrity checks (§5).*
4. Someone on the home Wi-Fi doesn't know the password. Can they still harm the laptop
   through SIP?
   *Only through a bug in how Asterisk reads SIP, since that comes before the password check
   (§8). Running Asterisk only during tests limits when they can try. Running it non-root
   limits what they get.*
5. Why would an IP allow-list in `pjsip.conf` do nothing on this laptop? Where does "local
   subnet only" have to live instead?
   *Inside the container every sender appears as `172.17.0.1` (§7). Windows Firewall is the
   only layer that sees the real source (§8).*
6. You add a narrow allow rule for UDP 5060, while a broad allow rule for the same program
   is still enabled. What changes?
   *Nothing. A packet gets through if any allow rule matches, and the broad rule still
   matches (§7).*
7. Why does the AMI secret matter as much as a login password?
   *Asterisk's own guide says to treat `originate` like `system`: an Originate can end in
   running a command (§8).*
8. Asterisk has been broken into. On this setup, what are the attacker's next two steps,
   and what would cut each one?
   *First, reach a service listening in Ubuntu through `host.docker.internal`. One that
   accepts a guessable password would give a shell; turning off password logins, or the
   service, cuts that. Second, from Ubuntu, run Windows programs as the user through interop;
   turning interop off would cut that. Both are host setup, outside this effort (§9). What
   this effort does instead is make the step before them harder: Asterisk runs only during
   tests, and not as root.*

---

## Sources

Every source below was fetched on 2026-09-24 at the URL given, and the quoted lines were
checked against the fetched text.

- **[S1]** RFC 3261, *SIP: Session Initiation Protocol*, June 2002.
  <https://www.rfc-editor.org/rfc/rfc3261.txt>. Sections used: §2, §4, §7, §18.1.1, §22.2,
  §22.4, §26.1.1.
- **[S2]** RFC 2617, *HTTP Authentication: Basic and Digest Access Authentication*,
  June 1999. <https://www.rfc-editor.org/rfc/rfc2617.txt>. Sections used: §3.1.2, §4.7.
- **[S3]** RFC 3264, *An Offer/Answer Model with the Session Description Protocol (SDP)*,
  June 2002. <https://www.rfc-editor.org/rfc/rfc3264.txt>. Section used: §10.1.
- **[S4]** RFC 3550, *RTP: A Transport Protocol for Real-Time Applications*, July 2003.
  <https://www.rfc-editor.org/rfc/rfc3550.txt>. Sections used: §1, §5.1.
- **[S5]** RFC 3551, *RTP Profile for Audio and Video Conferences with Minimal Control*,
  July 2003. <https://www.rfc-editor.org/rfc/rfc3551.txt>. Section used: §4.2.
- **[S6]** RFC 8834, *Media Transport and Use of RTP in WebRTC*, January 2021.
  <https://www.rfc-editor.org/rfc/rfc8834.txt>. Section used: §4.2.
- **[S7]** Asterisk, `README.md`, branch 22.
  <https://raw.githubusercontent.com/asterisk/asterisk/22/README.md>. Lines 18-20.
- **[S8]** Asterisk, `README-SERIOUSLY.bestpractices.md` (*Best Practices*), branch 22.
  <https://raw.githubusercontent.com/asterisk/asterisk/22/README-SERIOUSLY.bestpractices.md>.
  Lines 13, 215-217, 321-359.
- **[S9]** Asterisk, `configs/samples/pjsip.conf.sample`, branch 22.
  <https://raw.githubusercontent.com/asterisk/asterisk/22/configs/samples/pjsip.conf.sample>.
  Lines 39-44 (anonymous calls), 53-59 (dialplan contexts), 104-105 (AOR matching), 264
  (AOR), 662 (`context`), 693-700 (`identify_by`).
- **[S10]** Asterisk, `configs/samples/manager.conf.sample`, branch 22.
  <https://raw.githubusercontent.com/asterisk/asterisk/22/configs/samples/manager.conf.sample>.
  Lines 25, 295-331.
- **[S11]** Asterisk, `configs/samples/rtp.conf.sample`, branch 22.
  <https://raw.githubusercontent.com/asterisk/asterisk/22/configs/samples/rtp.conf.sample>.
  Lines 24-39.
- **[S12]** Docker docs, *Networking on Docker Desktop*.
  <https://raw.githubusercontent.com/docker/docs/main/content/manuals/desktop/features/networking/_index.md>.
  Sections "Backend components and responsibilities" and "How exposed ports work".
- **[S13]** Docker docs, *Start containers automatically*.
  <https://raw.githubusercontent.com/docker/docs/main/content/manuals/engine/containers/start-containers-automatically.md>.
  Restart policy table, lines 31-34.
- **[S14]** Microsoft Learn, *Windows Firewall Rules* (ms.date 2025-06-06).
  <https://learn.microsoft.com/en-us/windows/security/operating-system-security/network-security/windows-firewall/rules>.
  Sections "Rule precedence for inbound and outbound rules", "Applications rules", "Firewall
  rules recommendations". This page was fetched through a summarising tool, not as raw text,
  so no line numbers are given. The quotes are the ones the tool returned verbatim.

**Tests on this laptop, 2026-09-24** (each described where it is used):

- the source address a container sees on a published UDP port (§7);
- `nc -z host.docker.internal <port>` from a `busybox` container (§7);
- `powershell.exe` from Ubuntu: `WindowsIdentity.GetCurrent()` and
  `IsInRole(Administrator)` (§7).

**Earlier evidence:** the resolution comment on
[How do Asterisk under Docker Desktop and the agent in Ubuntu reach each other?](https://github.com/qvd808/agent-emergency-call/issues/27)
(the loopback listener reached from a container, §7).
