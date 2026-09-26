//! A minimal client for Asterisk's manager interface (AMI), for the escalation's transfer
//! (issue #20) and outbound check-ins (issue #22). Each use opens its own short session: log
//! in, run one action, log off. Wire format and permissions from
//! `docs/research/asterisk-manager-interface.md` on branch `research/asterisk-manager-interface`,
//! section 6 (issue #5).
//!
//! Finding the call's channel uses `Status`, not `DBGet`: `Status` is allowed to the `call`
//! class (`main/manager.c:9801` at 22.11.0), and `DBGet` would need `reporting` or `system`
//! on top of the `call,originate` user pinned in issue #30. It lists every channel with the
//! variables asked for (`manager.c:3800-3870`), so the agent picks the one whose `AS_UUID`,
//! set by the dialplan before `AudioSocket()`, is this call's UUID.
//!
//! Placing a call is `Originate` with `Async: true`, which answers at once; the outcome comes
//! later as events (research, sections 2 and 3). So that session alone logs in with events on,
//! and stays open while the phone rings.

use std::time::Duration;

use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;

use super::Placed;

pub type Error = Box<dyn std::error::Error + Send + Sync>;

/// A session that takes longer than this, from connecting to the last response, is abandoned.
const TIMEOUT: Duration = Duration::from_secs(3);

/// Where the dialplan's phones live: extensions.conf, `[from-phones]`.
pub const PHONES_CONTEXT: &str = "from-phones";

/// Where an outbound check-in goes once the resident answers: extensions.conf,
/// `[checkin-outbound]`, extension `start`. No phone can dial into that context.
const OUTBOUND_CONTEXT: &str = "checkin-outbound";
const OUTBOUND_EXTEN: &str = "start";

/// What the resident's phone shows for an outbound check-in. 3100 is the agent's own
/// extension, so calling back from the phone's history reaches a check-in too.
const OUTBOUND_CALLER_ID: &str = "\"Check-in\" <3100>";

/// How long past the ring timeout to wait for Asterisk to report the outcome.
const OUTCOME_MARGIN: Duration = Duration::from_secs(10);

/// How long to wait, after a missed call's `OriginateResponse`, for the `Hangup` that says why.
/// The two are raised by different paths, in no set order (research, section 3).
const HANGUP_WAIT: Duration = Duration::from_secs(1);

/// Numbers that are emergency services somewhere. A transfer never goes to one, whatever
/// `.env` says; the dialplan has no such extension anyway.
const EMERGENCY_NUMBERS: &[&str] = &["911", "112", "999", "000", "111", "110", "119", "100"];

#[derive(Debug, Clone)]
pub struct Ami {
    pub addr: String,
    pub username: String,
    pub secret: String,
}

/// The channel a call runs on, as `Status` reports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Channel {
    pub name: String,
    /// The caller's extension, e.g. `2000`, if Asterisk knows it.
    pub caller: Option<String>,
}

/// One AMI message: its `Key: Value` lines in order. Keys can repeat (`Variable`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Message(pub Vec<(String, String)>);

impl Message {
    /// The first value for `key`, compared case-insensitively as Asterisk does.
    pub fn get(&self, key: &str) -> Option<&str> {
        self.0.iter().find(|(k, _)| k.eq_ignore_ascii_case(key)).map(|(_, v)| v.as_str())
    }

    fn all<'a>(&'a self, key: &'a str) -> impl Iterator<Item = &'a str> {
        self.0.iter().filter(move |(k, _)| k.eq_ignore_ascii_case(key)).map(|(_, v)| v.as_str())
    }
}

/// Checks an extension a transfer or a placed call may go to: digits only, and never an
/// emergency number.
pub fn check_extension(extension: &str) -> Result<(), Error> {
    if extension.is_empty() || !extension.chars().all(|c| c.is_ascii_digit()) {
        return Err(format!("{extension:?} is not an internal extension number").into());
    }
    if EMERGENCY_NUMBERS.contains(&extension) {
        return Err(format!("{extension} is an emergency number; never call it").into());
    }
    Ok(())
}

