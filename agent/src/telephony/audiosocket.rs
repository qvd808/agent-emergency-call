//! AudioSocket framing: Asterisk's TCP protocol between the dialplan's `AudioSocket()` and
//! this agent (issue #4).
//!
//! Every message is a 3-byte header, then the payload:
//!
//! ```text
//! +------+--------+--------+---------------+
//! | kind | len_hi | len_lo | payload[len]  |
//! +------+--------+--------+---------------+
//! ```
//!
//! The length is big-endian and counts the payload only (`res/res_audiosocket.c:255`, `:372`
//! at Asterisk 22.11.0). Audio samples inside the payload are 16-bit little-endian.
//!
//! This module only turns bytes into messages and back. It does no I/O, so every case can be
//! tested without a socket. Line references below are to `docs/research/audiosocket-protocol.md`
//! on branch `research/audiosocket-protocol`, and through it to the Asterisk source.

use std::fmt;

const HEADER_LEN: usize = 3;

const KIND_HANGUP: u8 = 0x00;
const KIND_UUID: u8 = 0x01;
const KIND_DTMF: u8 = 0x03;
const KIND_ERROR: u8 = 0xFF;

/// Audio kinds `0x10`-`0x18` and their sample rates (`include/asterisk/res_audiosocket.h:40-79`).
const AUDIO_KINDS: [(u8, u32); 9] = [
    (0x10, 8_000),
    (0x11, 12_000),
    (0x12, 16_000),
    (0x13, 24_000),
    (0x14, 32_000),
    (0x15, 44_100),
    (0x16, 48_000),
    (0x17, 96_000),
    (0x18, 192_000),
];

/// One message read from Asterisk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Message {
    /// Always the first message: the UUID the dialplan passed to `AudioSocket()`.
    Uuid(Uuid),
    /// 16-bit little-endian mono samples at `rate_hz`. The dialplan app always sends 8 kHz.
    Audio { rate_hz: u32, pcm: Vec<u8> },
    /// One ASCII DTMF digit.
    Dtmf(u8),
    /// Asterisk never sends this (issue #4); parsed so a stray one can't crash the reader.
    Hangup,
    /// Asterisk 22.11.0 never sends this either. The payload, if any, is an error code.
    Error(Vec<u8>),
    /// A kind this module doesn't know. Its payload has been skipped, so the stream stays in
    /// step.
    Unknown { kind: u8, len: usize },
}

/// The 16-byte UUID from the first message, in the dialplan string's byte order (RFC 4122
/// order; inferred in issue #4, since libuuid's `uuid_parse` wasn't fetched).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Uuid(pub [u8; 16]);

impl fmt::Display for Uuid {
    /// The dialplan's form: `8-4-4-4-12` lowercase hex digits.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, byte) in self.0.iter().enumerate() {
            if matches!(i, 4 | 6 | 8 | 10) {
                f.write_str("-")?;
            }
            write!(f, "{byte:02x}")?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecodeError {
    /// Asterisk always sends the UUID first (`app_audiosocket.c:165`).
    FirstMessageNotUuid { kind: u8 },
    /// A UUID message whose payload isn't 16 bytes.
    BadUuidLength(usize),
    /// A DTMF message whose payload isn't 1 byte.
    BadDtmfLength(usize),
    /// The connection closed partway through a message's payload.
    Truncated { expected: usize, got: usize },
}

impl fmt::Display for DecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::FirstMessageNotUuid { kind } => {
                write!(f, "first message has kind {kind:#04x}, expected the UUID")
            }
            Self::BadUuidLength(len) => write!(f, "UUID message is {len} bytes, expected 16"),
            Self::BadDtmfLength(len) => write!(f, "DTMF message is {len} bytes, expected 1"),
            Self::Truncated { expected, got } => write!(
                f,
                "connection closed {got} bytes into a {expected}-byte payload"
            ),
        }
    }
}

impl std::error::Error for DecodeError {}

/// Reads messages out of a byte stream. TCP has no message boundaries, so bytes go in as they
/// arrive with [`Decoder::push`] and whole messages come out of [`Decoder::next_message`].
#[derive(Debug, Default)]
pub struct Decoder {
    buf: Vec<u8>,
    seen_uuid: bool,
}

impl Decoder {
    pub fn new() -> Self {
        Self::default()
    }

