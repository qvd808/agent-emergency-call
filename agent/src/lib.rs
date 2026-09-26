//! The live-call agent: AudioSocket framing and pacing, resampling, STT, the LLM client,
//! TTS, escalation, AMI call control, the call log and scheduled outbound check-ins.
//!
//! A library as well as a binary, so the eval's fake client runs the same AudioSocket code
//! path as a real call (issue #25).

pub mod audio;
pub mod call;
pub mod call_log;
pub mod checklist;
pub mod conversation;
pub mod end_of_turn;
pub mod escalation;
pub mod listen;
pub mod llm;
pub mod prosody;
pub mod schedule;
pub mod stt;
pub mod telephony;
pub mod tts;