impl Ami {
    /// Logs in and out: AMI is up and the account works.
    pub async fn check(&self) -> Result<(), Error> {
        tokio::time::timeout(TIMEOUT, async {
            let mut session = self.open(false).await?;
            session.logoff().await;
            Ok(())
        })
        .await
        .map_err(|_| "AMI timed out")?
    }

    /// The channel running AudioSocket for the call with this UUID.
    pub async fn find_channel(&self, uuid: &str) -> Result<Channel, Error> {
        tokio::time::timeout(TIMEOUT, async {
            let mut session = self.open(false).await?;
            let channels = session.status("AS_UUID").await?;
            session.logoff().await;
            find(&channels, uuid).ok_or_else(|| format!("no channel has AS_UUID={uuid}").into())
        })
        .await
        .map_err(|_| "AMI timed out")?
    }

    /// Sends the call to `extension` in the phones' context. Returns Asterisk's message.
    /// "Redirect successful" means the channel was handed off, not that anyone answered
    /// (research, section 1).
    pub async fn redirect(&self, channel: &str, extension: &str) -> Result<String, Error> {
        check_extension(extension)?;
        tokio::time::timeout(TIMEOUT, async {
            let mut session = self.open(false).await?;
            let response = session
                .action(
                    "Redirect",
                    &[
                        ("Channel", channel),
                        ("Context", PHONES_CONTEXT),
                        ("Exten", extension),
                        ("Priority", "1"),
                    ],
                )
                .await?;
            session.logoff().await;
            let message = response.get("Message").unwrap_or_default().to_string();
            match response.get("Response") {
                Some(r) if r.eq_ignore_ascii_case("success") => Ok(message),
                _ => Err(format!("Redirect refused: {message}").into()),
            }
        })
        .await
        .map_err(|_| "AMI timed out")?
    }

    /// Rings `extension` for up to `ring`, and follows the call to its outcome. Answered, the
    /// dialplan's `[checkin-outbound]` hands the call to the agent over AudioSocket, with
    /// `call` as its UUID. `extension` must pass the same check as a transfer's.
    pub async fn originate(
        &self,
        extension: &str,
        call: &str,
        ring: Duration,
    ) -> Result<Placed, Error> {
        check_extension(extension)?;
        tokio::time::timeout(ring + OUTCOME_MARGIN, async {
            let mut session = self.open(true).await?;
            let placed = session.originate(extension, call, ring).await;
            session.logoff().await;
            placed
        })
        .await
        .map_err(|_| "AMI reported no outcome in time")?
    }

    /// Logs in. With `events` on, the session also receives every event the account may read.
    async fn open(&self, events: bool) -> Result<Session<TcpStream>, Error> {
        let stream = TcpStream::connect(&self.addr).await?;
        let mut session = Session::new(stream);
        session.login(&self.username, &self.secret, events).await?;
        Ok(session)
    }
}

/// Why a placed call was missed, for the log. `Reason` 3 is no answer before the timeout;
/// busy, declined and unreachable all come as 0, and only the channel's `Hangup` tells them
/// apart. A call whose channel was never made has no `Hangup` (research, section 3).
fn why_missed(reason: Option<&str>, hangup: Option<&Message>) -> String {
    if reason == Some("3") {
        return "no answer before the ring timeout".to_string();
    }
    let Some(hangup) = hangup else {
        return "the phone couldn't be dialled: not registered, or unreachable".to_string();
    };
    let cause = hangup.get("Cause-txt").unwrap_or("no cause given");
    match hangup.get("TechCause") {
        Some(sip) => format!("{cause} (SIP {sip})"),
        None => cause.to_string(),
    }
}

fn is_success(message: &Message) -> bool {
    message.get("Response").is_some_and(|r| r.eq_ignore_ascii_case("success"))
}

/// Picks the channel whose `AS_UUID` is `uuid` from `Status` events.
fn find(channels: &[Message], uuid: &str) -> Option<Channel> {
    let wanted = format!("AS_UUID={uuid}");
    let status = channels
        .iter()
        .find(|m| m.all("Variable").any(|v| v.eq_ignore_ascii_case(&wanted)))?;
    Some(Channel {
        name: status.get("Channel")?.to_string(),
        caller: status
            .get("CallerIDNum")
            .filter(|c| !c.is_empty() && *c != "<unknown>")
            .map(str::to_string),
    })
}