    /// A decoder for Asterisk's end of the socket, reading what the agent sends, where no UUID
    /// comes first. Only a client standing in for Asterisk, such as the eval's, needs it.
    pub fn from_agent() -> Self {
        Self { buf: Vec::new(), seen_uuid: true }
    }

    pub fn push(&mut self, bytes: &[u8]) {
        self.buf.extend_from_slice(bytes);
    }

    /// The next whole message, or `None` until more bytes arrive.
    pub fn next_message(&mut self) -> Result<Option<Message>, DecodeError> {
        if self.buf.len() < HEADER_LEN {
            return Ok(None);
        }
        let kind = self.buf[0];
        let len = u16::from_be_bytes([self.buf[1], self.buf[2]]) as usize;
        if self.buf.len() < HEADER_LEN + len {
            return Ok(None);
        }
        let payload: Vec<u8> = self.buf.drain(..HEADER_LEN + len).skip(HEADER_LEN).collect();

        if !self.seen_uuid && kind != KIND_UUID {
            return Err(DecodeError::FirstMessageNotUuid { kind });
        }

        let message = match kind {
            KIND_UUID => {
                let bytes: [u8; 16] = payload
                    .try_into()
                    .map_err(|p: Vec<u8>| DecodeError::BadUuidLength(p.len()))?;
                self.seen_uuid = true;
                Message::Uuid(Uuid(bytes))
            }
            KIND_HANGUP => Message::Hangup,
            KIND_DTMF => match payload[..] {
                [digit] => Message::Dtmf(digit),
                _ => return Err(DecodeError::BadDtmfLength(payload.len())),
            },
            KIND_ERROR => Message::Error(payload),
            _ => match audio_rate(kind) {
                Some(rate_hz) => Message::Audio { rate_hz, pcm: payload },
                None => Message::Unknown { kind, len },
            },
        };
        Ok(Some(message))
    }

    /// Call once the connection has closed. Asterisk ends a call by closing the socket without
    /// a hangup message (`app_audiosocket.c:146`), so a close between messages, or inside a
    /// header, is a clean end. A close inside a payload is not.
    pub fn finish(&self) -> Result<(), DecodeError> {
        if self.buf.len() < HEADER_LEN {
            return Ok(());
        }
        let expected = u16::from_be_bytes([self.buf[1], self.buf[2]]) as usize;
        Err(DecodeError::Truncated { expected, got: self.buf.len() - HEADER_LEN })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EncodeError {
    /// Asterisk ends the call on a zero-length audio message (`res_audiosocket.c:373-376`).
    EmptyAudio,
    /// A sample is 2 bytes; Asterisk counts `length / 2` samples (`res_audiosocket.c:421`).
    OddLength(usize),
    /// The length field is 16 bits.
    TooLong(usize),
    /// Not one of the nine rates AudioSocket has a kind for.
    UnsupportedRate(u32),
}

impl fmt::Display for EncodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyAudio => f.write_str("audio payload is empty"),
            Self::OddLength(len) => write!(f, "audio payload is {len} bytes, not whole samples"),
            Self::TooLong(len) => write!(f, "payload is {len} bytes, over the 65535 limit"),
            Self::UnsupportedRate(rate) => write!(f, "AudioSocket has no kind for {rate} Hz"),
        }
    }
}

impl std::error::Error for EncodeError {}

/// One audio message, header and payload in a single buffer. Write it with one call:
/// Asterisk fails the call if the payload is not there within 5 ms of the header
/// (`res_audiosocket.c:389-404`).
pub fn encode_audio(rate_hz: u32, pcm: &[u8]) -> Result<Vec<u8>, EncodeError> {
    let kind = audio_kind(rate_hz).ok_or(EncodeError::UnsupportedRate(rate_hz))?;
    if pcm.is_empty() {
        return Err(EncodeError::EmptyAudio);
    }
    if !pcm.len().is_multiple_of(2) {
        return Err(EncodeError::OddLength(pcm.len()));
    }
    let len = u16::try_from(pcm.len()).map_err(|_| EncodeError::TooLong(pcm.len()))?;
    let mut message = Vec::with_capacity(HEADER_LEN + pcm.len());
    message.push(kind);
    message.extend_from_slice(&len.to_be_bytes());
    message.extend_from_slice(pcm);
    Ok(message)
}

/// Asks Asterisk to end the `AudioSocket()` app. The dialplan then carries on at its next
/// line (`app_audiosocket.c:200-203`).
pub fn encode_hangup() -> [u8; 3] {
    [KIND_HANGUP, 0, 0]
}

