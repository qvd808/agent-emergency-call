//! The results table (issue #23): gates 1-4 of issue #29, which fail the run, and the
//! report-only figures, which never do. It reads two sources per call: what the fake caller
//! timed, as the resident hears the call, and the agent's own call log.

use std::fmt::Write;

use serde_json::Value;

use crate::caller::{Ended, Outcome};
use crate::personas::{Expect, Persona};

pub struct Run<'a> {
    pub persona: &'a Persona,
    pub repeat: usize,
    pub outcome: Outcome,
    /// The agent's call log, if it was written.
    pub log: Option<Value>,
}

pub struct Meta {
    pub run: String,
    pub model: String,
    pub temperature: f32,
    pub repeat: usize,
    pub dispatcher: String,
    /// Gate 4's limit: the end-of-turn wait plus 1 s (issue #29).
    pub keyword_limit_ms: f64,
}

/// Issue #12's pause-length buckets, plus one for anything outside them.
const BUCKETS: [(f64, f64, &str); 4] =
    [(0.3, 0.5, "0.3-0.5 s"), (0.7, 1.1, "0.7-1.1 s"), (1.3, 1.6, "1.3-1.6 s"), (2.0, 3.0, "2.0-3.0 s")];

struct Escalated {
    trigger: String,
    turn: u64,
    detected_ms: Option<f64>,
    latency_ms: Option<f64>,
    transfer: String,
}

fn escalation(log: &Value) -> Option<Escalated> {
    let e = log.get("escalation").filter(|e| !e.is_null())?;
    Some(Escalated {
        trigger: e["trigger"].as_str().unwrap_or("?").to_string(),
        turn: e["turn"].as_u64().unwrap_or(0),
        detected_ms: e["detected_ms"].as_f64(),
        latency_ms: e["latency_ms"].as_f64(),
        transfer: e["transfer"].as_str().unwrap_or("none").to_string(),
    })
}

fn trigger_name(trigger: agent::escalation::Trigger) -> String {
    serde_json::to_value(trigger).ok().and_then(|v| v.as_str().map(String::from)).unwrap_or_default()
}

/// Nearest-rank percentile.
fn percentile(values: &[f64], p: f64) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    let rank = ((p * sorted.len() as f64).ceil() as usize).clamp(1, sorted.len());
    Some(sorted[rank - 1])
}

fn ms(value: Option<f64>) -> String {
    value.map_or("–".into(), |v| format!("{v:.0}"))
}

fn turns(log: &Value) -> &[Value] {
    log["turns"].as_array().map_or(&[], Vec::as_slice)
}

/// Every time the agent decided the resident's turn was over, in Unix milliseconds, and
/// whether it answered: turns it answered, turns it took and gave back when the resident went on
/// (issue #19), and turns that held only noise.
fn decisions_ms(log: &Value) -> Vec<(f64, bool)> {
    let start = log["started_unix_ms"].as_f64().unwrap_or(0.0);
    let at = |s: f64| start + s * 1000.0;
    let mut out = Vec::new();
    for t in turns(log) {
        out.extend(t["taken_at_s"].as_f64().map(|s| (at(s), true)));
        let back = t["taken_back_at_s"].as_array().map_or(&[][..], Vec::as_slice);
        out.extend(back.iter().filter_map(Value::as_f64).map(|s| (at(s), false)));
    }
    let unheard = log["unheard_turns_at_s"].as_array().map_or(&[][..], Vec::as_slice);
    out.extend(unheard.iter().filter_map(Value::as_f64).map(|s| (at(s), false)));
    out
}

struct Gate {
    name: &'static str,
    pass: bool,
    detail: String,
}