struct Session<S> {
    stream: BufReader<S>,
    next_id: u32,
}

impl<S: AsyncRead + AsyncWrite + Unpin> Session<S> {
    fn new(stream: S) -> Self {
        Session { stream: BufReader::new(stream), next_id: 0 }
    }

    /// Reads the banner, then logs in. With `events` off, only responses come back. On, every
    /// event the account's `read` classes allow comes too (`manager.c:838-865`): for this
    /// account that is `call` (asterisk/configure.sh).
    async fn login(&mut self, username: &str, secret: &str, events: bool) -> Result<(), Error> {
        let mut banner = String::new();
        self.stream.read_line(&mut banner).await?;
        if !banner.starts_with("Asterisk Call Manager/") {
            return Err(format!("not an AMI banner: {banner:?}").into());
        }
        let events = if events { "on" } else { "off" };
        let response = self
            .action("Login", &[("Username", username), ("Secret", secret), ("Events", events)])
            .await?;
        match response.get("Response") {
            Some(r) if r.eq_ignore_ascii_case("success") => Ok(()),
            _ => Err(format!("AMI login failed: {}", response.get("Message").unwrap_or("")).into()),
        }
    }

    /// Sends `Originate` and follows it to its outcome: `OriginateResponse` says whether the
    /// resident answered, and for a miss the `Hangup` of the call's channel says why. The two
    /// come in either order (research, section 3). Needs a session with events on.
    async fn originate(
        &mut self,
        extension: &str,
        call: &str,
        ring: Duration,
    ) -> Result<Placed, Error> {
        let channel = format!("PJSIP/{extension}");
        let timeout_ms = ring.as_millis().to_string();
        // Parameters from `main/manager_doc.xml:645-714` at 22.11.0, via the research, section 2.
        let id = self
            .send(
                "Originate",
                &[
                    ("Channel", &channel),
                    ("Context", OUTBOUND_CONTEXT),
                    ("Exten", OUTBOUND_EXTEN),
                    ("Priority", "1"),
                    // Milliseconds from dialling to answer.
                    ("Timeout", &timeout_ms),
                    // Answer now and report the outcome as an event, rather than holding the
                    // session until the phone stops ringing.
                    ("Async", "true"),
                    // The channel's uniqueid: the dialplan passes it to AudioSocket() as the
                    // call's UUID, and every event about the call carries it.
                    ("ChannelId", call),
                    // Without it, only signed linear is asked for, which the phones don't offer.
                    ("Codecs", "ulaw,alaw"),
                    ("CallerID", OUTBOUND_CALLER_ID),
                ],
            )
            .await?;
        let mut missed: Option<Message> = None;
        let mut hangup: Option<Message> = None;
        loop {
            let message = match missed {
                None => self.read().await?,
                // Missed: a moment more for the Hangup that says why, if there is one. The
                // outcome is known, so AMI closing now doesn't change it.
                Some(_) => match tokio::time::timeout(HANGUP_WAIT, self.read()).await {
                    Ok(Ok(message)) => message,
                    _ => break,
                },
            };
            let ours = message.get("ActionID") == Some(id.as_str());
            match message.get("Event") {
                // Originate's own response: queued, or refused outright.
                None if ours && !is_success(&message) => {
                    let why = message.get("Message").unwrap_or_default();
                    return Err(format!("Originate refused: {why}").into());
                }
                Some(event) if ours && event.eq_ignore_ascii_case("OriginateResponse") => {
                    if is_success(&message) {
                        return Ok(Placed::Answered);
                    }
                    missed = Some(message);
                }
                Some(event)
                    if event.eq_ignore_ascii_case("Hangup")
                        && message.get("Uniqueid") == Some(call) =>
                {
                    hangup = Some(message);
                }
                _ => {}
            }
            if missed.is_some() && hangup.is_some() {
                break;
            }
        }
        let reason = missed.as_ref().and_then(|m| m.get("Reason"));
        Ok(Placed::Missed(why_missed(reason, hangup.as_ref())))
    }

    async fn logoff(&mut self) {
        let _ = self.action("Logoff", &[]).await;
    }

