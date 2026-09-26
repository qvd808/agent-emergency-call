//! Scheduled outbound check-ins (issue #22). A round calls every resident on the list. Each one
//! gets up to [`Policy::attempts`] check-in attempts, [`Policy::retry_after`] apart, each
//! ringing for [`Policy::ring`]:
//!
//! - An answered attempt reaches the check-in, which runs as an inbound one does, with the
//!   outbound greeting. A resident who answers and then says nothing is the silence trigger's
//!   business (issue #11), not a missed attempt.
//! - Anything else is a missed attempt: no answer, busy, declined, the phone unreachable, or
//!   AMI failing.
//! - When every attempt is missed, a missed check-in alert naming the resident reaches the
//!   dispatcher as a mock notice: printed, and written to the round's log. Nothing is sent
//!   anywhere.
//!
//! Placing a call goes through Call control's [`Dial`] (issue #13); the schedule and the
//! retries live here, in the core.

use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::Serialize;
use tokio::task::JoinSet;
use tokio::time::{Instant, MissedTickBehavior};

use crate::telephony::audiosocket::Uuid;
use crate::telephony::{Dial, Placed};

/// How hard a check-in tries to reach the resident. The defaults are the ticket's.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Policy {
    pub attempts: u32,
    /// From the end of a missed attempt to the start of the next.
    pub retry_after: Duration,
    /// How long each attempt rings before it counts as unanswered.
    pub ring: Duration,
}

impl Default for Policy {
    fn default() -> Self {
        Policy {
            attempts: 3,
            retry_after: Duration::from_secs(120),
            ring: Duration::from_secs(30),
        }
    }
}

/// When rounds run: one at startup if `now` (`make checkin`), then one every `every`, if set.
/// A round finishes, retries and all, before the next one starts, so a resident never has two
/// check-ins going at once.
#[derive(Debug, Clone)]
pub struct Schedule {
    /// The residents' extensions.
    pub residents: Vec<String>,
    pub now: bool,
    pub every: Option<Duration>,
    pub policy: Policy,
}

/// Calls the agent has placed and not yet heard answered, each with the resident's extension,
/// so that when an answered call's media arrives it is known as a check-in the agent placed.
/// Cheap to clone.
#[derive(Clone, Default)]
pub struct Outbound(Arc<Mutex<HashMap<String, String>>>);

impl Outbound {
    fn expect(&self, call: &str, resident: &str) {
        self.0.lock().unwrap().insert(call.to_string(), resident.to_string());
    }

    fn forget(&self, call: &str) {
        self.0.lock().unwrap().remove(call);
    }

    /// The resident's extension, if the agent placed this call.
    pub fn take(&self, call: &str) -> Option<String> {
        self.0.lock().unwrap().remove(call)
    }
}

/// One resident's check-in in a round, as written to `calls/`.
#[derive(Debug, Serialize)]
pub struct CheckInLog {
    pub resident: String,
    /// Wall-clock start, in milliseconds since the Unix epoch.
    pub started_unix_ms: u128,
    pub attempts: Vec<AttemptLog>,
    /// The call that reached the check-in; its own log is `<call>.json`. `None` when every
    /// attempt was missed.
    pub reached: Option<String>,
    /// Whether a missed check-in alert went to the dispatcher.
    pub missed_check_in_alert: bool,
}

#[derive(Debug, Serialize)]
pub struct AttemptLog {
    /// The call's id: its channel's uniqueid in Asterisk, and its AudioSocket UUID if answered.
    pub call: String,
    pub started_unix_ms: u128,
    /// Why the attempt was missed; `None` if the resident answered.
    pub missed: Option<String>,
}