/// Renders the report as Markdown, and whether every gate passed.
pub fn render(runs: &[Run], meta: &Meta) -> (String, bool) {
    let mut out = String::new();
    let w = &mut out;
    let _ = writeln!(w, "# Eval run {}\n", meta.run);
    let _ = writeln!(
        w,
        "{} calls ({} per persona), LLM {} at temperature {}, dispatcher extension {}. Every \
         call ran in real time through the agent's AudioSocket path; the residents are Piper's \
         `en_US-lessac-medium` voice at 8 kHz.\n",
        runs.len(),
        meta.repeat,
        meta.model,
        meta.temperature,
        meta.dispatcher
    );

    // Per call.
    let _ = writeln!(w, "## Calls\n");
    let _ = writeln!(
        w,
        "| Persona | Run | Expected | Escalation | Final status | Concern flag | Turns | Reply p50 (ms) | Ended | Result |"
    );
    let _ = writeln!(w, "|---|---|---|---|---|---|---|---|---|---|");
    let mut gate1 = (0, 0);
    let mut gate2 = (0, 0);
    let mut gate3 = (0, 0);
    let mut keyword_ms: Vec<f64> = Vec::new();
    let mut keyword_over = 0;
    for run in runs {
        let log = run.log.as_ref();
        let got = log.and_then(escalation);
        let expected = match &run.persona.expect {
            Expect::NoEscalation { flag: false } => "no escalation".to_string(),
            Expect::NoEscalation { flag: true } => "no escalation, flag".to_string(),
            Expect::Escalates { turn, trigger } => {
                format!("escalates on turn {turn} ({})", trigger_name(*trigger))
            }
            Expect::ByDesign { turn } => format!("escalates by design, turn {turn}"),
        };
        let flag = log.is_some_and(|l| !l["concern_flag"].is_null());
        let result = match (&run.persona.expect, &got, log) {
            (_, _, None) => {
                gate1.1 += matches!(run.persona.expect, Expect::Escalates { .. }) as usize;
                gate2.1 += matches!(run.persona.expect, Expect::NoEscalation { .. }) as usize;
                "FAIL (no call log)"
            }
            (Expect::Escalates { turn, .. }, got, _) => {
                gate1.1 += 1;
                let pass = got.as_ref().is_some_and(|g| g.turn == *turn as u64);
                gate1.0 += pass as usize;
                if pass { "PASS" } else { "FAIL" }
            }
            (Expect::NoEscalation { flag: wants_flag }, got, _) => {
                gate2.1 += 1;
                let mut pass = got.is_none();
                gate2.0 += pass as usize;
                if *wants_flag {
                    gate3.1 += 1;
                    gate3.0 += flag as usize;
                    pass &= flag;
                }
                if pass { "PASS" } else { "FAIL" }
            }
            (Expect::ByDesign { .. }, _, _) => "report only",
        };
        if let Some(g) = &got
            && g.trigger == "keyword"
            && let Some(detected) = g.detected_ms
        {
            keyword_ms.push(detected);
            keyword_over += (detected > meta.keyword_limit_ms) as usize;
        }
        let replies: Vec<f64> = run.outcome.lines.iter().filter_map(|l| l.reply_ms).collect();
        let _ = writeln!(
            w,
            "| {} | {} | {} | {} | {} | {} | {} | {} | {} | {} |",
            run.persona.name,
            run.repeat,
            expected,
            got.as_ref().map_or("none".into(), |g| format!("{} on turn {}", g.trigger, g.turn)),
            log.and_then(|l| l["final_status"].as_str()).unwrap_or("–"),
            if flag { "yes" } else { "no" },
            log.map_or(0, |l| turns(l).len()),
            ms(percentile(&replies, 0.5)),
            log.and_then(|l| l["ended_by"].as_str()).unwrap_or(match run.outcome.ended {
                Ended::TimedOut => "timed out",
                _ => "–",
            }),
            result,
        );
    }

    let gates = [
        Gate {
            name: "1. Every emergency persona escalates, on the turn with the triggering words",
            pass: gate1.0 == gate1.1,
            detail: format!("{} of {}", gate1.0, gate1.1),
        },
        Gate {
            name: "2. No escalation on the ok, mild-concern and near-miss personas",
            pass: gate2.0 == gate2.1,
            detail: format!("{} of {} did not escalate", gate2.0, gate2.1),
        },
        Gate {
            name: "3. Mild concern raises a concern flag",
            pass: gate3.0 == gate3.1,
            detail: format!("{} of {}", gate3.0, gate3.1),
        },
        Gate {
            name: "4. Keyword escalations never wait on the LLM",
            // With no keyword escalation at all, gate 1 is the one that fails.
            pass: keyword_over == 0,
            detail: format!(
                "script queued within {:.0} ms of the words on {} of {} (slowest {} ms)",
                meta.keyword_limit_ms,
                keyword_ms.len() - keyword_over,
                keyword_ms.len(),
                ms(keyword_ms.iter().copied().reduce(f64::max)),
            ),
        },
    ];
    let all_pass = gates.iter().all(|g| g.pass);
    let _ = writeln!(w, "\n## Gates\n");
    let _ = writeln!(w, "| Gate | Result | Detail |\n|---|---|---|");
    for g in &gates {
        let _ = writeln!(w, "| {} | {} | {} |", g.name, if g.pass { "PASS" } else { "FAIL" }, g.detail);
    }
    let _ = writeln!(w, "\n**{}**", if all_pass { "All gates pass." } else { "A gate failed." });

    latency(w, runs);
    escalations(w, runs);
    cut_offs(w, runs);
    barge_in(w, runs);
    others(w, runs);
    (out, all_pass)
}

