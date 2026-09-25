//! The check-in list (issue #35), kept by the code rather than remembered by the model.
//!
//! A check-in finds out five things. Each turn the model is told where the list stands and
//! which item to ask next; it answers with what the resident's words said about each item
//! ([`Marks`]) and which item its reply asks about ([`Asking`]). The code merges the marks and
//! counts the asks. An item asked [`MAX_ASKS`] times without a clear answer is closed as
//! such, so a mishearing or a resident who drifts off can't hold the call in a loop, and once
//! every item is closed the model is told to say goodbye.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::escalation::normalise;

/// Asks per item before it is closed without a clear answer. The greeting counts as the
/// first ask about feeling.
pub const MAX_ASKS: u32 = 2;

/// The items, in the order they are asked.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Item {
    Feeling,
    Falls,
    Pain,
    Eaten,
    Needs,
}

pub const ITEMS: [Item; 5] = [Item::Feeling, Item::Falls, Item::Pain, Item::Eaten, Item::Needs];

impl Item {
    fn index(self) -> usize {
        self as usize
    }

    /// What the item asks, as the model is told it.
    fn describe(self) -> &'static str {
        match self {
            Item::Feeling => "how they are feeling today",
            Item::Falls => "whether they have had a fall",
            Item::Pain => "whether they have any pain",
            Item::Eaten => "whether they have eaten today",
            Item::Needs => "whether they need anything",
        }
    }

    fn name(self) -> &'static str {
        match self {
            Item::Feeling => "feeling",
            Item::Falls => "falls",
            Item::Pain => "pain",
            Item::Eaten => "eaten",
            Item::Needs => "needs",
        }
    }
}

/// What their words just now said about each item, in a few words. null for every item their
/// words didn't touch: never write "not mentioned".
// Every field required but nullable, for the reason on `Turn::reason` (issue #6).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Marks {
    #[schemars(required, extend("type" = ["string", "null"]))]
    pub feeling: Option<String>,
    #[schemars(required, extend("type" = ["string", "null"]))]
    pub falls: Option<String>,
    #[schemars(required, extend("type" = ["string", "null"]))]
    pub pain: Option<String>,
    #[schemars(required, extend("type" = ["string", "null"]))]
    pub eaten: Option<String>,
    #[schemars(required, extend("type" = ["string", "null"]))]
    pub needs: Option<String>,
}

impl Marks {
    fn get(&self, item: Item) -> Option<&str> {
        match item {
            Item::Feeling => self.feeling.as_deref(),
            Item::Falls => self.falls.as_deref(),
            Item::Pain => self.pain.as_deref(),
            Item::Eaten => self.eaten.as_deref(),
            Item::Needs => self.needs.as_deref(),
        }
        .map(str::trim)
        .filter(|note| !is_placeholder(note))
    }
}

/// A note that says nothing was said: the model writes "not mentioned" or "none mentioned" for
/// an untouched item instead of null (seen in the text runs of 2026-09-24), which would close
/// the item unasked. "none" alone is kept: for falls it is an answer.
fn is_placeholder(note: &str) -> bool {
    let note = note.trim().trim_end_matches('.').to_lowercase();
    note.is_empty()
        || note.contains("mention")
        || note.contains("not said")
        || note.contains("not asked")
        || note.contains("not discussed")
        || note.contains("no answer")
        || note.contains("unknown")
        || matches!(note.as_str(), "null" | "n/a" | "na" | "-")
}

/// What the reply asks about.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Asking {
    Feeling,
    Falls,
    Pain,
    Eaten,
    Needs,
    // One question about a problem they just mentioned. A `//` comment: a doc comment on a
    // variant would turn the schema's plain `enum` into a `oneOf`.
    FollowUp,
    // They asked for medical advice: the reply offers to put them through to a person. A "yes"
    // to it escalates, checked in code ([`accepts_offer`]); a "no" carries on.
    OfferPerson,
    Goodbye,
}

impl Asking {
    fn item(self) -> Option<Item> {
        match self {
            Asking::Feeling => Some(Item::Feeling),
            Asking::Falls => Some(Item::Falls),
            Asking::Pain => Some(Item::Pain),
            Asking::Eaten => Some(Item::Eaten),
            Asking::Needs => Some(Item::Needs),
            Asking::FollowUp | Asking::OfferPerson | Asking::Goodbye => None,
        }
    }
}

/// Where one item stands.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case", tag = "state", content = "note")]
pub enum Entry {
    Open,
    /// What they said, in the model's few words.
    Answered(String),
    /// Asked [`MAX_ASKS`] times without a clear answer.
    NoClearAnswer,
}

