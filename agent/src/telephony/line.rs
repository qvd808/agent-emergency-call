//! The AudioSocket side of [`Media`]: one task per call that owns the socket, converts the
//! line's 8 kHz audio to the core's 16 kHz and back, and paces what goes out.
//!
//! The task runs on one 20 ms clock. Every tick it writes exactly one frame to Asterisk: the
//! next 20 ms of queued audio, or silence when nothing is queued. Audio therefore never goes
//! out in a burst (issue #13), and marks can be reported by counting frames.
//!
//! Sending silence between the agent's sentences, instead of nothing, keeps a steady stream
//! of RTP going to the phone. That this matters to Linphone's jitter buffer is inferred, not
//! tested; the echo test sent frames only when the phone did.

use std::collections::VecDeque;
use std::io::ErrorKind;
use std::time::Duration;

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio::time::{Instant, MissedTickBehavior};

use super::audiosocket::{Decoder, Message, Uuid, encode_audio, encode_hangup};
use super::{CallControl, Cleared, Command, Frame, MarkId, Media, Speaker, core_duration};
use crate::audio::{
    CORE_FRAME, FRAME_MS, FrameResampler, LINE_FRAME, LINE_RATE_HZ, samples_from_le_bytes,
    samples_to_le_bytes,
};

const TICK: Duration = Duration::from_millis(FRAME_MS as u64);

/// After this long with no audio from Asterisk, the line fills in one silent frame per tick
/// until audio returns, so media time keeps moving (issue #13). Long enough that ordinary
/// network jitter doesn't trigger it. If late audio then arrives in a burst, media time runs
/// ahead of the wall clock by the filled amount (inferred; nobody has seen Asterisk stop
/// sending).
const GAP_FILL_AFTER: Duration = Duration::from_millis(100);

/// Frames the core hasn't read yet. At 20 ms each, 50 frames is a second of backlog.
const INBOUND_BUFFER: usize = 50;

pub type Error = Box<dyn std::error::Error + Send + Sync>;

/// What happened on the line, for the call log and for checking the pacing.
#[derive(Debug, Default, Clone)]
pub struct LineStats {
    pub frames_in: u64,
    /// Silent frames filled in because Asterisk sent nothing for a while.
    pub frames_filled: u64,
    pub frames_out: u64,
    /// The shortest and longest time between two frames written to Asterisk. Real-time
    /// pacing keeps both near 20 ms; a burst shows up as a shortest gap near zero.
    pub min_write_gap: Option<Duration>,
    pub max_write_gap: Option<Duration>,
    /// Audio messages at a rate other than 8 kHz, which were dropped.
    pub dropped_messages: u64,
    /// Frames dropped because the core had a second of audio it hadn't read yet.
    pub frames_dropped: u64,
}

/// Reads the UUID Asterisk sends first, then starts the call's line task. The task ends
/// when either side hangs up, and returns what it counted.
pub async fn accept<S>(
    mut stream: S,
) -> Result<(Uuid, Media, CallControl, JoinHandle<Result<LineStats, Error>>), Error>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let mut decoder = Decoder::new();
    let mut buf = [0u8; 4096];
    let uuid = loop {
        if let Some(message) = decoder.next_message()? {
            match message {
                Message::Uuid(uuid) => break uuid,
                // The decoder refuses anything else before the UUID.
                other => unreachable!("decoder returned {other:?} before the UUID"),
            }
        }
        let n = stream.read(&mut buf).await?;
        if n == 0 {
            decoder.finish()?;
            return Err("Asterisk closed the socket before sending the UUID".into());
        }
        decoder.push(&buf[..n]);
    };

    let (frames_tx, frames) = mpsc::channel(INBOUND_BUFFER);
    let (played_tx, played) = mpsc::unbounded_channel();
    let (commands_tx, commands) = mpsc::unbounded_channel();
    let media = Media { frames, played, speaker: Speaker::new(commands_tx.clone()) };
    let control = CallControl::new(commands_tx);
    let task = tokio::spawn(run(stream, decoder, frames_tx, played_tx, commands));
    Ok((uuid, media, control, task))
}