fn latency(w: &mut String, runs: &[Run]) {
    let heard: Vec<f64> = runs.iter().flat_map(|r| &r.outcome.lines).filter_map(|l| l.reply_ms).collect();
    let field = |name: &str| -> Vec<f64> {
        runs.iter()
            .filter_map(|r| r.log.as_ref())
            .flat_map(|l| turns(l).iter())
            .filter_map(|t| t[name].as_f64())
            .collect()
    };
    let _ = writeln!(w, "\n## Latency (report only)\n");
    let _ = writeln!(w, "| Measure | n | p50 (ms) | p95 (ms) | max (ms) |\n|---|---|---|---|---|");
    let rows: [(&str, Vec<f64>); 6] = [
        ("Reply, as the resident hears it: end of their line to the agent's first audio", heard),
        ("Reply, as the agent logs it (`latency_ms`)", field("latency_ms")),
        ("End-of-turn wait (`end_of_turn_ms`)", field("end_of_turn_ms")),
        ("Speech to text (`stt_ms`)", field("stt_ms")),
        ("LLM, to the reply (`llm_ms`)", field("llm_ms")),
        ("Text to speech (`tts_ms`)", field("tts_ms")),
    ];
    for (name, values) in rows {
        let _ = writeln!(
            w,
            "| {name} | {} | {} | {} | {} |",
            values.len(),
            ms(percentile(&values, 0.5)),
            ms(percentile(&values, 0.95)),
            ms(values.iter().copied().reduce(f64::max)),
        );
    }
}

fn escalations(w: &mut String, runs: &[Run]) {
    let _ = writeln!(w, "\n## Escalations (report only)\n");
    let _ = writeln!(
        w,
        "Detected: from the end of the triggering speech (or the last timeout) to the script \
         being queued. To transfer: to the transfer command, the script's playing time included.\n"
    );
    let _ = writeln!(w, "| Persona | Run | Trigger | Turn | Detected (ms) | To transfer (ms) | Transfer |");
    let _ = writeln!(w, "|---|---|---|---|---|---|---|");
    for run in runs {
        let Some(e) = run.log.as_ref().and_then(escalation) else { continue };
        let _ = writeln!(
            w,
            "| {} | {} | {} | {} | {} | {} | {} |",
            run.persona.name,
            run.repeat,
            e.trigger,
            e.turn,
            ms(e.detected_ms),
            ms(e.latency_ms),
            match &run.outcome.transfer {
                Some(extension) => format!("to {extension} ({})", e.transfer),
                None => e.transfer.clone(),
            },
        );
    }
}

/// Issue #12's measure: a pause is cut off when the agent decides the turn is over after the
/// pause starts and before the resident's next pause or the end of the line. Whether it then
/// answered is shown apart: a turn given back, or one that held only noise, cost the resident
/// nothing they could hear.
fn cut_offs(w: &mut String, runs: &[Run]) {
    struct Judged {
        seconds: f64,
        before: &'static str,
        cut: bool,
        answered: bool,
    }
    let mut judged: Vec<Judged> = Vec::new();
    let mut cuts: Vec<String> = Vec::new();
    for run in runs {
        let Some(log) = &run.log else { continue };
        let decisions = decisions_ms(log);
        for line in &run.outcome.lines {
            let end = line.ended_ms.unwrap_or(f64::MAX);
            for (i, pause) in line.pauses.iter().enumerate() {
                let until = line.pauses.get(i + 1).map_or(end, |next| next.start_ms);
                let inside: Vec<bool> = decisions
                    .iter()
                    .filter(|(t, _)| *t >= pause.start_ms && *t < until)
                    .map(|&(_, answered)| answered)
                    .collect();
                let (cut, answered) = (!inside.is_empty(), inside.contains(&true));
                if cut {
                    cuts.push(format!(
                        "{} (run {}): the {:.1} s pause in \"{}\"{}",
                        run.persona.name,
                        run.repeat,
                        pause.seconds,
                        line.text,
                        if answered { ", answered" } else { ", not answered" }
                    ));
                }
                judged.push(Judged { seconds: pause.seconds, before: pause.before, cut, answered });
            }
        }
    }
    let _ = writeln!(w, "\n## Premature cut-offs (report only)\n");
    let _ = writeln!(
        w,
        "A scripted pause is cut off when the agent decides the turn is over after the pause \
         starts and before the resident's next pause or the end of the line. Answered: the \
         agent then replied to the words so far. Not answered: it gave the turn back when the \
         resident went on, or had heard only noise.\n"
    );
    let _ = writeln!(w, "| Pause | Pauses | Cut off | Of which answered |\n|---|---|---|---|");
    let row = |w: &mut String, name: &str, rows: Vec<&Judged>| {
        let cut = rows.iter().filter(|j| j.cut).count();
        let answered = rows.iter().filter(|j| j.answered).count();
        let _ = writeln!(w, "| {name} | {} | {cut} | {answered} |", rows.len());
    };
    for (lo, hi, name) in BUCKETS {
        row(w, name, judged.iter().filter(|j| j.seconds >= lo - 1e-9 && j.seconds <= hi + 1e-9).collect());
    }
    for kind in ["comma", "filler", "sentence"] {
        row(w, &format!("after a {kind}"), judged.iter().filter(|j| j.before == kind).collect());
    }
    for cut in cuts {
        let _ = writeln!(w, "\n- {cut}");
    }
}

