//! Barge-in: the resident talking over the agent (issue #19). The policy is BI-2, the one
//! issue #12 settled on: pause the agent's audio on 2 VAD windows of speech; confirm the
//! interruption on 0.8 s of speech, 3 words or a keyword; otherwise resume after 1 s of
//! silence. Pausing first keeps the agent from talking over the resident at once, and
//! confirming before giving up the turn keeps a backchannel ("yeah") from costing the agent
//! its sentence.
//!
//! A pure state machine, fed one VAD window at a time while the agent is talking. The words
//! come from speech to text, a few hundred milliseconds after the speech; the live agent hands
//! them to [`confirms`] when they arrive, which the prototype's oracle did at once.

use crate::{VAD_ON, WINDOW_S};

/// Windows of speech that pause the agent: 64 ms.
pub const PAUSE_WINDOWS: u32 = 2;
/// Speech during a pause that confirms it. Slowed Piper backchannels ran past 0.5 s in the
/// prototype, hence 0.8 s (issue #12).
pub const CONFIRM_SPEECH: f64 = 0.8;
/// Words during a pause that confirm it.
pub const CONFIRM_WORDS: usize = 3;
/// Silence during a pause that resumes the agent.
pub const RESUME_AFTER: f64 = 1.0;
/// Words that confirm an interruption on their own (issue #3's keyword idea).
pub const KEYWORDS: [&str; 4] = ["stop", "wait", "help", "repeat"];

#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    /// The resident started talking: pause the agent's audio.
    Pause,
    /// The resident has the turn: drop the rest of the agent's audio and listen.
    Confirm(Confirmed),
    /// It wasn't an interruption: play on from where the audio paused.
    Resume,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Confirmed {
    /// Seconds of speech.
    Speech(f64),
    Words(usize),
    Keyword(String),
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum State {
    /// Speech windows in a row so far.
    Watching { run: u32 },
    /// Speech heard since the pause, and when the latest silence began.
    Paused { speech: f64, silent_since: Option<f64> },
}

#[derive(Debug, Clone)]
pub struct BargeIn {
    state: State,
}

impl Default for BargeIn {
    fn default() -> Self {
        BargeIn { state: State::Watching { run: 0 } }
    }
}

impl BargeIn {
    pub fn paused(&self) -> bool {
        matches!(self.state, State::Paused { .. })
    }

    /// Back to watching: the agent has stopped talking, or the interruption was settled
    /// outside this machine.
    pub fn reset(&mut self) {
        self.state = State::Watching { run: 0 };
    }

    /// One VAD window, starting at `t` seconds, while the agent is talking or paused.
    pub fn push(&mut self, t: f64, p: f32) -> Option<Action> {
        let speech = p >= VAD_ON;
        match &mut self.state {
            State::Watching { run } => {
                if !speech {
                    *run = 0;
                    return None;
                }
                *run += 1;
                if *run < PAUSE_WINDOWS {
                    return None;
                }
                self.state = State::Paused { speech: PAUSE_WINDOWS as f64 * WINDOW_S, silent_since: None };
                Some(Action::Pause)
            }
            State::Paused { speech: heard, silent_since } => {
                if speech {
                    *heard += WINDOW_S;
                    *silent_since = None;
                    if *heard >= CONFIRM_SPEECH - 1e-9 {
                        let heard = *heard;
                        self.reset();
                        return Some(Action::Confirm(Confirmed::Speech(heard)));
                    }
                    return None;
                }
                let since = *silent_since.get_or_insert(t);
                if t + WINDOW_S - since >= RESUME_AFTER - 1e-9 {
                    self.reset();
                    return Some(Action::Resume);
                }
                None
            }
        }
    }
}

/// Whether words said over the agent confirm an interruption: [`CONFIRM_WORDS`] or more, or a
/// keyword among them.
pub fn confirms(text: &str) -> Option<Confirmed> {
    let lower = text.to_lowercase();
    let words: Vec<&str> =
        lower.split(|c: char| !c.is_alphabetic() && c != '\'').filter(|w| !w.is_empty()).collect();
    if let Some(keyword) = words.iter().find(|w| KEYWORDS.contains(w)) {
        return Some(Confirmed::Keyword(keyword.to_string()));
    }
    (words.len() >= CONFIRM_WORDS).then_some(Confirmed::Words(words.len()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Feeds `n` windows of probability `p` from `*t`; returns the actions.
    fn feed(b: &mut BargeIn, t: &mut f64, n: usize, p: f32) -> Vec<Action> {
        (0..n)
            .filter_map(|_| {
                let action = b.push(*t, p);
                *t += WINDOW_S;
                action
            })
            .collect()
    }

    fn windows(seconds: f64) -> usize {
        (seconds / WINDOW_S).ceil() as usize
    }

    #[test]
    fn two_windows_of_speech_pause_the_agent() {
        let (mut b, mut t) = (BargeIn::default(), 0.0);
        assert_eq!(feed(&mut b, &mut t, 1, 0.9), vec![]);
        assert_eq!(feed(&mut b, &mut t, 1, 0.9), vec![Action::Pause]);
        assert!(b.paused());
    }

    #[test]
    fn a_single_loud_window_does_not() {
        let (mut b, mut t) = (BargeIn::default(), 0.0);
        feed(&mut b, &mut t, 1, 0.9);
        feed(&mut b, &mut t, 1, 0.1);
        assert_eq!(feed(&mut b, &mut t, 1, 0.9), vec![]);
        assert!(!b.paused());
    }

    #[test]
    fn long_enough_speech_confirms() {
        let (mut b, mut t) = (BargeIn::default(), 0.0);
        let actions = feed(&mut b, &mut t, windows(CONFIRM_SPEECH), 0.9);
        assert_eq!(actions.len(), 2, "{actions:?}");
        assert_eq!(actions[0], Action::Pause);
        assert!(matches!(actions[1], Action::Confirm(Confirmed::Speech(s)) if s >= CONFIRM_SPEECH - 1e-9));
        assert!(!b.paused());
    }

    #[test]
    fn a_backchannel_then_silence_resumes() {
        let (mut b, mut t) = (BargeIn::default(), 0.0);
        // "Yeah": 0.3 s, well short of confirming.
        assert_eq!(feed(&mut b, &mut t, windows(0.3), 0.9), vec![Action::Pause]);
        let actions = feed(&mut b, &mut t, windows(RESUME_AFTER), 0.1);
        assert_eq!(actions, vec![Action::Resume]);
        assert!(!b.paused());
    }

    #[test]
    fn speech_during_the_wait_restarts_it() {
        let (mut b, mut t) = (BargeIn::default(), 0.0);
        feed(&mut b, &mut t, 2, 0.9);
        assert_eq!(feed(&mut b, &mut t, windows(0.6), 0.1), vec![]);
        feed(&mut b, &mut t, 2, 0.9);
        assert_eq!(feed(&mut b, &mut t, windows(0.6), 0.1), vec![], "the silence started again");
    }

    #[test]
    fn three_words_or_a_keyword_confirm() {
        assert_eq!(confirms("Sorry, can you say that again?"), Some(Confirmed::Words(6)));
        assert_eq!(confirms("Wait."), Some(Confirmed::Keyword("wait".into())));
        assert_eq!(confirms("Help!"), Some(Confirmed::Keyword("help".into())));
        assert_eq!(confirms("Yeah."), None);
        assert_eq!(confirms("Oh, okay."), None);
        assert_eq!(confirms("I don't know"), Some(Confirmed::Words(3)));
    }
}