async fn run<S>(
    mut stream: S,
    mut decoder: Decoder,
    frames: mpsc::Sender<Frame>,
    played: mpsc::UnboundedSender<MarkId>,
    mut commands: mpsc::UnboundedReceiver<Command>,
) -> Result<LineStats, Error>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let mut inbox = Inbox::new();
    let mut outbox = Outbox::new();
    let mut stats = LineStats::default();
    let mut buf = [0u8; 4096];

    let mut clock = tokio::time::interval(TICK);
    // A late tick must not be followed by catch-up ticks back to back: that would be a
    // burst. `Delay` restarts the 20 ms count from the late tick instead (issue #24).
    clock.set_missed_tick_behavior(MissedTickBehavior::Delay);
    let mut last_audio_in = Instant::now();
    let mut last_write: Option<Instant> = None;

    loop {
        tokio::select! {
            // Reads first, so a closed socket is noticed before the next write.
            biased;
            read = stream.read(&mut buf) => {
                let n = read?;
                if n == 0 {
                    decoder.finish()?;
                    return Ok(stats);
                }
                decoder.push(&buf[..n]);
                while let Some(message) = decoder.next_message()? {
                    match message {
                        Message::Audio { rate_hz: LINE_RATE_HZ, pcm } => {
                            last_audio_in = Instant::now();
                            for frame in inbox.push(&pcm) {
                                stats.frames_in += 1;
                                deliver(&frames, frame, &mut stats);
                            }
                        }
                        Message::Audio { rate_hz, .. } => {
                            if stats.dropped_messages == 0 {
                                eprintln!("agent: dropping audio at {rate_hz} Hz, expected 8 kHz");
                            }
                            stats.dropped_messages += 1;
                        }
                        // DTMF doesn't cross the seam in stage 1 (issue #13).
                        other => eprintln!("agent: ignored {other:?}"),
                    }
                }
            }
            tick = clock.tick() => {
                let (frame, marks) = outbox.next_frame();
                // One write per message: Asterisk wants the payload within 5 ms of the
                // header (issue #4).
                let message = encode_audio(LINE_RATE_HZ, &samples_to_le_bytes(&frame))?;
                match stream.write_all(&message).await {
                    Ok(()) => {}
                    // Asterisk hung up between our last read and this write.
                    Err(e) if matches!(e.kind(), ErrorKind::BrokenPipe | ErrorKind::ConnectionReset) => {
                        return Ok(stats);
                    }
                    Err(e) => return Err(e.into()),
                }
                stats.frames_out += 1;
                // The time the write finished, not the tick's scheduled time, which is
                // always exactly 20 ms after the last one.
                let written = Instant::now();
                if let Some(last) = last_write {
                    let gap = written - last;
                    stats.min_write_gap = Some(stats.min_write_gap.map_or(gap, |g| g.min(gap)));
                    stats.max_write_gap = Some(stats.max_write_gap.map_or(gap, |g| g.max(gap)));
                }
                last_write = Some(written);
                for mark in marks {
                    let _ = played.send(mark);
                }

                if tick.saturating_duration_since(last_audio_in) >= GAP_FILL_AFTER {
                    stats.frames_filled += 1;
                    deliver(&frames, inbox.silence(), &mut stats);
                }
            }
            Some(command) = commands.recv() => match command {
                Command::Play(audio) => outbox.play(audio),
                Command::Mark(id) => outbox.mark(id),
                Command::Clear(reply) => { let _ = reply.send(outbox.clear()); }
                Command::Pause => outbox.paused = true,
                Command::Resume => outbox.paused = false,
                // Asterisk ends the AudioSocket() app, and the dialplan's next line hangs
                // up (asterisk/config/extensions.conf, extension 3100).
                Command::Hangup => {
                    match stream.write_all(&encode_hangup()).await {
                        Ok(()) => {}
                        Err(e) if matches!(e.kind(), ErrorKind::BrokenPipe | ErrorKind::ConnectionReset) => {}
                        Err(e) => return Err(e.into()),
                    }
                    return Ok(stats);
                }
            },
        }
    }
}

