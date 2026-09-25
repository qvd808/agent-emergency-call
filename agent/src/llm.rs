//! The LLM behind a provider interface, Ollama first (issue #25). Every turn comes back as a
//! [`Turn`], the object settled in issue #10 and given the check-in list in issue #35: what the
//! resident's words answered, the resident's status so far, what to say, and whether this is
//! the goodbye.

use std::time::{Duration, Instant};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tokio::sync::oneshot;

use crate::checklist::{Asking, Marks};

pub type Error = Box<dyn std::error::Error + Send + Sync>;

/// The check-in prompt: issue #10's first version, reworked around the check-in list in
/// issue #35.
pub const SYSTEM_PROMPT: &str = include_str!("../prompts/checkin.txt");

/// One agent turn, as the model returns it.
//
// The fields are in the order the model writes them. Ollama 0.20.7 generates an object's
// properties in the schema's `properties` order, not `required`'s (measured 2026-09-24:
// with the two orders reversed, the output followed `properties`). So the model first marks
// what the resident's words answered and judges the status, then says what it will ask,
// writes the reply, and only then decides whether that reply was the goodbye. Before,
// `serde_json` sorted the properties alphabetically, so `end_call` came first, decided before
// any reply was written (issue #35).
//
// The summary comes last because nothing said on this turn depends on it: the agent starts
// speaking once `end_call` is written ([`Head`]), while the model is still summing up. About
// three quarters of a turn's tokens came before the reply when the summary was second
// (measured on 2026-09-24, 128 tokens at 84 tokens/s).
//
// The `///` comments on the fields go into the schema as descriptions, which the model reads.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Turn {
    pub checklist: Marks,
    pub status: Status,
    /// Why, when the status isn't ok.
    // Required but nullable: Ollama forces only the properties listed in `required`, so an
    // optional field could be left out (issue #6).
    // `required` alone drops the null from the type, so it is put back.
    #[schemars(required, extend("type" = ["string", "null"]))]
    pub reason: Option<String>,
    /// What the reply asks about.
    pub asking: Asking,
    /// The words to say next.
    pub reply: String,
    /// True only on the goodbye turn.
    pub end_call: bool,
    /// The whole call so far, for a person reading it later.
    pub summary: String,
}

/// Everything in a [`Turn`] before the summary: all the agent needs to act on the turn.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Head {
    pub checklist: Marks,
    pub status: Status,
    pub reason: Option<String>,
    pub asking: Asking,
    pub reply: String,
    pub end_call: bool,
}

impl Head {
    /// The whole turn, for when the summary never came: the old summary stands in.
    pub fn into_turn(self, summary: String) -> Turn {
        Turn {
            checklist: self.checklist,
            status: self.status,
            reason: self.reason,
            asking: self.asking,
            reply: self.reply,
            end_call: self.end_call,
            summary,
        }
    }
}

/// The [`Head`] of a turn still being written, once the model has moved on to the summary:
/// the text up to the comma before the `"summary"` key, closed with a brace.
pub fn head_of(partial: &str) -> Option<Head> {
    let mut from = 0;
    while let Some(at) = partial[from..].find("\"summary\"") {
        let key = from + at;
        // A key follows a comma; inside a string the quote would follow a backslash.
        let before = partial[..key].trim_end();
        if let Some(object) = before.strip_suffix(',') {
            return serde_json::from_str(&format!("{object}}}")).ok();
        }
        from = key + 1;
    }
    None
}

// In rising order: a call's status is the highest any turn reached (issue #11).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Ok,
    Concern,
    Emergency,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    System,
    User,
    Assistant,
}

#[derive(Debug, Clone, Serialize)]
pub struct Message {
    pub role: Role,
    pub content: String,
}

impl Message {
    pub fn new(role: Role, content: impl Into<String>) -> Self {
        Message { role, content: content.into() }
    }
}

/// A turn and what it cost.
#[derive(Debug, Clone)]
pub struct Reply {
    pub turn: Turn,
    /// The model's output exactly as returned, to go back into the history.
    pub raw: String,
    /// Until the whole turn was written.
    pub took: Duration,
}

/// Anything that can take the conversation so far and return the next turn.
pub trait Llm: Send + Sync {
    fn turn(&self, messages: &[Message]) -> impl Future<Output = Result<Reply, Error>> + Send;

    /// As [`Llm::turn`], and sends the turn's [`Head`] to `head`, with the time it took, as
    /// soon as it is written. If the turn fails before that, nothing is sent.
    fn turn_streaming(
        &self,
        messages: &[Message],
        head: oneshot::Sender<(Head, Duration)>,
    ) -> impl Future<Output = Result<Reply, Error>> + Send;
}

/// The turn's JSON schema, with every subschema inlined: the converter in Ollama 0.20.7
/// resolves local `$ref`s (issue #6), but a flat schema doesn't depend on that.
pub fn turn_schema() -> serde_json::Value {
    let settings = schemars::generate::SchemaSettings::draft07().with(|s| {
        s.inline_subschemas = true;
    });
    settings.into_generator().into_root_schema_for::<Turn>().to_value()
}

pub struct Ollama {
    http: reqwest::Client,
    chat_url: String,
    model: String,
    schema: serde_json::Value,
}

impl Ollama {
    pub fn new(base_url: &str, model: &str) -> Self {
        Ollama {
            http: reqwest::Client::new(),
            chat_url: format!("{}/api/chat", base_url.trim_end_matches('/')),
            model: model.to_string(),
            schema: turn_schema(),
        }
    }