/// Runs rounds as `schedule` says, for as long as the agent runs.
pub async fn run<D: Dial + Clone + 'static>(
    schedule: Schedule,
    dial: D,
    outbound: Outbound,
    calls_dir: PathBuf,
) {
    if schedule.now {
        round(&schedule, &dial, &outbound, &calls_dir).await;
    }
    let Some(every) = schedule.every else { return };
    let mut ticks = tokio::time::interval_at(Instant::now() + every, every);
    // A round that runs past a tick starts the next one as soon as it ends, then keeps `every`
    // from there, rather than bursting to catch up.
    ticks.set_missed_tick_behavior(MissedTickBehavior::Delay);
    loop {
        ticks.tick().await;
        round(&schedule, &dial, &outbound, &calls_dir).await;
    }
}

/// Calls every resident at once, and writes each one's log once their check-in is settled.
async fn round<D: Dial + Clone + 'static>(
    schedule: &Schedule,
    dial: &D,
    outbound: &Outbound,
    calls_dir: &Path,
) {
    eprintln!("agent: check-in round for {}", schedule.residents.join(", "));
    let mut check_ins = JoinSet::new();
    for resident in &schedule.residents {
        let (resident, policy) = (resident.clone(), schedule.policy.clone());
        let (dial, outbound, calls_dir) = (dial.clone(), outbound.clone(), calls_dir.to_path_buf());
        check_ins.spawn(async move {
            let log = place_check_in(&resident, &policy, &dial, &outbound).await;
            match write(&calls_dir, &log) {
                Ok(path) => eprintln!("agent: check-in log written to {}", path.display()),
                Err(e) => eprintln!("agent: couldn't write the check-in log for {resident}: {e}"),
            }
        });
    }
    check_ins.join_all().await;
}

/// One resident's check-in: attempts until one is answered or every one is missed, then the
/// missed check-in alert if it comes to that.
pub async fn place_check_in<D: Dial>(
    resident: &str,
    policy: &Policy,
    dial: &D,
    outbound: &Outbound,
) -> CheckInLog {
    let mut log = CheckInLog {
        resident: resident.to_string(),
        started_unix_ms: unix_ms(),
        attempts: Vec::new(),
        reached: None,
        missed_check_in_alert: false,
    };
    for attempt in 1..=policy.attempts {
        if attempt > 1 {
            tokio::time::sleep(policy.retry_after).await;
        }
        let started_unix_ms = unix_ms();
        let (call, placed) = match new_call_id() {
            Ok(call) => {
                eprintln!(
                    "agent: check-in for {resident}: attempt {attempt} of {}, call {call}",
                    policy.attempts
                );
                outbound.expect(&call, resident);
                let placed = dial.place(resident, &call, policy.ring).await;
                (call, placed)
            }
            Err(e) => (String::new(), Placed::Missed(format!("no call id: {e}"))),
        };
        match placed {
            Placed::Answered => {
                eprintln!("agent: check-in for {resident}: call {call} answered");
                log.attempts.push(AttemptLog { call: call.clone(), started_unix_ms, missed: None });
                log.reached = Some(call);
                return log;
            }
            Placed::Missed(why) => {
                outbound.forget(&call);
                eprintln!("agent: check-in for {resident}: attempt {attempt} missed: {why}");
                log.attempts.push(AttemptLog { call, started_unix_ms, missed: Some(why) });
            }
        }
    }
    log.missed_check_in_alert = true;
    print_alert(&log);
    log
}

/// The mock notice: printed, and marked in the check-in's log.
fn print_alert(log: &CheckInLog) {
    let why: Vec<String> = log
        .attempts
        .iter()
        .enumerate()
        .map(|(i, a)| format!("attempt {}: {}", i + 1, a.missed.as_deref().unwrap_or("answered")))
        .collect();
    eprintln!(
        "agent: MOCK NOTICE TO THE DISPATCHER (not sent anywhere): missed check-in alert for the \
         resident on extension {}. Every attempt was missed: {}",
        log.resident,
        why.join("; "),
    );
}

/// Writes `<dir>/check-in-<resident>-<started_unix_ms>.json`.
fn write(dir: &Path, log: &CheckInLog) -> std::io::Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    let path = dir.join(format!("check-in-{}-{}.json", log.resident, log.started_unix_ms));
    let json = serde_json::to_string_pretty(log).map_err(std::io::Error::other)?;
    std::fs::write(&path, json + "\n")?;
    Ok(path)
}