#[derive(Debug, Clone)]
pub struct Checklist {
    entries: [Entry; 5],
    asks: [u32; 5],
    /// What the last reply asked about.
    last: Option<Asking>,
}

/// In the call log, keyed by item: `"falls": {"state": "answered", "note": "no", "asks": 1}`.
impl Serialize for Checklist {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;

        #[derive(Serialize)]
        struct Logged<'a> {
            #[serde(flatten)]
            entry: &'a Entry,
            asks: u32,
        }
        let mut map = serializer.serialize_map(Some(ITEMS.len()))?;
        for item in ITEMS {
            let i = item.index();
            map.serialize_entry(item.name(), &Logged { entry: &self.entries[i], asks: self.asks[i] })?;
        }
        map.end()
    }
}

impl Default for Checklist {
    fn default() -> Self {
        let mut asks = [0; 5];
        // The fixed greeting asks how they are feeling.
        asks[Item::Feeling.index()] = 1;
        let entries = std::array::from_fn(|_| Entry::Open);
        Checklist { entries, asks, last: Some(Asking::Feeling) }
    }
}

impl Checklist {
    /// Takes in a turn: the model's marks for `heard`, the resident's words, then what its
    /// reply asks.
    ///
    /// A mark for the item just asked is taken as it is: "no" answers "have you had a fall?".
    /// A mark for any other item must share a word with `heard`, or it is dropped: the model
    /// marked eaten "no" for a resident who never mentioned food (text run of 2026-09-24), and
    /// the item would have gone unasked. A grounded mark for an item already answered is a
    /// correction, and is added after the first answer.
    pub fn record(&mut self, marks: &Marks, asking: Asking, heard: &str) -> Recorded {
        let just_asked = self.last.and_then(Asking::item);
        let heard = normalise(heard);
        let mut recorded = Recorded::default();
        for item in ITEMS {
            let Some(note) = marks.get(item) else { continue };
            // The model often repeats earlier marks, so an answered item changes only when the
            // resident's words this turn are behind a new note: "no fall", then "actually, I
            // fell off my chair" (live call of 2026-09-25, issue #51). The earlier answer stays
            // in front, so a correction never loses what was said. Only an exact repeat counts
            // as one: "fall" after "no fall" is a correction, not a repeat. One closed without
            // a clear answer still takes a late answer.
            if let Entry::Answered(old) = &mut self.entries[item.index()] {
                let repeat = normalise(old).ends_with(&normalise(note));
                if !repeat && grounded(note, &heard) {
                    *old = format!("{old}, then {note}");
                }
                continue;
            }
            if Some(item) == just_asked || grounded(note, &heard) {
                self.entries[item.index()] = Entry::Answered(note.to_string());
            } else {
                recorded.ungrounded.push(item);
            }
        }
        // The resident has now answered (or not) every ask so far: an item asked its full
        // count and still open gets no more asks.
        for item in ITEMS {
            let i = item.index();
            if self.entries[i] == Entry::Open && self.asks[i] >= MAX_ASKS {
                self.entries[i] = Entry::NoClearAnswer;
                recorded.closed.push(item);
            }
        }
        if let Some(item) = asking.item() {
            self.asks[item.index()] += 1;
        }
        self.last = Some(asking);
        recorded
    }

    /// The first open item, if any.
    pub fn next(&self) -> Option<Item> {
        ITEMS.into_iter().find(|item| self.entries[item.index()] == Entry::Open)
    }

    /// What the last reply asked about; the greeting counts as asking about feeling.
    pub fn last(&self) -> Option<Asking> {
        self.last
    }

    pub fn done(&self) -> bool {
        self.next().is_none()
    }