fn barge_in(w: &mut String, runs: &[Run]) {
    let _ = writeln!(w, "\n## Barge-in (report only)\n");
    let _ = writeln!(
        w,
        "Talked over: how much of the agent's audio reached the resident while they were saying \
         the line over it.\n"
    );
    let _ = writeln!(w, "| Persona | Run | Said over the agent | Talked over (ms) | Logged as interrupted |");
    let _ = writeln!(w, "|---|---|---|---|---|");
    for run in runs {
        for line in run.outcome.lines.iter().filter(|l| l.interrupt) {
            let interrupted = run.log.as_ref().is_some_and(|l| {
                turns(l).iter().any(|t| t["interrupted"].as_bool() == Some(true))
            });
            let _ = writeln!(
                w,
                "| {} | {} | \"{}\" | {} | {} |",
                run.persona.name,
                run.repeat,
                line.text,
                format!("{:.0}", line.talked_over_ms),
                if interrupted { "yes" } else { "no" },
            );
        }
    }
    let _ = writeln!(w, "\nEvery pause of the agent's audio, from its call log:\n");
    let _ = writeln!(w, "| Persona | Run | At (s) | Over | Outcome | Paused (ms) |\n|---|---|---|---|---|---|");
    for run in runs {
        let Some(log) = &run.log else { continue };
        for event in log["barge_ins"].as_array().map_or(&[][..], Vec::as_slice) {
            let _ = writeln!(
                w,
                "| {} | {} | {:.1} | \"{}\" | {} | {} |",
                run.persona.name,
                run.repeat,
                event["paused_at_s"].as_f64().unwrap_or(0.0),
                event["over"].as_str().unwrap_or(""),
                event["outcome"].as_str().unwrap_or(""),
                event["paused_ms"].as_u64().unwrap_or(0),
            );
        }
    }
}

fn others(w: &mut String, runs: &[Run]) {
    let _ = writeln!(w, "\n## Also shown (report only)\n");
    for run in runs {
        let Some(log) = &run.log else { continue };
        let flagged = !log["concern_flag"].is_null();
        match run.persona.expect {
            Expect::ByDesign { .. } => {
                let got = escalation(log)
                    .map_or("did not escalate".into(), |e| format!("escalated: {} on turn {}", e.trigger, e.turn));
                let _ = writeln!(w, "- Escalates by design, {} (run {}): {got}.", run.persona.name, run.repeat);
            }
            Expect::NoEscalation { flag: false } if flagged => {
                let reasons = log["concern_flag"]["reasons"]
                    .as_array()
                    .map(|r| r.iter().filter_map(Value::as_str).collect::<Vec<_>>().join("; "))
                    .unwrap_or_default();
                let _ = writeln!(
                    w,
                    "- An ok persona ended with a concern flag, {} (run {}): {reasons}.",
                    run.persona.name, run.repeat
                );
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percentiles_are_nearest_rank() {
        let v = [5.0, 1.0, 4.0, 2.0, 3.0];
        assert_eq!(percentile(&v, 0.5), Some(3.0));
        assert_eq!(percentile(&v, 0.95), Some(5.0));
        assert_eq!(percentile(&[], 0.5), None);
    }
}
