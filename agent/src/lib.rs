//! The live-call agent: AudioSocket framing and pacing, resampling, STT, the LLM client,
//! TTS, escalation, AMI call control and the call log.
//!
//! A library as well as a binary, so the eval's fake client runs the same AudioSocket code
//! path as a real call (issue #25).

pub mod audio;
pub mod listen;
pub mod stt;
pub mod telephony;