    /// Where the list stands, for the model, sent with the resident's latest words.
    /// `wants_to_end`: those words ask to end the call ([`wants_to_end`]).
    pub fn note(&self, wants_to_end: bool) -> String {
        let mut note = String::from("[Check-in list, kept by the system, before their words above:\n");
        for item in ITEMS {
            let i = item.index();
            let state = match &self.entries[i] {
                Entry::Answered(what) => format!("done ({what})"),
                Entry::NoClearAnswer => "closed, no clear answer; don't ask again".to_string(),
                Entry::Open if self.asks[i] >= MAX_ASKS => format!(
                    "open, asked {} times already; if their words above don't answer it, \
                     leave it and don't ask again",
                    self.asks[i]
                ),
                Entry::Open if self.asks[i] > 0 => format!(
                    "open, asked {} time; if you ask again, use different words",
                    self.asks[i]
                ),
                Entry::Open => "open".to_string(),
            };
            note.push_str(&format!("- {}: {} - {state}\n", item.name(), item.describe()));
        }
        if wants_to_end {
            note.push_str(
                "They want to end the call. Mark what their words above answer, then say a \
                 short, warm goodbye now: asking is goodbye and end_call is true.]",
            );
            return note;
        }
        let open = |item: Item| self.entries[item.index()] == Entry::Open;
        // Their words most likely answer the question just asked: the greeting's question
        // about feeling on the first turn (issue #35).
        let just_asked = self.last.and_then(Asking::item).filter(|&item| open(item));
        let next = ITEMS
            .into_iter()
            .find(|&item| open(item) && Some(item) != just_asked && self.asks[item.index()] < MAX_ASKS);
        if just_asked.is_none() && next.is_none() {
            note.push_str(
                "Nothing is left to ask. Mark what their words above answer, then say a short, \
                 warm goodbye: asking is goodbye and end_call is true.]",
            );
            return note;
        }
        let then = match next {
            Some(item) => format!("ask about {}", item.name()),
            None => "say goodbye: nothing else is left to ask".to_string(),
        };
        match just_asked {
            Some(item) => {
                note.push_str(&format!(
                    "You last asked about {0}, so their words above most likely answer it: mark \
                     {0} from them, and anything else they answered. Then {then}. ",
                    item.name()
                ));
                if self.asks[item.index()] < MAX_ASKS {
                    note.push_str(&format!(
                        "Only if they plainly didn't answer about {0} (they didn't hear, or it \
                         makes no sense), ask about {0} again instead, in different words. ",
                        item.name()
                    ));
                }
            }
            None => note.push_str(&format!(
                "First mark every item their words above answer. Then {then}, unless you just \
                 marked it; then the next item still open. "
            )),
        }
        if self.last == Some(Asking::OfferPerson) {
            note.push_str(
                "You offered to put them through to a person, and they didn't take it: carry on \
                 with the list.]",
            );
        } else if self.last == Some(Asking::FollowUp) {
            note.push_str("You asked a follow-up last turn, so no follow-up now.]");
        } else {
            note.push_str("If they just mentioned a problem, you may ask one follow-up first.]");
        }
        note
    }
}

/// What [`Checklist::record`] did besides taking the marks in.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Recorded {
    /// Items closed without a clear answer.
    pub closed: Vec<Item>,
    /// Marks dropped because nothing in the resident's words backs them.
    pub ungrounded: Vec<Item>,
}

/// Words too common to show that a note came from what the resident said.
const COMMON: &[&str] = &[
    "the", "and", "but", "not", "yes", "has", "had", "have", "any", "all", "they", "their",
    "them", "was", "are", "for", "with", "just", "now", "today", "said", "says", "some", "bit",
    "very", "that", "this", "its", "one", "thanks", "thank", "you",
];

/// `note` shares a word with `heard`: equal, or the same first four letters ("fall" in
/// "falls" and "fallen"). Words under three letters and [`COMMON`] words don't count.
fn grounded(note: &str, heard: &[String]) -> bool {
    let content = |w: &&String| w.len() >= 3 && !COMMON.contains(&w.as_str());
    let same = |a: &str, b: &str| a == b || (a.len() >= 4 && b.len() >= 4 && a[..4] == b[..4]);
    normalise(note)
        .iter()
        .filter(content)
        .any(|n| heard.iter().filter(content).any(|h| same(n, h)))
}

/// The resident's answer takes the offer of a person: a yes that doesn't start with a no.
/// "Yes please" and "okay, I would" do; "no thanks, I'm fine" and "not now" don't. An
/// answer that is neither goes to the model as usual, which may offer again.
pub fn accepts_offer(text: &str) -> bool {
    const NO: &[&str] = &["no", "nope", "nah", "not", "dont", "im fine", "im okay", "im ok"];
    const YES: &[&str] = &[
        "yes", "yeah", "yep", "please", "sure", "okay", "ok", "alright", "all right", "i would",
        "go ahead", "that would", "i think so", "i guess so",
    ];
    let words = normalise(text);
    let joined = format!(" {} ", words.join(" "));
    let starts_with_no = NO.iter().any(|no| format!(" {joined}").starts_with(&format!("  {no} ")));
    !starts_with_no && YES.iter().any(|yes| joined.contains(&format!(" {yes} ")))
}

