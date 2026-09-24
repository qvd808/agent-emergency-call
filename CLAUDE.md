# CLAUDE.md

## Permission to edit

**You may create, edit, delete, rename and move any file in this repository, and run any
command that writes to the working tree, without asking.** This project is built for speed,
so the approval gate used in `proof-assistant` does not apply here. `.claude/settings.json`
backs this with permission rules so file writes do not prompt.

Three rules from that gate stay, because they protect work rather than slow it down:

- Re-read every file immediately before writing to it. I edit these files too, sometimes
  while you are working. Never write based on a copy you read earlier in the conversation.
- Never run `git checkout`, `git restore`, `git reset`, `git clean`, or `git stash` to
  discard working-tree changes. Uncommitted work in this repo may be mine.
- You make the commits: one small commit per issue with a clear message, and only once that
  issue's "done when" is met (`CONTEXT.md:106`, `CONTEXT.md:143`). No Claude co-author
  trailer; `.claude/settings.json` sets `attribution` to empty so none is added.

### Run it, do not hand it to me

Run commands yourself. Do not print a block of shell for me to paste.

A wrong path in a command you run costs you a tool call and you see the error immediately.
The same wrong path in a block I paste costs me a turn, and I find out by running it.
Putting the failure where it is cheapest is the whole point.

Exceptions, where handing me the command is still correct:

- Anything other people can see: creating a remote, pushing, opening a pull request,
  anything on a public repository, anything sent off this machine to a third party. Running
  the app itself, including its calls to the LLM API configured in `.env`, is not in this
  list.
- Anything destructive or hard to reverse.
- Anything that needs my hands: my Android phone, Linphone, the second softphone, placing a
  test call, an account only I can open. Stop and give me exact step-by-step instructions
  (`CONTEXT.md:106`).

### Issues on this repo are yours to drive

`qvd808/agent-emergency-call` is public, and its issues are the working surface for
`/wayfinder`. Create, label, assign, comment on and close them freely, without asking.
Every one of those actions is reversible except the text itself: anyone can read an issue,
and an edited or closed one may already have been seen or cached.

This is a standing exception to the outward-facing rule above. It covers issues on this
repository only. Pushing and pull requests stay mine.

A map is worked by opening and closing issues constantly. A skill that has to stop and ask
before each one is not usable.

Two things stay mine even here.

First, do not close a ticket that needs me by deciding it yourself. That is
`wayfinder:grilling` and `wayfinder:prototype` always, and `wayfinder:task` whenever the
work needs my hands — a test call from my phone, an account only I can open. Those resolve
through a real exchange, or through me actually doing the thing and saying so. Only
`wayfinder:research` is unconditionally yours to close.

Second, deleting issues is not covered. Close them instead.

A ticket that reads as resolved because you said so, rather than because it was, is the
failure this whole section exists to prevent. It does not look like giving up. It looks like
progress.

### The knowledge base

`docs/knowledge/` holds background notes: the concepts this project rests on (SIP and RTP,
codecs and sample rates, AudioSocket framing, VAD, turn-taking, escalation), written from
first principles so that I can review a decision, and explain it, without having to
reconstruct the theory first. Write them with the `knowledge-note` skill.

Rules that apply to every note there:

- Every factual claim carries a source: the URL that was actually fetched, plus the title
  and identifier of the work. The `## Claims` section below governs these files too, so a
  source that could not be fetched is reported as unfetched rather than cited from memory.
- Anything worked out rather than read is marked as inferred, in those words.
- Notes explain the concept. They do not decide the project's direction. A decision belongs
  on the issue tracker, in the ticket that resolves it.
- `docs/knowledge/README.md` is the index, one line per note, kept current as notes are
  added.

### Cite the line or say you inferred it

Any path, filename, flag or command you give me carries the `file:line` that states it.

When no source states it and you worked it out from context, mark it inferred in those
words. Inferring is allowed. Presenting an inference as if a file specified it is not.

When a document names another document, read the named one before acting on a procedure
that comes from it. A pointer you skipped is the usual cause of a confidently wrong path.

## Project

An "Are You OK?" voice check-in agent for seniors who live alone. A caller dials an
extension on a local Asterisk PBX, an AI agent runs a short spoken check-in, and a real
emergency is transferred to a human dispatcher extension.

`CONTEXT.md` is the brief and the source of truth: goal, architecture, stack, safety rules,
agent behaviour, metrics, repo layout, and the ordered issue list. Read it at the start of
every session. Its **Safety rules** section (`CONTEXT.md:42`) is non-negotiable and
outranks speed: no emergency-number extensions, no route to an outside network, escalation
fails safe, synthetic data only.

Read `CONTEXT.md` for more info.

You write the code. I own what ships (`CONTEXT.md:4`), so every change must be one I can
read and explain.

The domain glossary is `GLOSSARY.md` (created lazily by the `domain-modeling` skill), not
`CONTEXT.md`. Decisions worth recording go in `docs/adr/`.

## Claims

"Verified", "confirmed", "works" — usable only when pointing at command output or a file
that exists on disk. With no artifact, the correct word is "untested". A call that was not
actually placed did not work.

Every number carries its source: the command that produced it, the call log in `calls/`, or
the eval run. This matters most for latency: a p50 with no run behind it is a guess. A claim
that is expensive for me to check is a claim I cannot use.

Citations: fetch the source or do not cite it. Recalled titles, author lists, venues and
numbers drift. If a source could not be fetched, say so rather than filling the gap. This
covers protocol and config facts too: AudioSocket framing, PJSIP options and AMI actions
come from the Asterisk docs or source, fetched.

A specification saying something is unsupported is evidence about the specification, not
evidence about the implementation.

## Agent skills

### Issue tracker

Issues live as GitHub issues on `qvd808/agent-emergency-call`, driven by the `gh` CLI.
See `docs/agents/issue-tracker.md`.

### Skills

Project skills live in `.claude/skills/`: `wayfinder` (invoke with `/wayfinder`), `grilling`,
`domain-modeling`, `research`, `prototype`, `knowledge-note`, `caveman`. They were ported
from `proof-assistant`.

### Subagents

Every subagent spawn goes through `.claude/hooks/subagent-gate.sh`: it asks before each
spawn and denies one once 2 are already running. Check `/usage` before approving; an agent
killed mid-task loses its findings.
