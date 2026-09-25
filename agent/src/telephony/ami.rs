//! A minimal client for Asterisk's manager interface (AMI), for the escalation's transfer
//! (issue #20). Each use opens its own short session: log in with events off, run one action,
//! log off. Wire format and permissions from `docs/research/asterisk-manager-interface.md` on
//! branch `research/asterisk-manager-interface`, section 6 (issue #5).
//!
//! Finding the call's channel uses `Status`, not `DBGet`: `Status` is allowed to the `call`
//! class (`main/manager.c:9801` at 22.11.0), and `DBGet` would need `reporting` or `system`
//! on top of the `call,originate` user pinned in issue #30. It lists every channel with the
//! variables asked for (`manager.c:3800-3870`), so the agent picks the one whose `AS_UUID`,
//! set by the dialplan before `AudioSocket()`, is this call's UUID.

use std::time::Duration;

use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;

pub type Error = Box<dyn std::error::Error + Send + Sync>;

/// A session that takes longer than this, from connecting to the last response, is abandoned.
const TIMEOUT: Duration = Duration::from_secs(3);

/// Where the dialplan's phones live: extensions.conf, `[from-phones]`.
pub const PHONES_CONTEXT: &str = "from-phones";

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

/// Checks an extension a transfer may go to: digits only, and never an emergency number.
pub fn check_extension(extension: &str) -> Result<(), Error> {
    if extension.is_empty() || !extension.chars().all(|c| c.is_ascii_digit()) {
        return Err(format!("{extension:?} is not an internal extension number").into());
    }
    if EMERGENCY_NUMBERS.contains(&extension) {
        return Err(format!("{extension} is an emergency number; never transfer to it").into());
    }
    Ok(())
}

impl Ami {
    /// Logs in and out: AMI is up and the account works.
    pub async fn check(&self) -> Result<(), Error> {
        tokio::time::timeout(TIMEOUT, async {
            let mut session = self.open().await?;
            session.logoff().await;
            Ok(())
        })
        .await
        .map_err(|_| "AMI timed out")?
    }

    /// The channel running AudioSocket for the call with this UUID.
    pub async fn find_channel(&self, uuid: &str) -> Result<Channel, Error> {
        tokio::time::timeout(TIMEOUT, async {
            let mut session = self.open().await?;
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
            let mut session = self.open().await?;
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

    async fn open(&self) -> Result<Session<TcpStream>, Error> {
        let stream = TcpStream::connect(&self.addr).await?;
        let mut session = Session::new(stream);
        session.login(&self.username, &self.secret).await?;
        Ok(session)
    }
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

    /// Reads the banner, then logs in with events off, so only responses come back.
    async fn login(&mut self, username: &str, secret: &str) -> Result<(), Error> {
        let mut banner = String::new();
        self.stream.read_line(&mut banner).await?;
        if !banner.starts_with("Asterisk Call Manager/") {
            return Err(format!("not an AMI banner: {banner:?}").into());
        }
        let response = self
            .action("Login", &[("Username", username), ("Secret", secret), ("Events", "off")])
            .await?;
        match response.get("Response") {
            Some(r) if r.eq_ignore_ascii_case("success") => Ok(()),
            _ => Err(format!("AMI login failed: {}", response.get("Message").unwrap_or("")).into()),
        }
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
        session.login("agent", "secret").await.unwrap();
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
        let error = Session::new(client).login("agent", "wrong").await.unwrap_err();
        assert!(error.to_string().contains("Authentication failed"), "{error}");
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
