//! PROTOTYPE, throwaway (issue #35): text-only check-ins against the real prompt and model,
//! with the same model playing the resident from a persona. Shows whether the conversation
//! follows the resident: no re-asked greeting, no word-for-word repeats, goodbye with end_call.
//! The keyword rule and the goodbye backstop run as on a call; nothing is spoken.
//!
//! cargo run --release -p agent --example PROTOTYPE_chat [persona number]

use agent::checklist::{Asking, Checklist, accepts_offer, wants_to_end};
use agent::conversation::{GREETING_INBOUND, says_goodbye};
use agent::escalation;
use agent::llm::{Llm, Message, Ollama, Role, SYSTEM_PROMPT};

const URL: &str = "http://127.0.0.1:11434";
const MODEL: &str = "qwen3:4b-instruct-2507-q4_K_M";

/// (name, persona, fixed lines for the first turns)
const PERSONAS: &[(&str, &str, &[&str])] = &[
    (
        "fine, asks back",
        "You are Rose, 81, feeling fine today. You had porridge for breakfast. No falls, no pain, \
         you need nothing. You are polite and like to ask how the other person is.",
        &["I'm feeling fine, how are you?"],
    ),
    (
        "fell yesterday",
        "You are Arthur, 84. Yesterday you slipped in the kitchen and fell. You got up by \
         yourself but your hip is sore today. You haven't eaten yet today because you didn't \
         feel like cooking. You don't mention everything at once.",
        &[],
    ),
    (
        "hard of hearing",
        "You are Edna, 88, hard of hearing. The first two times you are asked a question, you \
         say 'Sorry, what was that?' or 'Can you say that again?'. After that you hear \
         properly. Your facts, in your own words, only when asked: you feel fine, you have not \
         fallen, your knees are stiff as they have been for years, you had toast this morning, \
         you need nothing.",
        &[],
    ),
    (
        "chatty",
        "You are Walter, 79, chatty. You love talking about your garden tomatoes and your \
         grandson's football. You are fine, no falls, no pain, you had soup, but you only say \
         so when asked, and you often drift back to the garden.",
        &[],
    ),
    (
        "wants to go",
        "You are Mabel, 83, busy baking. You say you're fine, then say you need to go and ask \
         them to end the call.",
        &[],
    ),
    (
        "misheard",
        "You are Frank, 86, fine, no falls, no pain, you ate lunch, need nothing.",
        // whisper tiny.en's reading of "I think I'm fine, how are you?" on call 4cb911ac.
        &["Don't you think fine? How are you?"],
    ),
    (
        "test run 2, as tiny.en heard it",
        "You are Ruth, 80. You slipped yesterday and your hip is sore but bearable. If you are \
         offered a person, say no thanks. You had cereal this morning and need nothing.",
        // The live call 9d1ad4b7 of 2026-09-24, word for word as the agent heard it.
        &[
            "I sleep yesterday, my hip is soaring right now.",
            "I'm getting hurt right now, but... I mean I still bearable so I go to the hospital.",
        ],
    ),
    (
        "test run 2, as said, declines the offer",
        "You are Ruth, 80. You slipped yesterday and your hip is sore but bearable. If you are \
         offered a person, say no thanks, you're alright. You had cereal this morning and need \
         nothing.",
        &[
            "I slipped yesterday, my hip is sore right now.",
            "It's kind of hurting right now, but I mean, it's still bearable. Should I go to the hospital?",
        ],
    ),
    (
        "test run 2, as said, takes the offer",
        "You are Ruth, 80.",
        &[
            "I slipped yesterday, my hip is sore right now.",
            "It's kind of hurting right now, but I mean, it's still bearable. Should I go to the hospital?",
            "Yes, please.",
        ],
    ),
];