/// The first message Asterisk sends. Only a client standing in for Asterisk, such as the
/// eval's, sends it.
pub fn encode_uuid(uuid: &Uuid) -> [u8; 19] {
    let mut message = [0u8; 19];
    message[..3].copy_from_slice(&[KIND_UUID, 0, 16]);
    message[3..].copy_from_slice(&uuid.0);
    message
}

fn audio_kind(rate_hz: u32) -> Option<u8> {
    AUDIO_KINDS.iter().find(|&&(_, r)| r == rate_hz).map(|&(k, _)| k)
}

fn audio_rate(kind: u8) -> Option<u32> {
    AUDIO_KINDS.iter().find(|&&(k, _)| k == kind).map(|&(_, r)| r)
}

#[cfg(test)]
mod tests {
    //! The cases listed under "Consequences for the framing unit tests" in the research note;
    //! each test names its case number.

    use super::*;

    const UUID_BYTES: [u8; 16] = [
        0x6f, 0x9c, 0x1d, 0x2e, 0x3a, 0x4b, 0x4c, 0x5d, 0x8e, 0x6f, 0x70, 0x81, 0x92, 0xa3, 0xb4,
        0xc5,
    ];

    fn uuid_message() -> Vec<u8> {
        let mut m = vec![0x01, 0x00, 0x10];
        m.extend_from_slice(&UUID_BYTES);
        m
    }

    /// A decoder that has already read the UUID.
    fn started() -> Decoder {
        let mut d = Decoder::new();
        d.push(&uuid_message());
        d.next_message().unwrap().unwrap();
        d
    }

    fn message(kind: u8, payload: &[u8]) -> Vec<u8> {
        let mut m = vec![kind];
        m.extend_from_slice(&(payload.len() as u16).to_be_bytes());
        m.extend_from_slice(payload);
        m
    }

    #[test]
    fn case_1_uuid_decodes_to_the_dialplan_string() {
        let mut d = Decoder::new();
        d.push(&uuid_message());
        let Some(Message::Uuid(uuid)) = d.next_message().unwrap() else { panic!() };
        assert_eq!(uuid.to_string(), "6f9c1d2e-3a4b-4c5d-8e6f-708192a3b4c5");
    }

    #[test]
    fn case_2_length_is_big_endian() {
        let mut d = started();
        let pcm: Vec<u8> = (0..320).map(|i| i as u8).collect();
        d.push(&[0x10, 0x01, 0x40]);
        d.push(&pcm);
        assert_eq!(
            d.next_message().unwrap(),
            Some(Message::Audio { rate_hz: 8_000, pcm })
        );
        assert_eq!(d.next_message().unwrap(), None);
    }

    #[test]
    fn case_3_audio_of_any_size_decodes_unchanged() {
        for size in [160, 480, 2] {
            let mut d = started();
            let pcm = vec![7u8; size];
            d.push(&message(0x10, &pcm));
            assert_eq!(
                d.next_message().unwrap(),
                Some(Message::Audio { rate_hz: 8_000, pcm })
            );
        }
    }

    #[test]
    fn case_4_dtmf() {
        let mut d = started();
        d.push(&[0x03, 0x00, 0x01, 0x35]);
        assert_eq!(d.next_message().unwrap(), Some(Message::Dtmf(b'5')));
    }

    #[test]
    fn case_5_16_khz_audio() {
        let mut d = started();
        d.push(&message(0x12, &[1, 2, 3, 4]));
        assert_eq!(
            d.next_message().unwrap(),
            Some(Message::Audio { rate_hz: 16_000, pcm: vec![1, 2, 3, 4] })
        );
    }

    #[test]
    fn case_6_error_with_and_without_a_code() {
        let mut d = started();
        d.push(&[0xFF, 0x00, 0x00, 0xFF, 0x00, 0x01, 0x01]);
        assert_eq!(d.next_message().unwrap(), Some(Message::Error(vec![])));
        assert_eq!(d.next_message().unwrap(), Some(Message::Error(vec![0x01])));
    }

    #[test]
    fn case_7_a_message_split_across_reads() {
        let mut d = started();
        let pcm = vec![9u8; 320];
        let bytes = message(0x10, &pcm);
        d.push(&bytes[..1]);
        assert_eq!(d.next_message().unwrap(), None);
        d.push(&bytes[1..3]);
        assert_eq!(d.next_message().unwrap(), None);
        d.push(&bytes[3..100]);
        assert_eq!(d.next_message().unwrap(), None);
        d.push(&bytes[100..]);
        assert_eq!(
            d.next_message().unwrap(),
            Some(Message::Audio { rate_hz: 8_000, pcm })
        );
    }

