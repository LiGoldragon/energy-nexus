use datom_codec::{Actualizing, Potential};
use energy_nexus_emission::{
    ClaudeUsage, CodexConsumeReset, CodexRateLimits, Consumed, Emission, Emittable, Outcome,
    Query as EmissionQuery, Response as EmissionResponse, Send,
};
use energy_nexus_wire::{
    AnnouncesReadiness, Declared, Framing, MemoryPathing, OwnedSockets, OwningSockets, Place,
    Readiness, SocketPathing,
};
use serde_json::{json, Value};
use signal::{ByteViewable, Restorable, Signal, Signalizable};
use signal_energy_nexus::{
    Instant, Level, Minutes, Name, Percent, Provider, Reading, Reset, ResetId, ResetStatus, Window,
};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};

#[derive(rkyv::Archive, rkyv::Serialize, rkyv::Deserialize, Clone, Debug, Default)]
pub struct Programs {
    pub codex: Option<String>,
    pub flow: Option<String>,
    pub credentials: Option<String>,
}
#[derive(Clone, Debug)]
pub struct Translator {
    pub place: Place,
    pub programs: Arc<Mutex<Programs>>,
}
pub trait Translating {
    fn emit(&self, emission: Emission) -> Outcome;
    fn serve(&self) -> std::io::Result<()>;
}
impl Translating for Translator {
    fn emit(&self, emission: Emission) -> Outcome {
        match emission {
            Emission::CodexRateLimits(value) => value.emit(self),
            Emission::CodexConsumeReset(value) => value.emit(self),
            Emission::ClaudeUsage(value) => value.emit(self),
            Emission::Send(value) => value.emit(self),
        }
    }
    fn serve(&self) -> std::io::Result<()> {
        self.place.confirm()?;
        let own = OwnedSockets(vec![
            self.place.public_translator(),
            self.place.meta_translator(),
            self.place.emission(),
        ]);
        own.remove_at_stop()?;
        own.remove();
        let public = UnixListener::bind(self.place.public_translator())?;
        let meta = UnixListener::bind(self.place.meta_translator())?;
        let emission = UnixListener::bind(self.place.emission())?;
        // All three sockets listen: the clients and the daemon find the
        // translator serving.
        if let Err(error) = Readiness::announce_ready() {
            eprintln!("readiness not announced: {error}");
        }
        let this = self.clone();
        std::thread::spawn(move || {
            for stream in emission.incoming().flatten() {
                let this = this.clone();
                std::thread::spawn(move || this.serve_emission(stream));
            }
        });
        let this = self.clone();
        std::thread::spawn(move || {
            for stream in meta.incoming().flatten() {
                let this = this.clone();
                std::thread::spawn(move || this.serve_meta(stream));
            }
        });
        for stream in public.incoming().flatten() {
            let this = self.clone();
            std::thread::spawn(move || this.serve_public(stream));
        }
        Ok(())
    }
}
pub trait CodexCalling {
    fn codex_call(&self, method: &str, params: Value) -> Result<Value, String>;
}
impl CodexCalling for Translator {
    fn codex_call(&self, method: &str, params: Value) -> Result<Value, String> {
        let directory = tempfile::tempdir().map_err(|error| error.to_string())?;
        let program = self
            .programs
            .lock()
            .ok()
            .and_then(|programs| programs.codex.clone())
            .ok_or("Codex program is unconfigured")?;
        let mut server = AppServer(Command::new(program)
            .arg("app-server")
            .current_dir(directory.path())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| e.to_string())?);
        let mut input = server.0.stdin.take().ok_or("app-server stdin")?;
        for request in [
            json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"clientInfo":{"name":"energy-nexus","title":"energy-nexus","version":"1"}}}),
            json!({"jsonrpc":"2.0","method":"initialized"}),
            json!({"jsonrpc":"2.0","id":2,"method":method,"params":params}),
        ] {
            writeln!(input, "{request}").map_err(|e| e.to_string())?;
        }
        let output = server.0.stdout.take().ok_or("app-server stdout")?;
        for line in BufReader::new(output).lines().map_while(Result::ok) {
            let Ok(value) = serde_json::from_str::<Value>(&line) else {
                continue;
            };
            if value.get("id").and_then(Value::as_i64) == Some(2) {
                return value
                    .get("result")
                    .cloned()
                    .ok_or_else(|| "app-server refused".into());
            }
        }
        Err("app-server closed".into())
    }
}
/// A spawned Codex app-server, killed and reaped when it goes out of scope.
struct AppServer(std::process::Child);
impl Drop for AppServer {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
impl Emittable<Translator> for CodexRateLimits {
    fn emit(&self, translator: &Translator) -> Outcome {
        translator
            .codex_call("account/rateLimits/read", json!({}))
            .ok()
            .and_then(|value| codex_reading(&value))
            .map_or(Outcome::Down, Outcome::Observed)
    }
}
impl Emittable<Translator> for CodexConsumeReset {
    fn emit(&self, translator: &Translator) -> Outcome {
        translator
            .codex_call(
                "account/rateLimitResetCredit/consume",
                json!({"creditId":self.reset_id.0,"idempotencyKey":self.idempotency.0}),
            )
            .map(|value| consume_outcome(&value, self.reset_id.clone()))
            .unwrap_or(Outcome::Down)
    }
}
impl Emittable<Translator> for ClaudeUsage {
    fn emit(&self, translator: &Translator) -> Outcome {
        let path = translator
            .programs
            .lock()
            .ok()
            .and_then(|programs| programs.credentials.clone());
        path.as_deref()
            .and_then(claude_reading)
            .map_or(Outcome::Down, Outcome::Observed)
    }
}
impl Emittable<Translator> for Send {
    fn emit(&self, translator: &Translator) -> Outcome {
        let program = translator
            .programs
            .lock()
            .ok()
            .and_then(|programs| programs.flow.clone());
        if program.as_deref().is_some_and(|program| {
            send_reminder(program, &self.recipient, &self.provider, &self.level)
        }) {
            Outcome::Sent
        } else {
            Outcome::Down
        }
    }
}
impl Translator {
    pub fn open(place: Place) -> Self {
        let programs = std::fs::read(place.translator_memory())
            .ok()
            .and_then(|bytes| rkyv::from_bytes::<Programs, rkyv::rancor::Error>(&bytes).ok())
            .unwrap_or_default();
        Self {
            place,
            programs: Arc::new(Mutex::new(programs)),
        }
    }
    fn configure(&self, set: &meta_signal_energy_nexus::Set) -> Result<bool, String> {
        let changed = match set {
            meta_signal_energy_nexus::Set::Program(value) => {
                let mut programs = self
                    .programs
                    .lock()
                    .map_err(|_| "translator memory unavailable")?;
                match value.program {
                    meta_signal_energy_nexus::Program::Codex => {
                        programs.codex = Some(value.path.0.clone())
                    }
                    meta_signal_energy_nexus::Program::Flow => {
                        programs.flow = Some(value.path.0.clone())
                    }
                }
                true
            }
            meta_signal_energy_nexus::Set::Credentials(value) => {
                self.programs
                    .lock()
                    .map_err(|_| "translator memory unavailable")?
                    .credentials = Some(value.0.clone());
                true
            }
            _ => false,
        };
        if changed {
            let programs = self
                .programs
                .lock()
                .map_err(|_| "translator memory unavailable")?;
            let bytes = rkyv::to_bytes::<rkyv::rancor::Error>(&*programs)
                .map_err(|error| error.to_string())?;
            let memory = self.place.translator_memory();
            let temporary = memory.with_extension("new");
            std::fs::write(&temporary, bytes.as_slice()).map_err(|error| error.to_string())?;
            std::fs::rename(temporary, memory)
                .map_err(|error| error.to_string())?;
        }
        Ok(changed)
    }
    fn serve_emission(&self, mut stream: UnixStream) {
        let Ok(bytes) = stream.read_frame() else {
            return;
        };
        let Ok(EmissionQuery::Emit(emission)) = Signal::<EmissionQuery>::from(bytes).restore()
        else {
            return;
        };
        if let Ok(signal) = EmissionResponse::Emitted(self.emit(emission)).signalize() {
            let _ = stream.write_frame(signal.bytes());
        }
    }
    fn serve_public(&self, mut client: UnixStream) {
        let Some(caller) = caller(&client) else {
            return;
        };
        let Ok(text) = client.read_frame().and_then(|v| {
            String::from_utf8(v)
                .map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidData, "non-UTF-8 text"))
        }) else {
            return;
        };
        let mut budget = budget();
        let mut potential = Potential::<signal_energy_nexus::Query>::from(text.as_str());
        let Ok(query) = potential.actualize(&mut budget) else {
            return;
        };
        let subscribe = matches!(query, signal_energy_nexus::Query::Subscribe);
        let Ok(mut nexus) = UnixStream::connect(self.place.public()) else {
            return;
        };
        let call = signal_energy_nexus::Call {
            caller,
            input: query,
        };
        let Ok(signal) = call.signalize() else {
            return;
        };
        if nexus.write_frame(signal.bytes()).is_err() {
            return;
        }
        loop {
            let Ok(bytes) = nexus.read_frame() else {
                return;
            };
            let Ok(response) = Signal::<signal_energy_nexus::Response>::from(bytes).restore()
            else {
                return;
            };
            let text = encode(&response);
            if client.write_frame(text.as_bytes()).is_err() || !subscribe {
                return;
            }
        }
    }
    fn serve_meta(&self, mut client: UnixStream) {
        let Some(caller) = caller(&client) else {
            return;
        };
        let Ok(text) = client.read_frame().and_then(|v| {
            String::from_utf8(v)
                .map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidData, "non-UTF-8 text"))
        }) else {
            return;
        };
        let mut budget = budget();
        let mut potential = Potential::<meta_signal_energy_nexus::Query>::from(text.as_str());
        let Ok(query) = potential.actualize(&mut budget) else {
            return;
        };
        let meta_signal_energy_nexus::Query::Configure(set) = &query;
        match self.configure(set) {
            Ok(true) => {
                let response = meta_signal_energy_nexus::Response::Configured(set.clone());
                let _ = client.write_frame(encode(&response).as_bytes());
                return;
            }
            Err(reason) => {
                let response = meta_signal_energy_nexus::Response::ConfigureRefused(reason);
                let _ = client.write_frame(encode(&response).as_bytes());
                return;
            }
            Ok(false) => {}
        }
        let Ok(mut nexus) = UnixStream::connect(self.place.meta()) else {
            return;
        };
        let call = meta_signal_energy_nexus::Call {
            caller,
            input: query,
        };
        let Ok(signal) = call.signalize() else {
            return;
        };
        if nexus.write_frame(signal.bytes()).is_err() {
            return;
        }
        let Ok(bytes) = nexus.read_frame() else {
            return;
        };
        let Ok(response) = Signal::<meta_signal_energy_nexus::Response>::from(bytes).restore()
        else {
            return;
        };
        let _ = client.write_frame(encode(&response).as_bytes());
    }
}
fn caller(stream: &UnixStream) -> Option<signal_energy_nexus::Caller> {
    let credentials =
        nix::sys::socket::getsockopt(stream, nix::sys::socket::sockopt::PeerCredentials).ok()?;
    Some(signal_energy_nexus::Caller {
        process_id: credentials.pid() as i64,
        user_id: credentials.uid() as i64,
    })
}
fn encode<T: datom_codec::Datomizable>(value: &T) -> String {
    use protos::{Protosizable, Textualizable};
    value.datomize(Vec::new()).protosize().textualize()
}
fn budget() -> datom_codec::Budget {
    datom_codec::Budget {
        remaining: 100_000,
        reader: protos::ReaderBudget::try_from(100_000).expect("the reader budget is within range"),
        depth: 0,
        maximum_depth: 128,
    }
}
pub trait TextClienting {
    fn request(&self, text: &str, meta: bool, follow: bool) -> Result<Vec<String>, String>;
}
impl TextClienting for Translator {
    fn request(&self, text: &str, meta: bool, follow: bool) -> Result<Vec<String>, String> {
        let path = if meta {
            self.place.meta_translator()
        } else {
            self.place.public_translator()
        };
        let mut stream = UnixStream::connect(path).map_err(|e| e.to_string())?;
        stream
            .write_frame(text.as_bytes())
            .map_err(|e| e.to_string())?;
        let mut answers = Vec::new();
        loop {
            match stream.read_frame() {
                Ok(bytes) => {
                    answers.push(String::from_utf8(bytes).map_err(|e| e.to_string())?);
                    if !follow {
                        return Ok(answers);
                    }
                }
                Err(error)
                    if !answers.is_empty() && error.kind() == std::io::ErrorKind::UnexpectedEof =>
                {
                    return Ok(answers)
                }
                Err(error) => return Err(error.to_string()),
            }
        }
    }
}
fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}
fn codex_reading(value: &Value) -> Option<Reading> {
    let allowed = value
        .get("ordinaryUsageAllowed")
        .and_then(Value::as_bool)
        .unwrap_or(true);
    let bucket = value.get("rateLimits").or_else(|| {
        value
            .get("rateLimitsByLimitId")?
            .as_object()?
            .values()
            .next()
    })?;
    let mut windows = Vec::new();
    for key in ["primary", "secondary"] {
        let Some(window) = bucket.get(key).filter(|v| !v.is_null()) else {
            continue;
        };
        let duration = window
            .get("windowDurationMins")
            .and_then(Value::as_i64)
            .unwrap_or(0);
        windows.push(Window {
            name: if duration == 10080 {
                Name::Week
            } else {
                Name::Session
            },
            used: Percent(
                window
                    .get("usedPercent")
                    .and_then(Value::as_f64)
                    .unwrap_or(0.0)
                    .round() as i64,
            ),
            resets: Instant(window.get("resetsAt").and_then(Value::as_i64).unwrap_or(0)),
            duration: Minutes(duration),
        });
    }
    let resets = value
        .get("rateLimitResetCredits")
        .and_then(|v| v.get("credits"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|credit| {
            Some(Reset {
                reset_id: ResetId(credit.get("id")?.as_str()?.to_owned()),
                reset_status: match credit.get("status")?.as_str()? {
                    "available" => ResetStatus::Available,
                    "redeeming" => ResetStatus::Redeeming,
                    "redeemed" => ResetStatus::Redeemed,
                    _ => return None,
                },
                expires: Instant(credit.get("expiresAt").and_then(Value::as_i64).unwrap_or(0)),
            })
        })
        .collect();
    Some(Reading {
        provider: Provider::Codex,
        allowed,
        window_vector: windows,
        reset_vector: resets,
        at: Instant(now()),
    })
}
fn claude_reading(credentials_path: &str) -> Option<Reading> {
    let text = std::fs::read_to_string(credentials_path).ok()?;
    let credentials: Value = serde_json::from_str(&text).ok()?;
    let oauth = credentials.get("claudeAiOauth")?;
    let token = oauth.get("accessToken")?.as_str()?;
    let response = ureq::get("https://api.anthropic.com/api/oauth/usage")
        .header("Authorization", &format!("Bearer {token}"))
        .header("anthropic-beta", "oauth-2025-04-20")
        .call()
        .ok()?;
    let body = response.into_body().read_to_string().ok()?;
    let body: Value = serde_json::from_str(&body).ok()?;
    claude_document(&body)
}
fn rfc3339(value: &Value) -> i64 {
    value
        .as_str()
        .and_then(|v| {
            time::OffsetDateTime::parse(v, &time::format_description::well_known::Rfc3339).ok()
        })
        .map(|v| v.unix_timestamp())
        .unwrap_or(0)
}
fn claude_document(value: &Value) -> Option<Reading> {
    let mut windows = Vec::new();
    for (key, name, duration) in [
        ("five_hour", Name::Session, 300),
        ("seven_day", Name::Week, 10080),
    ] {
        let Some(window) = value.get(key).filter(|v| !v.is_null()) else {
            continue;
        };
        windows.push(Window {
            name,
            used: Percent(
                window
                    .get("utilization")
                    .and_then(Value::as_f64)
                    .unwrap_or(0.0)
                    .round() as i64,
            ),
            resets: Instant(rfc3339(window.get("resets_at").unwrap_or(&Value::Null))),
            duration: Minutes(duration),
        });
    }
    if let (Some(week), Some(scoped)) = (
        value.get("seven_day").filter(|value| !value.is_null()),
        value
            .get("weekly_scoped_shares")
            .and_then(Value::as_array)
            .and_then(|shares| shares.first()),
    ) {
        let allowance = scoped
            .get("allowance_percent_of_weekly")
            .and_then(Value::as_f64)
            .unwrap_or(0.0);
        let used = scoped
            .get("used_percent_of_weekly")
            .and_then(Value::as_f64)
            .unwrap_or(0.0);
        if allowance > 0.0 {
            windows.push(Window {
                name: Name::FableWeek,
                used: Percent((used * 100.0 / allowance).round() as i64),
                resets: Instant(rfc3339(week.get("resets_at").unwrap_or(&Value::Null))),
                duration: Minutes(10080),
            });
        }
    }
    Some(Reading {
        provider: Provider::Claude,
        allowed: true,
        window_vector: windows,
        reset_vector: Vec::new(),
        at: Instant(now()),
    })
}
fn consume_outcome(value: &Value, reset_id: ResetId) -> Outcome {
    match value.get("outcome").and_then(Value::as_str).unwrap_or("") {
        "consumed" => Outcome::Consumed(Consumed::Reset(Reset {
            reset_id,
            reset_status: ResetStatus::Redeemed,
            expires: Instant(0),
        })),
        "nothingToReset" => Outcome::Consumed(Consumed::NothingToReset),
        "noCredit" => Outcome::Consumed(Consumed::NoCredit),
        "alreadyRedeemed" => Outcome::Consumed(Consumed::AlreadyRedeemed),
        _ => Outcome::Down,
    }
}
fn send_reminder(
    program: &str,
    recipient: &signal_energy_nexus::Recipient,
    provider: &Provider,
    level: &Level,
) -> bool {
    let address = &recipient.0;
    let datom = format!(
        "Send.{{ {{ {:?} {} {:?} }} Soft.Notice.«Energy {:?} is {:?}.» }}",
        address.aspect, address.topic, address.model, provider, level
    );
    Command::new(program)
        .arg(datom)
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn codex_fixture_maps_windows_and_resets() {
        let value = json!({"ordinaryUsageAllowed":false,"rateLimits":{"primary":{"usedPercent":8,"resetsAt":20,"windowDurationMins":300},"secondary":{"usedPercent":34,"resetsAt":30,"windowDurationMins":10080}},"rateLimitResetCredits":{"credits":[{"id":"rc_1","status":"available","expiresAt":40}]}});
        let reading = codex_reading(&value).unwrap();
        assert!(!reading.allowed);
        assert_eq!(reading.window_vector.len(), 2);
        assert_eq!(reading.reset_vector[0].reset_id.0, "rc_1");
    }
    #[test]
    fn claude_fixture_maps_named_windows() {
        let value = json!({"five_hour":{"utilization":8.0,"resets_at":"2026-11-04T10:00:00Z"},"seven_day":{"utilization":34.0,"resets_at":"2026-11-08T10:00:00Z"},"weekly_scoped_shares":[{"allowance_percent_of_weekly":50.0,"used_percent_of_weekly":31.5} ]});
        let reading = claude_document(&value).unwrap();
        assert_eq!(reading.window_vector[1].name, Name::Week);
        assert_eq!(reading.window_vector[1].used, Percent(34));
        assert_eq!(reading.window_vector[2].name, Name::FableWeek);
        assert_eq!(reading.window_vector[2].used, Percent(63));
    }
    #[test]
    fn claude_missing_window_is_absent() {
        let value = json!({"five_hour":{"utilization":8.0,"resets_at":"2026-11-04T10:00:00Z"},"seven_day":{"utilization":34.0,"resets_at":"2026-11-08T10:00:00Z"},"weekly_scoped_shares":[]});
        let reading = claude_document(&value).unwrap();
        assert_eq!(reading.window_vector.len(), 2);
        assert!(reading
            .window_vector
            .iter()
            .all(|window| window.name != Name::FableWeek));
    }
    #[test]
    fn flow_reminder_fixture_records_one_soft_notice() {
        use std::os::unix::fs::PermissionsExt;
        let directory = tempfile::tempdir().unwrap();
        let log = directory.path().join("flow.log");
        let program = directory.path().join("flow");
        std::fs::write(
            &program,
            format!(
                r#"#!/bin/sh
printf '%s\n' "$1" > {}
"#,
                log.display()
            ),
        )
        .unwrap();
        let mut permissions = std::fs::metadata(&program).unwrap().permissions();
        permissions.set_mode(0o700);
        std::fs::set_permissions(&program, permissions).unwrap();
        let recipient = signal_energy_nexus::Recipient(signal_energy_nexus::Address {
            aspect: signal_flow::Aspect::Mind,
            topic: "energyNexus".into(),
            model: signal_flow::Model::Sol,
        });
        assert!(send_reminder(
            program.to_str().unwrap(),
            &recipient,
            &Provider::Codex,
            &Level::Low
        ));
        let transcript = std::fs::read_to_string(log).unwrap();
        println!("flow reminder fixture: {}", transcript.trim());
        assert!(transcript.contains(
            "Send.{ { Mind energyNexus Sol } Soft.Notice.«Energy Codex is Low.» }"
        ));
        // The argument is a Query of the Flow contract Energy is built with,
        // signal-flow 57.0.0: a Send to the configured recipient.
        let query = Potential::<signal_flow::Query>::from(transcript.trim())
            .actualize(&mut budget())
            .expect("the reminder reads as a Flow Query");
        match query {
            signal_flow::Query::Send(send) => assert_eq!(send.recipient, recipient.0),
            other => panic!("a reminder is a Send, got {other:?}"),
        }
    }
    #[test]
    fn codex_app_server_is_reaped_after_its_answer() {
        use std::os::unix::fs::PermissionsExt;
        let directory = tempfile::tempdir().unwrap();
        let pid = directory.path().join("app-server.pid");
        let program = directory.path().join("codex");
        std::fs::write(
            &program,
            format!(
                r#"#!/bin/sh
echo $$ > {}
read a; read b; read c
printf '%s\n' '{{"jsonrpc":"2.0","id":2,"result":{{"ok":true}}}}'
sleep 60
"#,
                pid.display()
            ),
        )
        .unwrap();
        let mut permissions = std::fs::metadata(&program).unwrap().permissions();
        permissions.set_mode(0o700);
        std::fs::set_permissions(&program, permissions).unwrap();
        let translator = Translator {
            place: Place {
                name: energy_nexus_wire::PlaceName("test".into()),
                runtime: directory.path().into(),
                state: directory.path().into(),
            },
            programs: Arc::new(Mutex::new(Programs {
                codex: Some(program.to_str().unwrap().into()),
                ..Programs::default()
            })),
        };
        let value = translator.codex_call("account/rateLimits/read", json!({})).unwrap();
        assert_eq!(value, json!({"ok":true}));
        let child = std::fs::read_to_string(pid).unwrap();
        let process = std::path::Path::new("/proc").join(child.trim());
        println!("app-server {} after the call: {}", child.trim(), process.exists());
        assert!(!process.exists(), "the app-server is reaped, no zombie left");
    }
    #[test]
    fn meta_query_text_fixture() {
        let query = meta_signal_energy_nexus::Query::Configure(
            meta_signal_energy_nexus::Set::Polling(Minutes(5)),
        );
        assert_eq!(encode(&query), "Configure.Polling.5");
    }
    #[test]
    fn meta_query_text_reads_the_bare_minutes_and_refuses_the_braced_form() {
        use datom_codec::{Actualizing, Potential};
        let read = |text: &str| {
            Potential::<meta_signal_energy_nexus::Query>::from(text).actualize(&mut budget())
        };
        let bare = read("Configure.Polling.5").expect("the bare form reads");
        assert_eq!(
            bare,
            meta_signal_energy_nexus::Query::Configure(meta_signal_energy_nexus::Set::Polling(
                Minutes(5)
            ))
        );
        assert!(read("Configure.Polling.{ 5 }").is_err());
    }
}