/// Hands a frame to the core without waiting: waiting would stall the 20 ms clock and with
/// it the agent's outgoing audio. A core that has stopped reading loses frames instead.
fn deliver(frames: &mpsc::Sender<Frame>, frame: Frame, stats: &mut LineStats) {
    match frames.try_send(frame) {
        Ok(()) | Err(mpsc::error::TrySendError::Closed(_)) => {}
        Err(mpsc::error::TrySendError::Full(_)) => {
            if stats.frames_dropped == 0 {
                eprintln!("agent: the core is a second behind; dropping the resident's audio");
            }
            stats.frames_dropped += 1;
        }
    }
}

/// Turns Asterisk's audio, in messages of any size, into exact 20 ms frames at 16 kHz.
/// AudioSocket messages are usually 320 bytes, but that isn't guaranteed (issue #4).
struct Inbox {
    /// Line samples not yet making up a whole frame.
    pending: Vec<i16>,
    /// A trailing odd byte from the last message, waiting for its other half.
    odd_byte: Option<u8>,
    up: FrameResampler,
}

impl Inbox {
    fn new() -> Self {
        Self {
            pending: Vec::with_capacity(LINE_FRAME * 2),
            odd_byte: None,
            up: FrameResampler::upsampler(),
        }
    }

    fn push(&mut self, pcm: &[u8]) -> Vec<Frame> {
        let mut bytes = Vec::with_capacity(pcm.len() + 1);
        bytes.extend(self.odd_byte.take());
        bytes.extend_from_slice(pcm);
        if bytes.len() % 2 == 1 {
            self.odd_byte = bytes.pop();
        }
        self.pending.extend(samples_from_le_bytes(&bytes));

        let whole = self.pending.len() / LINE_FRAME * LINE_FRAME;
        let frames = self.pending[..whole]
            .chunks_exact(LINE_FRAME)
            .map(|f| self.up.process(f))
            .map(to_frame)
            .collect();
        self.pending.drain(..whole);
        frames
    }

    /// A silent frame for a gap. It goes through the resampler like real audio, so the
    /// filter's state stays continuous when audio comes back.
    fn silence(&mut self) -> Frame {
        to_frame(self.up.process(&[0; LINE_FRAME]))
    }
}

fn to_frame(samples: Vec<i16>) -> Frame {
    samples.try_into().expect("the upsampler returns one 16 kHz frame")
}

enum Queued {
    Audio(VecDeque<i16>),
    Mark(MarkId),
}

/// The agent's queued audio and marks, taken 20 ms at a time. No clock in here: the line
/// task calls [`Outbox::next_frame`] once per tick, so this can be tested without waiting.
struct Outbox {
    queue: VecDeque<Queued>,
    queued_samples: usize,
    down: FrameResampler,
    /// Marks whose audio has been fed to the resampler but, because of its delay, not all
    /// written yet. Each counts down the ticks it still has to wait.
    in_flight: VecDeque<(MarkId, usize)>,
    /// Ticks a sample spends inside the resampler before it reaches the line.
    delay_ticks: usize,
    /// While paused, the line gets silence and the queue waits where it stopped (barge-in,
    /// issue #19).
    paused: bool,
}

impl Outbox {
    fn new() -> Self {
        let down = FrameResampler::downsampler();
        let delay_ticks = down.delay().div_ceil(LINE_FRAME);
        Self {
            queue: VecDeque::new(),
            queued_samples: 0,
            down,
            in_flight: VecDeque::new(),
            delay_ticks,
            paused: false,
        }
    }

    fn play(&mut self, audio: Vec<i16>) {
        if !audio.is_empty() {
            self.queued_samples += audio.len();
            self.queue.push_back(Queued::Audio(audio.into()));
        }
    }

    fn mark(&mut self, id: MarkId) {
        self.queue.push_back(Queued::Mark(id));
    }

    /// Drops everything queued, and ends a pause: what is queued next plays at once.
    fn clear(&mut self) -> Cleared {
        self.paused = false;
        let dropped_marks = self
            .queue
            .drain(..)
            .filter_map(|q| match q {
                Queued::Mark(id) => Some(id),
                Queued::Audio(_) => None,
            })
            .collect();
        let dropped = core_duration(std::mem::take(&mut self.queued_samples));
        Cleared { dropped, dropped_marks }
    }

