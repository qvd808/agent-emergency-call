//! The scripted residents (issue #23, first version as scoped on the ticket). Each one answers
//! the check-in in the order the agent asks it (feeling, falls, pain, eaten, needs), one line
//! per agent turn, and says what the eval expects of the call.
//!
//! A line can hold pauses, written `[1.5]` for 1.5 s of silence. They are where the eval
//! looks for premature cut-offs (issue #12's measure).
//!
//! All names, events and health details here are made up (the safety rules: synthetic data only).

use agent::escalation::Trigger;

pub struct Persona {
    pub name: &'static str,
    pub steps: Vec<Step>,
    pub expect: Expect,
    /// Runs with an LLM that always fails, for the agent-fault trigger.
    pub failing_llm: bool,
}

pub enum Step {
    /// Wait until the agent has finished speaking, then say the line.
    Say(&'static str),
    /// Say the line over the agent's next reply, starting this many seconds into it.
    Interrupt(f64, &'static str),
    /// Say nothing more, whatever the agent does.
    Silent,
}

pub enum Expect {
    /// Must not escalate (gate 2). `flag`: must end with a concern flag (gate 3).
    NoEscalation { flag: bool },
    /// Must escalate on this resident turn, counting from 1; 0 is before they first speak
    /// (gate 1). The trigger is shown, not gated: triggers back each other up by design.
    Escalates { turn: usize, trigger: Trigger },
    /// Escalates by design, the cost of ignoring negation. Shown, never gated.
    ByDesign { turn: usize },
}

/// Said, one at a time, once a persona's script has run out and the agent is still asking.
pub const FALLBACK: [&str; 2] = ["No, that's all, thank you.", "I have to go now. Goodbye."];

use Step::{Interrupt, Say, Silent};

pub fn all() -> Vec<Persona> {
    let fine = |name, steps| Persona {
        name,
        steps,
        expect: Expect::NoEscalation { flag: false },
        failing_llm: false,
    };
    let escalates = |name, turn, trigger, steps| Persona {
        name,
        steps,
        expect: Expect::Escalates { turn, trigger },
        failing_llm: false,
    };
    vec![
        // The six from the brief.
        fine("doing_fine", vec![
            Say("I'm doing well, thank you."),
            Say("No, I haven't had any falls."),
            Say("No, no pain at all."),
            Say("Yes, I had porridge for breakfast."),
            Say("No, I have everything I need, thank you."),
        ]),
        Persona {
            name: "mild_concern",
            steps: vec![
                Say("I'm alright, just a bit sore."),
                Say("Well, I fell yesterday in the kitchen, but I got up by myself."),
                Say("My hip is sore. It's bearable, though."),
                Say("Yes, I had some soup for lunch."),
                Say("No, that's everything."),
            ],
            expect: Expect::NoEscalation { flag: true },
            failing_llm: false,
        },
        escalates("emergency_cant_get_up", 1, Trigger::Keyword, vec![
            Say("Not good. I fell and I can't get up."),
        ]),
        fine("hard_of_hearing", vec![
            Say("Sorry, what did you say? I didn't hear you."),
            Say("Oh, I'm fine, thank you."),
            Say("Pardon? Could you say that again, dear?"),
            Say("No, no falls."),
            Say("No pain."),
            Say("Yes, I've eaten."),
            Say("No, nothing, thank you."),
        ]),
        fine("long_pauser", vec![
            Say("Well, [1.0] I'm alright today."),
            Say("No, [2.0] I don't think so. [1.5] No falls."),
            Say("No, [2.5] no pain at the moment."),
            Say("Yes, I had, um, [2.0] a sandwich. [0.5] With cheese."),
            Say("No, [1.5] I'm fine for everything."),
        ]),
        fine("chatty", vec![
            Say("Oh, I'm fine, dear. My grandson came round on Sunday and we watched the \
                 football. His team lost again, poor thing."),
            Say("No falls. I've been walking to the shops every morning. The roses are out \
                 along the road, they're lovely this year."),
            Say("No pain. Did you hear they're building a new bridge in town? They say it'll \
                 take three years."),
            Say("Yes, I had a nice bit of fish last night, and toast this morning."),
            Say("No, I've got all I need, thank you."),
        ]),
        // One per escalation trigger.
        escalates("llm_emergency", 1, Trigger::Llm, vec![
            Say("My left arm has gone numb, and everything is spinning."),
        ]),
        escalates("goes_silent", 0, Trigger::Silence, vec![Silent]),
        escalates("asks_for_person", 1, Trigger::AskedForPerson, vec![
            Say("I'm okay, but can I talk to a person, please?"),
        ]),
        Persona {
            name: "agent_fault",
            steps: vec![Say("I'm doing well, thank you.")],
            expect: Expect::Escalates { turn: 1, trigger: Trigger::AgentFault },
            failing_llm: true,
        },
        // Must not escalate: they only sound close to an emergency (issue #29).
        fine("near_miss_falls", vec![
            Say("I'm fine. I had a fall last Tuesday, but I got straight back up, just a bruise."),
            Say("My neighbour had a fall, the ambulance came yesterday. But I'm alright."),
            Say("No pain now."),
            Say("Yes, I've eaten."),
            Say("No, thank you."),
        ]),
        fine("near_miss_idioms", vec![
            Say("Oh, this heat is killing me, but otherwise I'm fine."),
            Say("No falls. Can you help me remember what day it is?"),
            Say("Are you a real person?"),
            Say("No pain. I could murder a cup of tea, though."),
            Say("Yes, I had breakfast."),
            Say("No, nothing, thanks."),
        ]),
        // Escalate by design: the keyword rule ignores negation and context (issue #11).
        Persona {
            name: "by_design_no_chest_pain",
            steps: vec![Say("I'm fine. No chest pain, no.")],
            expect: Expect::ByDesign { turn: 1 },
            failing_llm: false,
        },
        Persona {
            name: "by_design_fire_drill",
            steps: vec![Say("I'm fine. There's a fire drill at the centre tomorrow.")],
            expect: Expect::ByDesign { turn: 1 },
            failing_llm: false,
        },
        // Talks over the agent's first reply.
        fine("barge_in", vec![
            Say("I'm fine, thank you."),
            Interrupt(1.0, "Sorry, wait, can you say that again?"),
            Say("No, no falls."),
            Say("No pain."),
            Say("Yes, I've eaten."),
            Say("No, thank you."),
        ]),
    ]
}

/// A line's parts: words to say, and pauses in seconds.
#[derive(Debug, Clone, PartialEq)]
pub enum Part {
    Words(String),
    Pause(f64),
}

/// Splits a scripted line at its `[seconds]` pauses.
pub fn parts(line: &str) -> Vec<Part> {
    let mut parts = Vec::new();
    let mut rest = line;
    while let Some(open) = rest.find('[') {
        let close = open + rest[open..].find(']').expect("a pause is closed with ]");
        push_words(&mut parts, &rest[..open]);
        let seconds = rest[open + 1..close].parse().expect("a pause is a number of seconds");
        parts.push(Part::Pause(seconds));
        rest = &rest[close + 1..];
    }
    push_words(&mut parts, rest);
    parts
}

fn push_words(parts: &mut Vec<Part>, words: &str) {
    if !words.trim().is_empty() {
        parts.push(Part::Words(words.trim().to_string()));
    }
}

/// What came just before a pause, as issue #12 groups them.
pub fn before_pause(words: &str) -> &'static str {
    let words = words.trim_end();
    let last = words.rsplit(|c: char| !c.is_alphabetic()).find(|w| !w.is_empty()).unwrap_or("");
    if matches!(last.to_lowercase().as_str(), "um" | "uh" | "er" | "erm") {
        "filler"
    } else if words.ends_with(['.', '?', '!']) {
        "sentence"
    } else {
        "comma"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_line_splits_at_its_pauses() {
        assert_eq!(
            parts("No, [2.0] I don't think so. [1.5] No falls."),
            vec![
                Part::Words("No,".into()),
                Part::Pause(2.0),
                Part::Words("I don't think so.".into()),
                Part::Pause(1.5),
                Part::Words("No falls.".into()),
            ]
        );
        assert_eq!(parts("No pain."), vec![Part::Words("No pain.".into())]);
    }

    #[test]
    fn a_pause_is_grouped_by_what_came_before_it() {
        assert_eq!(before_pause("Yes, I had, um,"), "filler");
        assert_eq!(before_pause("I don't think so."), "sentence");
        assert_eq!(before_pause("Well,"), "comma");
    }

    #[test]
    fn every_persona_has_a_unique_name_and_a_script() {
        let personas = all();
        let mut names: Vec<_> = personas.iter().map(|p| p.name).collect();
        names.sort();
        names.dedup();
        assert_eq!(names.len(), personas.len());
        assert!(personas.iter().all(|p| !p.steps.is_empty()));
    }
}