    /// Sends an action and returns its response, skipping anything else.
    async fn action(&mut self, action: &str, headers: &[(&str, &str)]) -> Result<Message, Error> {
        let id = self.send(action, headers).await?;
        loop {
            let message = self.read().await?;
            if message.get("Response").is_some() && message.get("ActionID") == Some(id.as_str()) {
                return Ok(message);
            }
        }
    }

    /// `Status` for every channel, with `variable` in each: the `Status` events.
    async fn status(&mut self, variable: &str) -> Result<Vec<Message>, Error> {
        let id = self.send("Status", &[("Variables", variable)]).await?;
        let mut channels = Vec::new();
        loop {
            let message = self.read().await?;
            if message.get("ActionID") != Some(id.as_str()) {
                continue;
            }
            if let Some(response) = message.get("Response") {
                if !response.eq_ignore_ascii_case("success") {
                    let why = message.get("Message").unwrap_or_default();
                    return Err(format!("Status refused: {why}").into());
                }
                continue;
            }
            // The list ends with an event carrying `EventList: Complete` (research, section 6).
            if message.get("EventList").is_some_and(|e| !e.eq_ignore_ascii_case("start")) {
                return Ok(channels);
            }
            if message.get("Event").is_some_and(|e| e.eq_ignore_ascii_case("Status")) {
                channels.push(message);
            }
        }
    }

    async fn send(&mut self, action: &str, headers: &[(&str, &str)]) -> Result<String, Error> {
        self.next_id += 1;
        let id = format!("agent-{}", self.next_id);
        let mut text = format!("Action: {action}\r\nActionID: {id}\r\n");
        for (key, value) in headers {
            text.push_str(&format!("{key}: {value}\r\n"));
        }
        text.push_str("\r\n");
        self.stream.get_mut().write_all(text.as_bytes()).await?;
        Ok(id)
    }

