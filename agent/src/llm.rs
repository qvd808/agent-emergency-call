//! The LLM behind a provider interface, Ollama first (issue #25). Every turn comes back as a
//! [`Turn`], the object settled in issue #10: what to say, the resident's status so far, and
//! whether this is the goodbye.

use std::time::Duration;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub type Error = Box<dyn std::error::Error + Send + Sync>;

/// The check-in prompt from issue #10's prototype, accepted as the first version.
pub const SYSTEM_PROMPT: &str = include_str!("../prompts/checkin.txt");

/// One agent turn, as the model returns it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Turn {
    /// The words to say next.
    pub reply: String,
    pub status: Status,
    /// Why, when the status isn't `ok`.
    // Required but nullable: Ollama forces only the properties listed in `required`, so an
    // optional field could be left out (issue #6).
    // `required` alone drops the null from the type, so it is put back.
    #[schemars(required, extend("type" = ["string", "null"]))]
    pub reason: Option<String>,
    /// The whole call so far, for a person reading it later.
    pub summary: String,
    /// True only on the goodbye turn (issue #10).
    pub end_call: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
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
    pub took: Duration,
}

/// Anything that can take the conversation so far and return the next turn.
pub trait Llm: Send + Sync {
    fn turn(&self, messages: &[Message]) -> impl Future<Output = Result<Reply, Error>> + Send;
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

    fn request(&self, messages: &[Message]) -> serde_json::Value {
        serde_json::json!({
            "model": self.model,
            "messages": messages,
            "stream": false,
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

impl Llm for Ollama {
    async fn turn(&self, messages: &[Message]) -> Result<Reply, Error> {
        let started = std::time::Instant::now();
        let response = self
            .http
            .post(&self.chat_url)
            .json(&self.request(messages))
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
        assert_eq!(required, ["end_call", "reason", "reply", "status", "summary"]);
        assert_eq!(
            schema["properties"]["status"]["enum"],
            serde_json::json!(["ok", "concern", "emergency"])
        );
        assert_eq!(schema["properties"]["reason"]["type"], serde_json::json!(["string", "null"]));
    }

    #[test]
    fn a_turn_parses_from_the_models_json() {
        let raw = r#"{"reply":"Have you had a fall?","status":"ok","reason":null,
                      "summary":"Feels fine.","end_call":false}"#;
        let turn: Turn = serde_json::from_str(raw).unwrap();
        assert_eq!(turn.status, Status::Ok);
        assert_eq!(turn.reason, None);
    }
}