#[tokio::main]
async fn main() {
    if std::env::args().nth(1).as_deref() == Some("schema") {
        println!("{}", agent::llm::turn_schema());
        return;
    }
    if std::env::args().nth(1).as_deref() == Some("first-turn") {
        // The first LLM request of a call, as JSON, for timing it against Ollama directly.
        let heard = std::env::args().nth(2).unwrap();
        let body = serde_json::json!({
            "model": MODEL, "stream": false, "keep_alive": "1h",
            "format": agent::llm::turn_schema(),
            "messages": [
                Message::new(Role::System, SYSTEM_PROMPT),
                Message::new(Role::User, format!("{heard}\n\n{}", Checklist::default().note(false))),
            ],
        });
        println!("{body}");
        return;
    }
    let only: Option<usize> = std::env::args().nth(1).and_then(|a| a.parse().ok());
    let llm = Ollama::new(URL, MODEL);
    let http = reqwest::Client::new();
    for (i, (name, persona, first)) in PERSONAS.iter().enumerate() {
        if only.is_some_and(|n| n != i + 1) {
            continue;
        }
        println!("\n=== {} {name}", i + 1);
        println!("AGENT: {GREETING_INBOUND}");
        let mut agent = vec![Message::new(Role::System, SYSTEM_PROMPT)];
        let mut list = Checklist::default();
        let mut resident = vec![
            Message::new(
                Role::System,
                format!(
                    "{persona} You are answering a phone check-in call from an automated \
                     assistant. Reply with only what you say out loud, one or two short \
                     spoken sentences, no stage directions."
                ),
            ),
            Message::new(Role::User, GREETING_INBOUND),
        ];
        for turn in 1..=12 {
            let said = match first.get(turn - 1) {
                Some(line) => line.to_string(),
                None => chat(&http, &resident).await,
            };
            println!("RESIDENT: {said}");
            resident.push(Message::new(Role::Assistant, said.clone()));
            if let Some(hit) = escalation::check(&said, &said) {
                println!("  -> ESCALATE ({:?}: {:?})", hit.trigger, hit.phrase);
                break;
            }
            if list.last() == Some(Asking::OfferPerson) && accepts_offer(&said) {
                println!("  -> ESCALATE (AskedForPerson: took the offer)");
                break;
            }
            // As conversation.rs: the note goes with the words, the history keeps only the words.
            let mut messages = agent.clone();
            messages.push(Message::new(Role::User, format!("{said}\n\n{}", list.note(wants_to_end(&said)))));
            let reply = llm.turn(&messages).await.expect("LLM turn");
            let t = &reply.turn;
            let backstop = !t.end_call && (t.asking == Asking::Goodbye || says_goodbye(&t.reply));
            let recorded = list.record(&t.checklist, t.asking, &said);
            agent.push(Message::new(Role::User, said));
            agent.push(Message::new(Role::Assistant, reply.raw.clone()));
            println!(
                "AGENT: {}   [asking {:?}, {:?}{}{}{}, {} ms]{}",
                t.reply,
                t.asking,
                t.status,
                t.reason.as_deref().map(|r| format!(": {r}")).unwrap_or_default(),
                if t.end_call { ", end_call" } else { "" },
                if backstop { ", end_call ADDED BY BACKSTOP" } else { "" },
                reply.took.as_millis(),
                if recorded == Default::default() { String::new() } else { format!(" {recorded:?}") },
            );
            if t.end_call || backstop {
                println!("  summary: {}", t.summary);
                println!("  list: {}", serde_json::to_string(&list).unwrap());
                break;
            }
            resident.push(Message::new(Role::User, t.reply.clone()));
        }
    }
}

/// The resident's next line: plain chat, no schema.
async fn chat(http: &reqwest::Client, messages: &[Message]) -> String {
    let body = serde_json::json!({
        "model": MODEL, "messages": messages, "stream": false, "keep_alive": "1h",
    });
    let response: serde_json::Value = http
        .post(format!("{URL}/api/chat"))
        .json(&body)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    response["message"]["content"].as_str().unwrap().trim().to_string()
}
