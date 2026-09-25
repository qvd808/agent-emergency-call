//! What escalates a check-in (issue #11), and the record an escalation leaves in the call log.
//!
//! Five triggers, none able to suppress another: the keyword rule here, the LLM's `emergency`
//! status, silence, the resident asking for a person (also the keyword rule, its own
//! category), and an agent fault. The conversation loop fires the other three.

use std::sync::LazyLock;

use serde::Serialize;

/// Words allowed between two words of a phrase: "I can't seem to get up" still matches
/// `cant get up`.
const MAX_GAP: usize = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Trigger {
    Keyword,
    Llm,
    Silence,
    AskedForPerson,
    AgentFault,
}

/// One per escalated call (issue #11).
#[derive(Debug, Clone, Serialize)]
pub struct Escalation {
    pub trigger: Trigger,
    /// The matched phrase and the transcript it matched in; the LLM's reason; the count of
    /// unanswered prompts; or the error.
    pub evidence: String,
    /// The resident's turn it fired on, counting from 1; 0 before the resident first spoke.
    pub turn: usize,
    /// From the end of the triggering speech (or the last timeout) to the escalation starting:
    /// playback stopped and the script queued.
    pub detected_ms: u64,
    /// From the end of the triggering speech (or the last timeout) to the transfer command,
    /// the script's playing time included (issue #11).
    pub latency_ms: Option<u64>,
    /// What came of the transfer command.
    pub transfer: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hit {
    pub trigger: Trigger,
    /// The phrase, in normalised form.
    pub phrase: String,
}

struct Phrases {
    /// Anywhere in the text, with gaps: (phrase's words, trigger).
    anywhere: Vec<(Vec<String>, Trigger)>,
    /// The whole utterance.
    whole: Vec<Vec<String>>,
}

static PHRASES: LazyLock<Phrases> =
    LazyLock::new(|| parse(include_str!("../data/escalation_phrases.txt")));

fn parse(file: &str) -> Phrases {
    let mut phrases = Phrases { anywhere: Vec::new(), whole: Vec::new() };
    let mut section = "";
    for line in file.lines().map(str::trim) {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            section = match name {
                "emergency" | "emergency, whole" | "asked for a person" => name,
                other => panic!("escalation_phrases.txt: unknown section [{other}]"),
            };
            continue;
        }
        let words = normalise(line);
        match section {
            "emergency" => phrases.anywhere.push((words, Trigger::Keyword)),
            "asked for a person" => phrases.anywhere.push((words, Trigger::AskedForPerson)),
            "emergency, whole" => phrases.whole.push(words),
            _ => panic!("escalation_phrases.txt: {line:?} comes before any section"),
        }
    }
    phrases
}

/// Checks one utterance, and the resident's whole turn so far, which catches a phrase whisper
/// split across two utterances. An emergency phrase wins over asking for a person.
pub fn check(utterance: &str, turn: &str) -> Option<Hit> {
    let words = normalise(utterance);
    if is_whole_help(&words) {
        return Some(Hit { trigger: Trigger::Keyword, phrase: words.join(" ") });
    }
    let words = normalise(turn);
    let mut hits = PHRASES.anywhere.iter().filter(|(phrase, _)| contains(&words, phrase));
    let first = hits.clone().find(|(_, t)| *t == Trigger::Keyword).or_else(|| hits.next())?;
    Some(Hit { trigger: first.1, phrase: first.0.join(" ") })
}

/// The utterance is nothing but whole-utterance phrases, one or more: "help", and also
/// "help, help!" or "help me, please help". Repeats are allowed because a frightened resident
/// says it twice (inferred; not in issue #11's list).
fn is_whole_help(words: &[String]) -> bool {
    if words.is_empty() {
        return false;
    }
    PHRASES.whole.iter().any(|phrase| {
        words.len() >= phrase.len()
            && words[..phrase.len()] == phrase[..]
            && (words.len() == phrase.len() || is_whole_help(&words[phrase.len()..]))
    })
}

/// `phrase`'s words appear in `words` in order, with at most [`MAX_GAP`] other words between
/// each pair.
fn contains(words: &[String], phrase: &[String]) -> bool {
    fn rest(words: &[String], phrase: &[String], at: usize) -> bool {
        let Some((first, others)) = phrase.split_first() else { return true };
        (at..words.len().min(at + MAX_GAP + 1))
            .any(|i| words[i] == *first && rest(words, others, i + 1))
    }
    let Some((first, others)) = phrase.split_first() else { return false };
    (0..words.len()).any(|i| words[i] == *first && rest(words, others, i + 1))
}

