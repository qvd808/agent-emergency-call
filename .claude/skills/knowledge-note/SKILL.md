---
name: knowledge-note
description: Write or restructure a note in docs/knowledge/ so it teaches in dependency order, from the goal down to the mechanism. Use when writing a background note, when revising one that was hard to follow, or when a reader could not tell what the note was building toward or why a section appeared where it did.
---

# Knowledge notes

A note in `docs/knowledge/` is an **explanation**, not a decision record and not a tutorial.
Diátaxis defines explanation as documentation that is "understanding-oriented" and describes
it as "a discursive treatment of a subject, that permits *reflection*"
(<https://diataxis.fr/explanation/>). The reader arrives with the question *can you tell me
about this?* and leaves able to review a decision without reconstructing the theory first.

That is the goal these notes already have. This skill is about the one thing that decides
whether a note reaches it: **the order the material arrives in.**

## The failure this skill exists to prevent

A note fails when it is ordered by the *argument the writer was having* rather than by the
*dependency graph the reader is climbing*. It reads as a sequence of true, well-sourced
paragraphs that the reader cannot assemble, because each one presumes something the note has
not yet given them.

The three symptoms, in the order they usually appear:

1. **The goal is never stated.** The note opens on mechanism. The reader can follow every
   sentence and still not know what is being built or why this topic exists.
2. **A deviation is taught before its baseline.** The note says "this project does not do
   that" about a *that* it has not explained. Naming the standard approach is not teaching
   it.
3. **A decision is nested inside a concept.** The trade-off analysis grows subsections until
   the concept it was attached to has disappeared under it.

None of these are density problems. A note is not too long; it is out of order.

## Rule 1 — Build the ladder before writing a sentence

Do this first, every time, in scratch. It is not optional and it is not skippable for a
"short" note.

1. **List the terms.** Every technical term the finished note will use: `SIP`, `RTP`,
   `codec`, `frame`, `sample rate`, every one. Include terms borrowed from earlier notes.
2. **For each term, write where it is defined.** Either a section of this note, or a specific
   section of an earlier note (`note 01 §4`), or "assumed — the reader has this already."
   If a term has no defining location, the note has a hole; that hole is a section you have
   not written yet.
3. **Draw the edges.** Term A depends on term B if A cannot be explained without B.
4. **Order the sections so every edge points backwards.** A term's defining section comes
   before every section that uses it. No exceptions, including for a term that "everybody
   knows."

The ordered list is the note's skeleton. Write the prose into it. If while writing you need
a term the ladder places later, the ladder was wrong — fix the ladder, do not write a
forward reference.

## Rule 2 — §1 is always the goal, and it is fixed

Every note opens with a section that answers three questions, in this order, before any
mechanism appears:

- **What exists at the end.** The concrete artifact, in the project's own terms: a function
  with a signature, a type, a config that works, a checked property. Not "we will study
  AudioSocket" but "a coroutine `read_message(reader) -> Message` that turns the TCP byte
  stream into typed frames, so the echo server can be written against it."
- **Why now.** What is blocked or expensive without it, and what would change if the note
  were skipped. Say plainly if the answer is "nothing mathematical is missed."
- **What this note does not cover.** The boundary. Diátaxis warns against letting explanation
  "absorb other things," cautioning that "allowing them to creep in interferes with the
  explanation itself."

This section is what the reader holds in their head for the rest of the note. Without it,
every later section is a fact with nowhere to attach.

## Rule 3 — Baseline before deviation

Whenever the note describes a choice that departs from ordinary practice, **the ordinary
practice gets its own section, taught as if unknown, before the word "instead" or "does not"
appears.**

That section owes the reader:

- the pipeline or shape drawn out, with each stage's input and output named;
- the vocabulary the stage introduces, defined on first use;
- why anyone does it that way — the problem the standard approach solves.

Only then does the deviation section open. A deviation section may not introduce baseline
vocabulary. If you find yourself defining a term inside the paragraph that rejects it, the
baseline section is missing.

This is "Introduced Abstractions" — "Before any technical concept, establish the problem it
solves" — and "Decoupled Explanation" — "teaching a concept on its own before showing how it
works in a specific tool"
(<https://github.com/Xamfonos/technical-writing-best-practices>, `technical-writing-style-guide.md`).

## Rule 4 — Concept sections and decision sections do not nest

A topic that contains a choice splits into two consecutive sections at the same heading
level:

- **How it works.** The mechanism, taught neutrally. No project choice appears here.
- **What this project picked, and why.** The choice, with its trade-off.

The decision never becomes a subsection tree hanging off the concept. If the decision needs
five parts, it is five parts of the decision section, not `§3.1.4` under the concept.

## Rule 5 — Decision sections have one fixed order

Inside "what this project picked," the order is:

1. **The problem the choice is about** — restated in one or two sentences, in this note's
   terms.
2. **The options as they exist in the wild** — each described on its own terms and neutrally,
   with its source. The reader must be able to see why a competent person picked each one.
3. **The choice, stated plainly.**
4. **What it costs**, and the invariant or rule that pays the cost.
5. **What the choice does not buy** — the rationales that sound right and are wrong.
6. **What it actually buys.**

Costs and non-reasons before the options is the most common ordering mistake: the reader is
being asked to weigh a trade-off between alternatives they have not been shown. This ordering
is the "Earned Solutions" principle — "The problem must be established, deepened, and
clarified before presenting the fix" — applied to a comparison
(<https://github.com/Xamfonos/technical-writing-best-practices>).

## Rule 6 — Two heading levels, and every section declares its prerequisite

- **Depth cap: `##` and `###` only.** No `####`, and no `§3.1.1`-style manual numbering below
  the second level. Content that wants a fourth level is a section of its own, promoted.
- **Every `##` section opens with a one-line prerequisite.** Italic, immediately under the
  heading: *Needs: §2.1, and note 01 §4.* Or *Needs: nothing before it.*

The prerequisite line is what makes Rule 1's ladder visible to the reader and checkable by
you. If a section's prerequisite points forward, the note is out of order.

## Rule 7 — Concrete instance before general rule

Within a section, the smallest real example comes before the statement it illustrates: the
term before the grammar, the trace before the invariant, the failing input before the rule
that rejects it. Diátaxis puts the same weight on staying "focused on the concrete."

The general rule still gets stated, precisely and in full. This is about which one arrives
first.

## What this skill never does

- **It never shortens.** Density is the point of these notes. Reordering does not licence
  cutting a quotation, a worked example, a trace, or a caveat. If a section moves, it moves
  whole. A note getting *longer* because a missing baseline section had to be written is the
  expected outcome.
- **It never summarises in place of explaining.** A summary paragraph is not a substitute for
  the section it summarises.
- **It never relaxes `CLAUDE.md`.** Every factual claim still carries the URL that was
  actually fetched plus the work's title and identifier; a source that could not be fetched is
  reported as unfetched rather than cited from memory; anything worked out rather than read is
  marked as inferred, in those words. Notes explain concepts and do not decide the project's
  direction.
- **It never touches anything outside `docs/knowledge/`.** Code, config and the agent docs
  are other work; a note that needs one of them changed says so and links the ticket.

## Writing a new note

1. Build the ladder (Rule 1). Show it before writing prose if the note is large.
2. Write §1 from the fixed template (Rule 2).
3. Write sections in ladder order, each with its prerequisite line (Rule 6).
4. Keep the existing conventions of the directory: the italic preamble under the title that
   links the ticket the note was written for, a `## Sources` section at the end, a self-check
   section before it.
5. Run the checklist in `NOTE-SHAPE.md`.
6. Add the one-line row to `docs/knowledge/README.md`.

## Revising an existing note

The note's *content* is assumed good. The job is the order.

1. Build the ladder from the note as it stands: list its terms, and for each, find the line
   where the note first *uses* it and the line where it first *defines* it. Any term used
   before it is defined is a finding.
2. Report the findings as a list of moves — "§3.1 needs a baseline section before it",
   "§3.1.3 runs before §3.1.1 should" — before rewriting anything.
3. Apply the moves. Move whole sections. Write the missing baseline sections. Renumber.
4. Nothing is deleted. If a passage now sits in two places, merge it into the earlier one and
   leave a pointer, do not drop it.
5. Run the checklist in `NOTE-SHAPE.md`.