    /// One message: `Key: Value` lines up to a blank line.
    async fn read(&mut self) -> Result<Message, Error> {
        let mut message = Message::default();
        loop {
            let mut line = String::new();
            if self.stream.read_line(&mut line).await? == 0 {
                return Err("AMI closed the connection".into());
            }
            let line = line.trim_end_matches(['\r', '\n']);
            if line.is_empty() {
                if message.0.is_empty() {
                    continue;
                }
                return Ok(message);
            }
            if let Some((key, value)) = line.split_once(':') {
                message.0.push((key.to_string(), value.trim_start().to_string()));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncReadExt;

    /// Plays Asterisk: answers each action in `script` with the canned reply, in order.
    async fn asterisk(mut server: tokio::io::DuplexStream, script: Vec<&'static str>) -> String {
        server.write_all(b"Asterisk Call Manager/11.0.0\r\n").await.unwrap();
        let mut seen = String::new();
        for reply in script {
            // Wait for one whole action (ends with a blank line).
            while !seen.ends_with("\r\n\r\n") {
                let mut buf = [0u8; 512];
                let n = server.read(&mut buf).await.unwrap();
                seen.push_str(std::str::from_utf8(&buf[..n]).unwrap());
            }
            let id = seen.rsplit("ActionID: ").next().unwrap().split("\r\n").next().unwrap();
            server.write_all(reply.replace("{id}", id).as_bytes()).await.unwrap();
            seen.push('.');
        }
        seen
    }

    #[tokio::test]
    async fn finds_the_call_s_channel_by_its_uuid() {
        let (client, server) = tokio::io::duplex(4096);
        let fake = tokio::spawn(asterisk(
            server,
            vec![
                "Response: Success\r\nActionID: {id}\r\nMessage: Authentication accepted\r\n\r\n",
                // An event that isn't ours can come before the response (research, section
                // 6); then the list: a channel of another call, and this call's.
                "Event: Newchannel\r\nPrivilege: call,all\r\nChannel: PJSIP/1001-00000002\r\n\r\n\
                 Response: Success\r\nActionID: {id}\r\nEventList: start\r\n\
                 Message: Channel status will follow\r\n\r\n\
                 Event: Status\r\nPrivilege: Call\r\nChannel: PJSIP/1001-00000001\r\n\
                 CallerIDNum: 1001\r\nVariable: AS_UUID=\r\nActionID: {id}\r\n\r\n\
                 Event: Status\r\nPrivilege: Call\r\nChannel: PJSIP/2000-00000007\r\n\
                 CallerIDNum: 2000\r\nVariable: AS_UUID=0f1e2d3c-4b5a-6978-8796-a5b4c3d2e1f0\r\n\
                 ActionID: {id}\r\n\r\n\
                 Event: StatusComplete\r\nActionID: {id}\r\nEventList: Complete\r\n\
                 ListItems: 2\r\nItems: 2\r\n\r\n",
            ],
        ));
        let mut session = Session::new(client);
        session.login("agent", "secret", false).await.unwrap();
        let channels = session.status("AS_UUID").await.unwrap();
        assert_eq!(channels.len(), 2);
        let channel = find(&channels, "0f1e2d3c-4b5a-6978-8796-a5b4c3d2e1f0").unwrap();
        assert_eq!(channel.name, "PJSIP/2000-00000007");
        assert_eq!(channel.caller.as_deref(), Some("2000"));
        assert_eq!(find(&channels, "ffffffff-4b5a-6978-8796-a5b4c3d2e1f0"), None);

        let sent = fake.await.unwrap();
        assert!(sent.contains("Action: Login\r\n"), "{sent}");
        assert!(sent.contains("Events: off\r\n"), "{sent}");
        assert!(sent.contains("Action: Status\r\n"), "{sent}");
        assert!(sent.contains("Variables: AS_UUID\r\n"), "{sent}");
    }

    #[tokio::test]
    async fn a_refused_login_is_an_error() {
        let (client, server) = tokio::io::duplex(4096);
        tokio::spawn(asterisk(
            server,
            vec!["Response: Error\r\nActionID: {id}\r\nMessage: Authentication failed\r\n\r\n"],
        ));
        let error = Session::new(client).login("agent", "wrong", false).await.unwrap_err();
        assert!(error.to_string().contains("Authentication failed"), "{error}");
    }

    const CALL: &str = "0d4c6f7e-2a61-4f0b-8c1e-5b9a3e7d2f10";
    const LOGIN_OK: &str = "Response: Success\r\nActionID: {id}\r\nMessage: Authentication accepted\r\n\r\n";

    /// Logs in with events on, places a call to 1001 and returns what became of it, with
    /// everything the agent sent. `reply` is Asterisk's answer to the `Originate`.
    async fn originate(reply: &'static str) -> (Result<Placed, Error>, String) {
        let (client, server) = tokio::io::duplex(4096);
        let fake = tokio::spawn(asterisk(server, vec![LOGIN_OK, reply]));
        let mut session = Session::new(client);
        session.login("agent", "secret", true).await.unwrap();
        let placed = session.originate("1001", CALL, Duration::from_secs(30)).await;
        drop(session);
        (placed, fake.await.unwrap())
    }

    #[tokio::test]
    async fn an_answered_call_is_reported_answered() {
        // The response, then events: another call's, and this one's outcome, as in the research
        // (section 2).
        let reply = "Response: Success\r\nActionID: {id}\r\nMessage: Originate successfully queued\r\n\r\n\
             Event: Newchannel\r\nPrivilege: call,all\r\nChannel: PJSIP/2000-00000003\r\n\
             Uniqueid: 1727300000.12\r\n\r\n\
             Event: OriginateResponse\r\nPrivilege: call,all\r\nActionID: {id}\r\n\
             Response: Success\r\nChannel: PJSIP/1001-00000007\r\nContext: checkin-outbound\r\n\
             Exten: start\r\nReason: 4\r\nUniqueid: 0d4c6f7e-2a61-4f0b-8c1e-5b9a3e7d2f10\r\n\
             CallerIDNum: 3100\r\nCallerIDName: Check-in\r\n\r\n";
        let (placed, sent) = originate(reply).await;
        assert_eq!(placed.unwrap(), Placed::Answered);
        assert!(sent.contains("Events: on\r\n"), "{sent}");
        for header in [
            "Action: Originate\r\n",
            "Channel: PJSIP/1001\r\n",
            "Context: checkin-outbound\r\n",
            "Exten: start\r\n",
            "Timeout: 30000\r\n",
            "Async: true\r\n",
            "ChannelId: 0d4c6f7e-2a61-4f0b-8c1e-5b9a3e7d2f10\r\n",
        ] {
            assert!(sent.contains(header), "{header:?} not in {sent}");
        }
    }

    #[tokio::test]
    async fn a_busy_phone_is_missed_with_the_hangup_s_cause() {
        // The Hangup can come before the OriginateResponse; another channel's is ignored.
        let reply = "Response: Success\r\nActionID: {id}\r\nMessage: Originate successfully queued\r\n\r\n\
             Event: Hangup\r\nPrivilege: call,all\r\nUniqueid: 1727300000.12\r\nCause: 16\r\n\
             Cause-txt: Normal Clearing\r\n\r\n\
             Event: Hangup\r\nPrivilege: call,all\r\nUniqueid: 0d4c6f7e-2a61-4f0b-8c1e-5b9a3e7d2f10\r\n\
             Cause: 17\r\nCause-txt: User busy\r\nTechCause: 486\r\n\r\n\
             Event: OriginateResponse\r\nPrivilege: call,all\r\nActionID: {id}\r\n\
             Response: Failure\r\nChannel: PJSIP/1001\r\nReason: 0\r\n\
             Uniqueid: 0d4c6f7e-2a61-4f0b-8c1e-5b9a3e7d2f10\r\n\r\n";
        let (placed, _) = originate(reply).await;
        assert_eq!(placed.unwrap(), Placed::Missed("User busy (SIP 486)".to_string()));
    }

    #[tokio::test]
    async fn an_unanswered_call_is_missed_at_the_ring_timeout() {
        let reply = "Response: Success\r\nActionID: {id}\r\nMessage: Originate successfully queued\r\n\r\n\
             Event: OriginateResponse\r\nPrivilege: call,all\r\nActionID: {id}\r\n\
             Response: Failure\r\nChannel: PJSIP/1001\r\nReason: 3\r\n\
             Uniqueid: 0d4c6f7e-2a61-4f0b-8c1e-5b9a3e7d2f10\r\n\r\n\
             Event: Hangup\r\nPrivilege: call,all\r\nUniqueid: 0d4c6f7e-2a61-4f0b-8c1e-5b9a3e7d2f10\r\n\
             Cause: 19\r\nCause-txt: User alerting, no answer\r\n\r\n";
        let (placed, _) = originate(reply).await;
        assert_eq!(placed.unwrap(), Placed::Missed("no answer before the ring timeout".to_string()));
    }

    #[tokio::test(start_paused = true)]
    async fn a_phone_that_can_t_be_dialled_is_missed_without_a_hangup() {
        // No channel is made for an unregistered endpoint, so no Hangup ever comes.
        let reply = "Response: Success\r\nActionID: {id}\r\nMessage: Originate successfully queued\r\n\r\n\
             Event: OriginateResponse\r\nPrivilege: call,all\r\nActionID: {id}\r\n\
             Response: Failure\r\nChannel: PJSIP/1001\r\nReason: 0\r\n\
             Uniqueid: 0d4c6f7e-2a61-4f0b-8c1e-5b9a3e7d2f10\r\n\r\n";
        let (placed, _) = originate(reply).await;
        let Placed::Missed(why) = placed.unwrap() else { panic!("not missed") };
        assert!(why.contains("couldn't be dialled"), "{why}");
    }

    #[tokio::test]
    async fn a_refused_originate_is_an_error() {
        let reply = "Response: Error\r\nActionID: {id}\r\nMessage: Extension does not exist.\r\n\r\n";
        let (placed, _) = originate(reply).await;
        let error = placed.unwrap_err();
        assert!(error.to_string().contains("Extension does not exist."), "{error}");
    }

    #[test]
    fn transfers_only_go_to_internal_extensions() {
        assert!(check_extension("2000").is_ok());
        assert!(check_extension("1001").is_ok());
        assert!(check_extension("911").is_err());
        assert!(check_extension("112").is_err());
        assert!(check_extension("_X.").is_err());
        assert!(check_extension("").is_err());
    }
}