/// A fresh random UUID for each attempt, since Asterisk refuses a uniqueid already in use
/// (issue #5's research, section 2). Version 4: 122 random bits, with the version and variant
/// set as RFC 9562 section 5.4 lays out (https://www.rfc-editor.org/rfc/rfc9562.txt).
fn new_call_id() -> std::io::Result<String> {
    let mut bytes = [0u8; 16];
    std::fs::File::open("/dev/urandom")?.read_exact(&mut bytes)?;
    bytes[6] = (bytes[6] & 0x0f) | 0x40; // version 0b0100
    bytes[8] = (bytes[8] & 0x3f) | 0x80; // variant 0b10
    Ok(Uuid(bytes).to_string())
}

fn unix_ms() -> u128 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;

    /// Plays Call control: each placed call gets the next scripted outcome, and is recorded
    /// with when it was placed.
    #[derive(Default)]
    struct Fake {
        outcomes: Mutex<VecDeque<Placed>>,
        placed: Mutex<Vec<(String, String, Instant)>>,
    }

    impl Fake {
        fn new(outcomes: Vec<Placed>) -> Self {
            Fake { outcomes: Mutex::new(outcomes.into()), ..Default::default() }
        }
    }

    impl Dial for Fake {
        async fn place(&self, extension: &str, call: &str, _: Duration) -> Placed {
            self.placed.lock().unwrap().push((extension.into(), call.into(), Instant::now()));
            self.outcomes.lock().unwrap().pop_front().unwrap()
        }
    }

    fn missed() -> Placed {
        Placed::Missed("no answer before the ring timeout".to_string())
    }

    #[tokio::test(start_paused = true)]
    async fn three_missed_attempts_two_minutes_apart_raise_the_alert() {
        let fake = Fake::new(vec![missed(), missed(), missed()]);
        let outbound = Outbound::default();
        let log = place_check_in("1001", &Policy::default(), &fake, &outbound).await;

        assert!(log.missed_check_in_alert);
        assert_eq!(log.reached, None);
        assert_eq!(log.attempts.len(), 3);
        assert!(log.attempts.iter().all(|a| a.missed.is_some()));
        let placed = fake.placed.lock().unwrap();
        assert!(placed.iter().all(|(extension, _, _)| extension == "1001"));
        assert_eq!(placed[1].2 - placed[0].2, Duration::from_secs(120));
        assert_eq!(placed[2].2 - placed[1].2, Duration::from_secs(120));
        // A fresh id each attempt, none of them still expected.
        assert!(placed[0].1 != placed[1].1 && placed[1].1 != placed[2].1);
        assert!(placed.iter().all(|(_, call, _)| outbound.take(call).is_none()));
    }

    #[tokio::test(start_paused = true)]
    async fn an_answered_attempt_reaches_the_check_in_and_stops_retrying() {
        let fake = Fake::new(vec![missed(), Placed::Answered]);
        let outbound = Outbound::default();
        let log = place_check_in("1001", &Policy::default(), &fake, &outbound).await;

        assert!(!log.missed_check_in_alert);
        assert_eq!(log.attempts.len(), 2);
        let answered = log.reached.unwrap();
        assert_eq!(answered, log.attempts[1].call);
        // The answered call's media is known as a check-in to 1001, once.
        assert_eq!(outbound.take(&answered).as_deref(), Some("1001"));
        assert_eq!(outbound.take(&answered), None);
        assert_eq!(outbound.take(&log.attempts[0].call), None);
    }

    #[test]
    fn call_ids_are_version_4_uuids() {
        let id = new_call_id().unwrap();
        assert_eq!(id.len(), 36);
        assert_eq!(&id[14..15], "4");
        assert!(matches!(&id[19..20], "8" | "9" | "a" | "b"), "{id}");
    }
}