/// Lower case, apostrophes dropped ("can't" → "cant"), every other mark a space, and the
/// spelling variants folded into one form: cannot / can not → cant, i am → im, there is →
/// theres, will not → wont, 9 1 1 → 911.
pub fn normalise(text: &str) -> Vec<String> {
    let text: String = text
        .to_lowercase()
        .chars()
        .filter(|c| !matches!(c, '\'' | '’'))
        .map(|c| if c.is_alphanumeric() { c } else { ' ' })
        .collect();
    let words: Vec<&str> = text.split_whitespace().collect();
    let mut out = Vec::with_capacity(words.len());
    let mut i = 0;
    while i < words.len() {
        let (folded, used) = match &words[i..] {
            ["can", "not", ..] => ("cant", 2),
            ["cannot", ..] => ("cant", 1),
            ["i", "am", ..] => ("im", 2),
            ["there", "is", ..] => ("theres", 2),
            ["will", "not", ..] => ("wont", 2),
            ["9", "1", "1", ..] => ("911", 3),
            [word, ..] => (*word, 1),
            [] => unreachable!(),
        };
        out.push(folded.to_string());
        i += used;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hit(text: &str) -> Option<(Trigger, String)> {
        check(text, text).map(|h| (h.trigger, h.phrase))
    }

    fn keyword(phrase: &str) -> Option<(Trigger, String)> {
        Some((Trigger::Keyword, phrase.to_string()))
    }

    #[test]
    fn the_phrase_file_parses() {
        assert!(PHRASES.anywhere.len() > 40);
        assert_eq!(PHRASES.whole.len(), 3);
    }

    #[test]
    fn spellings_fold_into_one_form() {
        assert_eq!(normalise("I can't"), ["i", "cant"]);
        assert_eq!(normalise("I cannot, I can not"), ["i", "cant", "i", "cant"]);
        assert_eq!(normalise("I’m / I am"), ["im", "im"]);
        assert_eq!(normalise("There is a fire!"), ["theres", "a", "fire"]);
        assert_eq!(normalise("Call 9-1-1."), ["call", "911"]);
    }

    #[test]
    fn the_resolution_s_traps() {
        // Negation is ignored, so this escalates even though it starts with "no".
        assert_eq!(hit("No! I can't get up."), keyword("cant get up"));
        assert_eq!(hit("Can you help me with the shopping?"), None);
        assert_eq!(hit("Are you a real person?"), None);
    }

    #[test]
    fn the_done_when_sentence_escalates() {
        assert_eq!(hit("I fell and I can't get up."), keyword("cant get up"));
    }

    #[test]
    fn up_to_two_words_may_sit_inside_a_phrase() {
        assert_eq!(hit("I can't seem to get up"), keyword("cant get up"));
        assert_eq!(hit("I can't seem to ever get up"), None);
        assert_eq!(hit("I cannot get up"), keyword("cant get up"));
        assert_eq!(hit("I am on the floor"), keyword("im on the floor"));
    }

    #[test]
    fn help_counts_only_as_the_whole_utterance() {
        assert_eq!(hit("Help!"), keyword("help"));
        assert_eq!(hit("Help, help!"), keyword("help help"));
        assert_eq!(hit("Help me, please help."), keyword("help me please help"));
        assert_eq!(hit("I need help."), None);
        assert_eq!(hit("Help yourself to some tea."), None);
    }

    #[test]
    fn asking_for_a_person_is_its_own_trigger() {
        let hit = hit("Can I talk to a person, please?");
        assert_eq!(hit, Some((Trigger::AskedForPerson, "talk to a person".into())));
    }

    #[test]
    fn an_emergency_phrase_wins_over_asking_for_a_person() {
        assert_eq!(hit("Put me through, I can't breathe"), keyword("cant breathe"));
    }

    #[test]
    fn a_fine_check_in_does_not_escalate() {
        for text in [
            "I'm feeling fine, how are you?",
            "No, I haven't had a fall.",
            "I don't have any pain today.",
            "I haven't eaten anything yet.",
            "No, I don't need anything, thank you.",
        ] {
            assert_eq!(hit(text), None, "{text}");
        }
    }

    #[test]
    fn a_denied_symptom_still_escalates_by_design() {
        // The cost of ignoring negation, accepted in issue #11.
        assert_eq!(hit("No chest pain at all."), keyword("chest pain"));
    }

    #[test]
    fn a_phrase_split_across_utterances_is_caught_in_the_turn() {
        let hit = check("get up.", "I fell and I can't get up.");
        assert_eq!(hit.map(|h| h.phrase), Some("cant get up".into()));
    }

    #[test]
    fn every_emergency_number_spelling_matches() {
        assert_eq!(hit("Call 911!"), keyword("call 911"));
        assert_eq!(hit("call 9-1-1"), keyword("call 911"));
        assert_eq!(hit("Please call nine one one."), keyword("call nine one one"));
    }
}
