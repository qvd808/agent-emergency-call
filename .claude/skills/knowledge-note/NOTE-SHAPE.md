# Note shape

The skeleton, the checklist, and one worked reordering.

## The skeleton

```markdown
# <Topic, as a noun phrase>

*Background for [<ticket name>](<issue url>). <One or two sentences: what this note is for,
and any honest warning about it — that it is config trivia, that it is mechanical, that it is
the hardest topic so far.>*

---

## 1. The goal

*Needs: nothing before it.*

<What exists at the end: the concrete artifact, in the project's own terms — a signature, a
type, a checked property.>

<Why now: what is blocked or expensive without it. What is not missed if the topic is
skipped.>

<What this note does not cover: the boundary.>

---

## 2. <The baseline concept — how this is normally done>

*Needs: §1.*

<The standard shape, drawn out. Each stage's input and output named. Every term this note
will later use, defined here on first use. Why anyone does it this way.>

---

## 3. <Where the difficulty actually is>

*Needs: §2.*

<The specific thing that makes the general problem hard here. Concrete instance first, then
the rule.>

---

## 4. <What this project picked, and why>

*Needs: §2, §3.*

<In this fixed order:>
<  1. the problem the choice is about, restated in this note's terms>
<  2. the options in the wild, each neutral, each sourced>
<  3. the choice, stated plainly>
<  4. what it costs, and the invariant that pays for it>
<  5. what it does not buy>
<  6. what it actually buys>

---

## 5. <The mechanism, in full>

*Needs: §4.*

---

## 6. <Worked by hand>

*Needs: §5.*

<Traces. The smallest ones first.>

---

## 7. The shape in this repo

*Needs: §5.*

<The Python module, config file or dialplan the concept lands in.>

---

## 8. Checking yourself

*Needs: everything above.*

---

## Sources

<Every source, with the URL that was actually fetched, the title, and the identifier.>
```

Sections 2 through 7 are named for their content, not for these placeholders. Their *count*
varies; their *order* is the ladder. Sections 1, 8 and Sources are fixed.

## The checklist

Run all of it. Every failed line is a move, not a note.

**Goal**

- [ ] Reading §1 alone tells you what artifact exists at the end, stated as a signature, a
      type, or a checked property.
- [ ] §1 says what is blocked without this topic.
- [ ] §1 says what the note does not cover.
- [ ] No mechanism appears in §1.

**Dependency order**

- [ ] Every technical term's first *use* comes after its first *definition* — in this note,
      or at a named section of an earlier note.
- [ ] Every `##` section carries a prerequisite line.
- [ ] No prerequisite line points forward.
- [ ] No section assumes a term marked "assumed" that the reader of the previous notes would
      not actually have.

**Baseline before deviation**

- [ ] Every "this project does not do that" has a preceding section that taught *that*.
- [ ] No deviation section introduces baseline vocabulary.
- [ ] The baseline section says why anyone does it the standard way, not only what it is.

**Decision sections**

- [ ] Concept and decision are sibling sections, not nested.
- [ ] Options in the wild appear before costs and before non-reasons.
- [ ] Each option is described so a competent person's reason for picking it is visible.
- [ ] The choice's costs are followed by the invariant or rule that pays for them.

**Shape**

- [ ] No heading deeper than `###`.
- [ ] No manual numbering below the second level.
- [ ] Within each section, the concrete instance precedes the general rule.

**Preservation**

- [ ] Nothing was deleted. Quotations, traces, caveats and worked examples are all still
      present, whole.
- [ ] No section was replaced by a summary of itself.

**`CLAUDE.md`**

- [ ] Every factual claim carries the URL that was actually fetched, plus title and
      identifier.
- [ ] Any source that could not be fetched is reported as unfetched, not cited from memory.
- [ ] Anything worked out rather than read is marked as inferred, in those words.
- [ ] Every path, filename, flag and command carries its `file:line`.
- [ ] The note explains; it does not decide the project's direction.
- [ ] `docs/knowledge/README.md` has the note's row.

## Worked example — what went wrong in note 02

This example comes from the sibling `proof-assistant` repo, where this skill was written. It
is kept because the failure it shows does not depend on the subject.

`../proof-assistant/docs/knowledge/02-parsing-lambda-terms.md` is the case this skill was
written from. Its content is sound and its sources are real. Its order is not.

**As written.** §1 *Why the parser comes this early* → §2 *Where the difficulty actually is*
→ §3 *No lexer, and what replaces it*, whose §3.1 opens:

> The obvious architecture is two passes: a lexer turning `&str` into a `Vec` of tokens, then
> a parser turning that into a `NamedTerm`.

That sentence is the first appearance of *lexer*, *token*, *two passes* and *pass* in the
note, and it is also the sentence that rejects them. Three paragraphs later the note says
"**This project does not do that.**" The baseline was named, quoted at, and discarded, but
never taught. A reader without a compilers background has been handed a decision about a
thing they do not have.

The subsections then run `§3.1` decision → `§3.1.1` the other model → `§3.1.2` costs →
`§3.1.3` non-reasons → `§3.1.4` what it buys. That is four levels deep, and it is the order
of an argument being settled, not of a concept being learned: the costs of the choice arrive
before the reader has seen the alternatives they are being traded against.

**The ladder it should have had.**

| Rung | Section | Needs |
|---|---|---|
| 1 | The goal — `parse(&str) -> NamedTerm`, so every later note can write test terms as text | — |
| 2 | How text becomes a tree, normally — characters, the lexer and what a token is, the parser and what an AST is, why the split exists at all | 1 |
| 3 | Where the difficulty actually is — the notation conventions, the two rules | 2 |
| 4 | What this project picked: no lexer — problem, then Nystrom / Rocq / Lean as three real options, then the choice, then trivia as its cost and the cursor invariant that pays it, then the TCB non-reason, then reversibility | 2, 3 |
| 5 | The lexical shapes, whitespace, comments, identifiers | 4 |
| 6 | The grammar | 3, 5 |
| 7 | Two parses, done by hand | 6 |
| 8 | Errors, the Rust shape, the self-check | 7 |

Rung 2 does not exist in the note today. It is not a summary of rung 4 — it is the section
whose absence makes rung 4 unreadable, and writing it makes the note longer.

Everything else on that list is already written, sourced and correct. The revision is
one new section and a reordering.