    #[test]
    fn case_8_close_between_messages_or_mid_header_is_clean() {
        assert_eq!(Decoder::new().finish(), Ok(()));
        assert_eq!(started().finish(), Ok(()));
        let mut d = started();
        d.push(&[0x10, 0x01]);
        assert_eq!(d.next_message().unwrap(), None);
        assert_eq!(d.finish(), Ok(()));
    }

    #[test]
    fn case_9_close_mid_payload_is_truncated() {
        let mut d = started();
        d.push(&[0x10, 0x01, 0x40]);
        d.push(&[0u8; 100]);
        assert_eq!(d.next_message().unwrap(), None);
        assert_eq!(d.finish(), Err(DecodeError::Truncated { expected: 320, got: 100 }));
    }

    #[test]
    fn case_10_first_message_must_be_the_uuid() {
        let mut d = Decoder::new();
        d.push(&message(0x10, &[0, 0]));
        assert_eq!(
            d.next_message(),
            Err(DecodeError::FirstMessageNotUuid { kind: 0x10 })
        );
    }

    #[test]
    fn case_10_uuid_must_be_16_bytes() {
        let mut d = Decoder::new();
        d.push(&message(0x01, &[0; 15]));
        assert_eq!(d.next_message(), Err(DecodeError::BadUuidLength(15)));
    }

    #[test]
    fn case_11_unknown_kind_is_skipped_without_losing_step() {
        let mut d = started();
        d.push(&message(0x02, &[1, 2, 3]));
        d.push(&message(0x20, &[]));
        d.push(&message(0x10, &[5, 6]));
        assert_eq!(d.next_message().unwrap(), Some(Message::Unknown { kind: 0x02, len: 3 }));
        assert_eq!(d.next_message().unwrap(), Some(Message::Unknown { kind: 0x20, len: 0 }));
        assert_eq!(
            d.next_message().unwrap(),
            Some(Message::Audio { rate_hz: 8_000, pcm: vec![5, 6] })
        );
    }

    #[test]
    fn case_12_encode_8_khz_frame() {
        let pcm = vec![3u8; 320];
        let m = encode_audio(8_000, &pcm).unwrap();
        assert_eq!(m.len(), 323);
        assert_eq!(m[..3], [0x10, 0x01, 0x40]);
        assert_eq!(m[3..], pcm[..]);
    }

    #[test]
    fn case_13_encode_16_khz_frame() {
        let m = encode_audio(16_000, &[0u8; 640]).unwrap();
        assert_eq!(m[..3], [0x12, 0x02, 0x80]);
        assert_eq!(m.len(), 643);
    }

    #[test]
    fn case_14_encode_hangup() {
        assert_eq!(encode_hangup(), [0x00, 0x00, 0x00]);
    }

    #[test]
    fn case_15_empty_audio_is_refused() {
        assert_eq!(encode_audio(8_000, &[]), Err(EncodeError::EmptyAudio));
    }

    #[test]
    fn case_16_odd_length_audio_is_refused() {
        assert_eq!(encode_audio(8_000, &[0; 3]), Err(EncodeError::OddLength(3)));
    }

    #[test]
    fn case_17_length_limit() {
        // 65535 is odd, so the longest valid audio payload is 65534 bytes.
        assert!(encode_audio(8_000, &vec![0; 65_534]).is_ok());
        assert_eq!(
            encode_audio(8_000, &vec![0; 65_536]),
            Err(EncodeError::TooLong(65_536))
        );
    }

    #[test]
    fn case_18_encoder_only_emits_audio_kinds() {
        for (kind, rate) in AUDIO_KINDS {
            assert_eq!(encode_audio(rate, &[0, 0]).unwrap()[0], kind);
            assert!((0x10..=0x18).contains(&kind));
        }
        assert_eq!(encode_audio(11_025, &[0, 0]), Err(EncodeError::UnsupportedRate(11_025)));
    }

    #[test]
    fn encoded_audio_decodes_back() {
        let pcm: Vec<u8> = (0..=255).collect();
        let mut d = started();
        d.push(&encode_audio(16_000, &pcm).unwrap());
        assert_eq!(
            d.next_message().unwrap(),
            Some(Message::Audio { rate_hz: 16_000, pcm })
        );
    }
}