/// The resident's words ask to end the call: "I need to go", "can you hang up?", "goodbye".
/// The model is then told to say goodbye at once, whatever is left on the list; left to
/// itself it kept asking (issue #35).
pub fn wants_to_end(text: &str) -> bool {
    const PHRASES: &[&str] = &[
        "need to go", "have to go", "got to go", "gotta go", "must go", "better go",
        "let you go", "end the call", "end this call", "hang up", "goodbye", "bye",
    ];
    let words = format!(" {} ", normalise(text).join(" "));
    PHRASES.iter().any(|phrase| words.contains(&format!(" {phrase} ")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn marks(feeling: Option<&str>, falls: Option<&str>) -> Marks {
        Marks {
            feeling: feeling.map(Into::into),
            falls: falls.map(Into::into),
            ..Marks::default()
        }
    }

    #[test]
    fn the_first_answer_is_taken_as_the_greeting_s_answer() {
        let note = Checklist::default().note(false);
        assert!(note.contains("You last asked about feeling"), "{note}");
        assert!(note.contains("Then ask about falls"), "{note}");
    }

    #[test]
    fn a_yes_takes_the_offer_of_a_person() {
        for yes in ["Yes, please.", "Yeah.", "Okay, I would.", "Sure, go ahead.", "I think so."] {
            assert!(accepts_offer(yes), "{yes}");
        }
        for no in ["No, thanks, I'm fine.", "Not now.", "Nope, it's okay.", "I'm fine, really.",
                   "What do you mean?", "Hmm."] {
            assert!(!accepts_offer(no), "{no}");
        }
    }

    #[test]
    fn after_a_declined_offer_the_list_carries_on() {
        let mut list = Checklist::default();
        list.record(&marks(Some("hip sore"), None), Asking::OfferPerson, "Should I see a doctor?");
        assert_eq!(list.last(), Some(Asking::OfferPerson));
        assert!(list.note(false).contains("they didn't take it: carry on"));
    }

    #[test]
    fn asking_to_go_ends_the_call() {
        assert!(wants_to_end("I'm fine, thanks. I need to go now, baking's almost done!"));
        assert!(wants_to_end("Can you end the call?"));
        assert!(wants_to_end("Okay, bye bye."));
        assert!(!wants_to_end("I'm going to the shops later."));
        assert!(!wants_to_end("I had a big lunch."));
        let note = Checklist::default().note(true);
        assert!(note.contains("say a short, warm goodbye now"), "{note}");
    }

    #[test]
    fn the_greeting_counts_as_asking_about_feeling() {
        let list = Checklist::default();
        assert_eq!(list.next(), Some(Item::Feeling));
        assert!(list.note(false).contains("feeling: how they are feeling today - open, asked 1 time"));
        assert!(list.note(false).contains("use different words"));
    }

    #[test]
    fn an_answer_in_passing_closes_its_item_too() {
        let mut list = Checklist::default();
        list.record(&marks(Some("fine"), Some("no falls")), Asking::Pain, "Fine, no falls.");
        assert_eq!(list.next(), Some(Item::Pain));
        assert!(list.note(false).contains("falls: whether they have had a fall - done (no falls)"));
    }

    #[test]
    fn an_empty_note_is_not_an_answer() {
        let mut list = Checklist::default();
        list.record(&marks(Some("  "), None), Asking::Feeling, "Sorry?");
        assert_eq!(list.next(), Some(Item::Feeling));
    }

    #[test]
    fn not_mentioned_is_not_an_answer() {
        let mut list = Checklist::default();
        for placeholder in ["not mentioned", "None mentioned.", "null", "unknown", "N/A"] {
            list.record(&marks(None, Some(placeholder)), Asking::Falls, "What?");
            assert_eq!(list.next(), Some(Item::Feeling), "{placeholder}");
            assert!(!list.note(false).contains("falls: whether they have had a fall - done"));
        }
        list.record(&marks(None, Some("none")), Asking::Pain, "None.");
        assert!(list.note(false).contains("falls: whether they have had a fall - done (none)"));
    }

    #[test]
    fn an_item_asked_twice_without_an_answer_is_closed() {
        let mut list = Checklist::default();
        // Greeting (1st ask) unanswered; the reply asks again (2nd).
        assert_eq!(list.record(&Marks::default(), Asking::Feeling, "Eh?"), Recorded::default());
        assert!(list.note(false).contains("asked 2 times already"));
        // Still no answer: closed, and the list moves on.
        assert_eq!(list.record(&Marks::default(), Asking::Falls, "Eh?").closed, [Item::Feeling]);
        assert_eq!(list.next(), Some(Item::Falls));
        assert!(list.note(false).contains("closed, no clear answer"));
    }

    #[test]
    fn a_follow_up_is_not_allowed_twice_in_a_row() {
        let mut list = Checklist::default();
        list.record(&marks(Some("hip hurts"), None), Asking::FollowUp, "My hip hurts.");
        assert!(list.note(false).contains("no follow-up now"));
        list.record(&Marks::default(), Asking::Falls, "It's fine.");
        assert!(list.note(false).contains("you may ask one follow-up"));
    }

    #[test]
    fn a_mark_for_another_item_needs_the_resident_s_words_behind_it() {
        let mut list = Checklist::default();
        let heard = "Not too bad. My hip's sore from falling yesterday.";
        let marks = Marks {
            feeling: Some("not too bad".into()),
            falls: Some("fell yesterday".into()),
            pain: Some("sore hip".into()),
            eaten: Some("no".into()),
            needs: Some("nothing mentioned about needs".into()),
        };
        let recorded = list.record(&marks, Asking::FollowUp, heard);
        // eaten "no" has nothing behind it; the needs note is a placeholder, dropped earlier.
        assert_eq!(recorded.ungrounded, [Item::Eaten]);
        assert_eq!(list.next(), Some(Item::Eaten));
        assert!(list.note(false).contains("pain: whether they have any pain - done (sore hip)"));
    }

    #[test]
    fn the_item_just_asked_takes_a_bare_answer() {
        let mut list = Checklist::default();
        list.record(&marks(Some("fine"), None), Asking::Falls, "Fine.");
        list.record(&marks(None, Some("no")), Asking::Pain, "No.");
        assert!(list.note(false).contains("falls: whether they have had a fall - done (no)"));
    }

    #[test]
    fn a_correction_in_the_resident_s_words_updates_an_answered_item() {
        let mut list = Checklist::default();
        list.record(&marks(Some("fine"), None), Asking::Falls, "I'm feeling fine.");
        list.record(&marks(None, Some("no fall")), Asking::Pain, "No, I haven't had a fall.");
        let heard = "Actually, I do have a fall. It was yesterday, I fell off my chair.";
        list.record(&marks(None, Some("fell off my chair yesterday")), Asking::Eaten, heard);
        let note = list.note(false);
        let corrected = "done (no fall, then fell off my chair yesterday)";
        assert!(note.contains(&format!("falls: whether they have had a fall - {corrected}")), "{note}");
        // "fall" after "no fall" is a correction, not a repeat.
        list.record(&marks(None, Some("fall")), Asking::Needs, "I did fall.");
        assert!(list.note(false).contains("then fell off my chair yesterday, then fall)"));
    }

    #[test]
    fn a_repeated_or_unfounded_mark_leaves_an_answered_item_alone() {
        let mut list = Checklist::default();
        list.record(&marks(Some("fine"), None), Asking::Falls, "Fine.");
        list.record(&marks(None, Some("no fall")), Asking::Pain, "No fall.");
        // The model repeats the note, even with the resident's words behind it.
        list.record(&marks(None, Some("No fall.")), Asking::Eaten, "No fall, no pain either.");
        // A different note with nothing this turn behind it.
        list.record(&marks(None, Some("fell yesterday")), Asking::Needs, "I had some toast.");
        assert!(list.note(false).contains("falls: whether they have had a fall - done (no fall)"));
    }

    #[test]
    fn the_log_keys_each_item_by_name() {
        let mut list = Checklist::default();
        list.record(&marks(Some("fine"), None), Asking::Falls, "Fine.");
        let json = serde_json::to_value(&list).unwrap();
        assert_eq!(json["feeling"], serde_json::json!({"state": "answered", "note": "fine", "asks": 1}));
        assert_eq!(json["falls"], serde_json::json!({"state": "open", "asks": 1}));
    }

    #[test]
    fn a_full_list_says_goodbye() {
        let mut list = Checklist::default();
        let all = Marks {
            feeling: Some("fine".into()),
            falls: Some("no falls".into()),
            pain: Some("no pain".into()),
            eaten: Some("porridge".into()),
            needs: Some("nothing".into()),
        };
        let heard = "Fine. No falls, no pain. I had porridge, and I need nothing.";
        assert_eq!(list.record(&all, Asking::Goodbye, heard), Recorded::default());
        assert!(list.done());
        assert!(list.note(false).contains("Nothing is left to ask"));
    }
}