    fn request(&self, messages: &[Message], stream: bool) -> serde_json::Value {
        serde_json::json!({
            "model": self.model,
            "messages": messages,
            "stream": stream,
            // A JSON schema here is enforced as a grammar during sampling (issue #6).
            "format": self.schema,
            // Keeps the model loaded between calls; Ollama's default unloads it after 5 min
            // (docs/api.md:522 at v0.20.7), and reloading would slow the next first turn (inferred).
            "keep_alive": "1h",
        })
    }
}

#[derive(Deserialize)]
struct ChatResponse {
    message: ChatMessage,
}

#[derive(Deserialize)]
struct ChatMessage {
    content: String,
}

/// One line of a streamed reply (NDJSON): a piece of the content, until `done`.
#[derive(Deserialize)]
struct Chunk {
    message: Option<ChatMessage>,
    #[serde(default)]
    done: bool,
    error: Option<String>,
}

impl Llm for Ollama {
    async fn turn_streaming(
        &self,
        messages: &[Message],
        head: oneshot::Sender<(Head, Duration)>,
    ) -> Result<Reply, Error> {
        let started = Instant::now();
        let mut response = self
            .http
            .post(&self.chat_url)
            .json(&self.request(messages, true))
            .send()
            .await?
            .error_for_status()?;
        let mut head = Some(head);
        let mut raw = String::new();
        let mut pending: Vec<u8> = Vec::new();
        let mut done = false;
        while !done {
            let Some(bytes) = response.chunk().await? else { break };
            pending.extend_from_slice(&bytes);
            while let Some(end) = pending.iter().position(|&b| b == b'\n') {
                let line: Vec<u8> = pending.drain(..=end).collect();
                if line.iter().all(u8::is_ascii_whitespace) {
                    continue;
                }
                let chunk: Chunk = serde_json::from_slice(&line)?;
                if let Some(error) = chunk.error {
                    return Err(error.into());
                }
                if let Some(message) = chunk.message {
                    raw.push_str(&message.content);
                }
                done |= chunk.done;
                if head.is_some()
                    && let Some(early) = head_of(&raw)
                {
                    let _ = head.take().unwrap().send((early, started.elapsed()));
                }
            }
        }
        let turn = serde_json::from_str(&raw).map_err(|e| format!("bad turn {raw:?}: {e}"))?;
        Ok(Reply { turn, raw, took: started.elapsed() })
    }

    async fn turn(&self, messages: &[Message]) -> Result<Reply, Error> {
        let started = Instant::now();
        let response = self
            .http
            .post(&self.chat_url)
            .json(&self.request(messages, false))
            .send()
            .await?
            .error_for_status()?
            .json::<ChatResponse>()
            .await?;
        let raw = response.message.content;
        let turn = serde_json::from_str(&raw).map_err(|e| format!("bad turn {raw:?}: {e}"))?;
        Ok(Reply { turn, raw, took: started.elapsed() })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_schema_is_flat_and_requires_every_field() {
        let schema = turn_schema();
        let text = schema.to_string();
        assert!(!text.contains("$ref"), "{text}");
        let mut required: Vec<&str> =
            schema["required"].as_array().unwrap().iter().map(|v| v.as_str().unwrap()).collect();
        required.sort();
        assert_eq!(
            required,
            ["asking", "checklist", "end_call", "reason", "reply", "status", "summary"]
        );
        let order: Vec<&String> = schema["properties"].as_object().unwrap().keys().collect();
        assert_eq!(
            order,
            ["checklist", "status", "reason", "asking", "reply", "end_call", "summary"]
        );
        let marks = &schema["properties"]["checklist"];
        assert_eq!(marks["required"].as_array().unwrap().len(), 5);
        assert_eq!(marks["properties"]["pain"]["type"], serde_json::json!(["string", "null"]));
        assert_eq!(
            schema["properties"]["asking"]["enum"],
            serde_json::json!([
                "feeling", "falls", "pain", "eaten", "needs", "follow_up", "offer_person", "goodbye"
            ])
        );
        assert_eq!(
            schema["properties"]["status"]["enum"],
            serde_json::json!(["ok", "concern", "emergency"])
        );
        assert_eq!(schema["properties"]["reason"]["type"], serde_json::json!(["string", "null"]));
    }

    #[test]
    fn a_turn_parses_from_the_models_json() {
        let raw = r#"{"checklist":{"feeling":"fine","falls":null,"pain":null,"eaten":null,
                      "needs":null},"status":"ok","reason":null,"asking":"falls",
                      "reply":"Have you had a fall?","end_call":false,"summary":"Feels fine."}"#;
        let turn: Turn = serde_json::from_str(raw).unwrap();
        assert_eq!(turn.status, Status::Ok);
        assert_eq!(turn.reason, None);
    }

    #[test]
    fn the_head_is_ready_once_the_summary_starts() {
        // Spaced the way Ollama writes it.
        let partial = r#"{"checklist": {"feeling": "fine", "falls": null, "pain": null,
            "eaten": null, "needs": null} , "status": "ok" , "reason": null , "asking": "falls" ,
            "reply": "Good. Did you write a \"summary\" today? Any falls?" , "end_call": false ,
            "summ"#;
        assert_eq!(head_of(partial), None);
        let partial = format!("{}ary\": \"Feels fi", &partial[..partial.len()]);
        let head = head_of(&partial).unwrap();
        assert_eq!(head.reply, r#"Good. Did you write a "summary" today? Any falls?"#);
        assert_eq!(head.asking, Asking::Falls);
        assert!(!head.end_call);
    }

    #[test]
    fn no_head_before_end_call_is_written() {
        let partial = r#"{"checklist": {"feeling": "fine", "falls": null, "pain": null,
            "eaten": null, "needs": null}, "status": "ok", "reason": null, "asking": "falls",
            "reply": "Any falls?""#;
        assert_eq!(head_of(partial), None);
    }
}