    /// The next 20 ms for the line, at 8 kHz, and the marks whose audio has now all been
    /// written once this frame is.
    fn next_frame(&mut self) -> (Vec<i16>, Vec<MarkId>) {
        let mut input = Vec::with_capacity(CORE_FRAME);
        // Fill one frame of input. Marks met on the way are taken too, and so is any mark
        // sitting right after the frame's last sample, so it isn't held back a tick. Paused,
        // nothing is taken.
        while !self.paused {
            match self.queue.front_mut() {
                Some(Queued::Mark(id)) => {
                    self.in_flight.push_back((*id, self.delay_ticks));
                    self.queue.pop_front();
                }
                Some(Queued::Audio(audio)) if input.len() < CORE_FRAME => {
                    let take = audio.len().min(CORE_FRAME - input.len());
                    input.extend(audio.drain(..take));
                    self.queued_samples -= take;
                    if audio.is_empty() {
                        self.queue.pop_front();
                    }
                }
                _ => break,
            }
        }
        // Short of a frame (the end of the agent's audio, or nothing queued): pad with
        // silence. Idle ticks also flush the resampler's delay.
        input.resize(CORE_FRAME, 0);
        let frame = self.down.process(&input);

        let mut played = Vec::new();
        self.in_flight.retain_mut(|(id, ticks_left)| {
            if *ticks_left == 0 {
                played.push(*id);
                false
            } else {
                *ticks_left -= 1;
                true
            }
        });
        (frame, played)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::telephony::audiosocket::encode_audio as encode;
    use tokio::io::{DuplexStream, duplex};

    const UUID_MESSAGE: [u8; 19] = [
        0x01, 0x00, 0x10, 0x6f, 0x9c, 0x1d, 0x2e, 0x3a, 0x4b, 0x4c, 0x5d, 0x8e, 0x6f, 0x70, 0x81,
        0x92, 0xa3, 0xb4, 0xc5,
    ];

    fn drain(outbox: &mut Outbox, ticks: usize) -> Vec<(usize, MarkId)> {
        (0..ticks).flat_map(|t| outbox.next_frame().1.into_iter().map(move |m| (t, m))).collect()
    }

    #[test]
    fn idle_outbox_sends_silence() {
        let mut outbox = Outbox::new();
        for _ in 0..3 {
            let (frame, marks) = outbox.next_frame();
            assert_eq!(frame, vec![0; LINE_FRAME]);
            assert!(marks.is_empty());
        }
    }

    #[test]
    fn audio_goes_out_one_frame_per_tick() {
        let mut outbox = Outbox::new();
        outbox.play(vec![1000; CORE_FRAME * 3]);
        for _ in 0..3 {
            outbox.next_frame();
        }
        assert_eq!(outbox.queued_samples, 0);
        assert!(outbox.queue.is_empty());
    }

    /// A mark after 3 frames of audio is reported on the tick its audio's last sample leaves
    /// the resampler, never earlier.
    #[test]
    fn a_mark_is_reported_once_its_audio_is_written() {
        let mut outbox = Outbox::new();
        let delay = outbox.delay_ticks;
        outbox.play(vec![1000; CORE_FRAME * 3]);
        outbox.mark(7);
        // Tick 2 takes the last audio frame and the mark behind it; the resampler holds
        // `delay` more ticks.
        assert_eq!(drain(&mut outbox, 10), vec![(2 + delay, 7)]);
    }

    #[test]
    fn a_mark_mid_frame_waits_for_the_frame() {
        let mut outbox = Outbox::new();
        let delay = outbox.delay_ticks;
        outbox.play(vec![1000; 100]);
        outbox.mark(1);
        outbox.play(vec![1000; 100]);
        outbox.mark(2);
        assert_eq!(drain(&mut outbox, 5), vec![(delay, 1), (delay, 2)]);
    }

    #[test]
    fn a_mark_with_nothing_before_it_is_reported_after_the_delay() {
        let mut outbox = Outbox::new();
        let delay = outbox.delay_ticks;
        outbox.mark(3);
        assert_eq!(drain(&mut outbox, 5), vec![(delay, 3)]);
    }

    #[test]
    fn clear_drops_unsent_audio_and_its_marks() {
        let mut outbox = Outbox::new();
        let delay = outbox.delay_ticks;
        outbox.play(vec![1000; CORE_FRAME]);
        outbox.mark(1);
        outbox.play(vec![1000; CORE_FRAME * 2]);
        outbox.mark(2);
        // The first frame and mark 1 go out; mark 2 and two frames are still queued.
        let first = drain(&mut outbox, 1);
        let cleared = outbox.clear();
        assert_eq!(cleared, Cleared { dropped: Duration::from_millis(40), dropped_marks: vec![2] });
        // Mark 1 was already in the resampler, so it's still reported.
        let mut all = first;
        all.extend(drain(&mut outbox, 5).into_iter().map(|(t, m)| (t + 1, m)));
        assert_eq!(all, vec![(delay, 1)]);
        // Only silence from here on, once the resampler has flushed.
        assert_eq!(outbox.next_frame().0, vec![0; LINE_FRAME]);
    }

    #[test]
    fn a_pause_holds_the_audio_and_its_marks_where_they_stopped() {
        let mut outbox = Outbox::new();
        let delay = outbox.delay_ticks;
        outbox.play(vec![1000; CORE_FRAME * 3]);
        outbox.mark(1);
        outbox.next_frame();
        outbox.paused = true;
        // Paused: silence once the resampler has flushed, and the mark waits.
        let paused = drain(&mut outbox, 10);
        assert!(paused.is_empty(), "{paused:?}");
        assert_eq!(outbox.next_frame().0, vec![0; LINE_FRAME]);
        assert_eq!(outbox.queued_samples, CORE_FRAME * 2);
        // Resumed: the two frames left, then the mark after the resampler's delay.
        outbox.paused = false;
        assert_eq!(drain(&mut outbox, 10), vec![(1 + delay, 1)]);
    }

    #[test]
    fn clearing_ends_a_pause() {
        let mut outbox = Outbox::new();
        outbox.play(vec![1000; CORE_FRAME]);
        outbox.paused = true;
        outbox.clear();
        assert!(!outbox.paused);
    }

    #[test]
    fn inbox_rechunks_any_message_size_into_20_ms_frames() {
        let mut inbox = Inbox::new();
        // 1.5 frames, then an odd byte split across two messages, then the rest.
        assert_eq!(inbox.push(&[0u8; 480]).len(), 1);
        assert_eq!(inbox.push(&[0u8; 1]).len(), 0);
        assert_eq!(inbox.push(&[0u8; 159]).len(), 1);
        assert!(inbox.pending.is_empty() && inbox.odd_byte.is_none());
    }

    async fn started_line() -> (DuplexStream, Media, JoinHandle<Result<LineStats, Error>>) {
        let (mut asterisk, agent) = duplex(1 << 16);
        asterisk.write_all(&UUID_MESSAGE).await.unwrap();
        let (uuid, media, _control, task) = accept(agent).await.unwrap();
        assert_eq!(uuid.to_string(), "6f9c1d2e-3a4b-4c5d-8e6f-708192a3b4c5");
        (asterisk, media, task)
    }

    /// Reads one audio message from the agent, as Asterisk would.
    async fn read_frame(asterisk: &mut DuplexStream) -> Vec<u8> {
        let mut header = [0u8; 3];
        asterisk.read_exact(&mut header).await.unwrap();
        assert_eq!(header[0], 0x10, "8 kHz audio");
        let mut payload = vec![0u8; u16::from_be_bytes([header[1], header[2]]) as usize];
        asterisk.read_exact(&mut payload).await.unwrap();
        payload
    }

    /// A second of audio queued at once still reaches Asterisk one 320-byte frame every
    /// 20 ms. Time is tokio's paused clock, so the test runs instantly and exactly.
    #[tokio::test(start_paused = true)]
    async fn a_second_of_audio_is_paced_not_burst() {
        let (mut asterisk, media, task) = started_line().await;
        let start = Instant::now();
        media.speaker.play(vec![1000; CORE_FRAME * 50]);
        let mut arrivals = Vec::new();
        for _ in 0..60 {
            assert_eq!(read_frame(&mut asterisk).await.len(), 320);
            arrivals.push(Instant::now() - start);
        }
        for pair in arrivals.windows(2) {
            assert_eq!(pair[1] - pair[0], TICK, "frames at {:?} and {:?}", pair[0], pair[1]);
        }
        drop(asterisk);
        let stats = task.await.unwrap().unwrap();
        assert_eq!(stats.min_write_gap, Some(TICK));
        assert_eq!(stats.max_write_gap, Some(TICK));
    }

    #[tokio::test(start_paused = true)]
    async fn marks_arrive_after_their_audio() {
        let (mut asterisk, mut media, _task) = started_line().await;
        media.speaker.play(vec![1000; CORE_FRAME * 10]);
        media.speaker.mark(42);
        // When the first frame carrying audio reached Asterisk. The clock may have sent a
        // silent frame before the audio was queued.
        let (first_audio_tx, first_audio) = tokio::sync::oneshot::channel();
        tokio::spawn(async move {
            let mut first_audio_tx = Some(first_audio_tx);
            loop {
                let frame = read_frame(&mut asterisk).await;
                if frame.iter().any(|&b| b != 0)
                    && let Some(tx) = first_audio_tx.take()
                {
                    let _ = tx.send(Instant::now());
                }
            }
        });
        assert_eq!(media.played.recv().await, Some(42));
        let reported = Instant::now();
        // The mark follows the tenth audio frame, plus the resampler's delay.
        let expected = TICK * (9 + Outbox::new().delay_ticks as u32);
        assert_eq!(reported - first_audio.await.unwrap(), expected);
    }

    #[tokio::test(start_paused = true)]
    async fn inbound_audio_arrives_as_16_khz_frames() {
        let (mut asterisk, mut media, _task) = started_line().await;
        // Two and a half frames in odd-sized messages.
        for size in [100, 500, 200] {
            asterisk.write_all(&encode(LINE_RATE_HZ, &vec![0u8; size]).unwrap()).await.unwrap();
        }
        for _ in 0..2 {
            assert_eq!(media.frames.recv().await.unwrap().len(), CORE_FRAME);
        }
    }

    /// With nothing from Asterisk, silent frames keep media time moving: one per tick once
    /// the gap reaches 100 ms.
    #[tokio::test(start_paused = true)]
    async fn a_quiet_line_is_filled_with_silence() {
        let (asterisk, mut media, _task) = started_line().await;
        let start = Instant::now();
        let first = media.frames.recv().await.unwrap();
        assert_eq!(first, [0; CORE_FRAME]);
        assert_eq!(Instant::now() - start, GAP_FILL_AFTER);
        media.frames.recv().await.unwrap();
        assert_eq!(Instant::now() - start, GAP_FILL_AFTER + TICK);
        drop(asterisk);
    }

    #[tokio::test(start_paused = true)]
    async fn the_line_ends_when_asterisk_closes_the_socket() {
        let (asterisk, mut media, task) = started_line().await;
        drop(asterisk);
        assert!(task.await.unwrap().is_ok());
        assert!(media.frames.recv().await.is_none());
        // Speaking to an ended call is harmless.
        media.speaker.play(vec![0; 10]);
        assert_eq!(media.speaker.clear().await, Cleared::default());
    }

    #[tokio::test(start_paused = true)]
    async fn hangup_sends_the_hangup_message_and_ends_the_line() {
        let (mut asterisk, agent) = duplex(1 << 16);
        asterisk.write_all(&UUID_MESSAGE).await.unwrap();
        let (_uuid, _media, control, task) = accept(agent).await.unwrap();
        control.hangup();
        assert!(task.await.unwrap().is_ok());
        // Whatever silent frames went out first, the last message is the hangup.
        let mut sent = Vec::new();
        asterisk.read_to_end(&mut sent).await.unwrap();
        assert_eq!(sent[sent.len() - 3..], encode_hangup());
    }
}
