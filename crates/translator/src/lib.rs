use datom_codec::{Actualizing, Potential};
use energy_nexus_emission::{
    Consumed, Emission, Outcome, Query as EmissionQuery, Response as EmissionResponse,
};
use energy_nexus_wire::{Framing, Place, SocketPathing};
use serde_json::{json, Value};
use signal::{ByteViewable, Restorable, Signal, Signalizable};
use signal_energy_nexus::{
    Instant, Level, Minutes, Name, Percent, Provider, Reading, Reset, ResetId, ResetStatus, Window,
};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::process::{Command, Stdio};

#[derive(Clone, Debug)]
pub struct Programs {
    pub codex: String,
    pub flow: String,
}
impl Default for Programs {
    fn default() -> Self {
        Self {
            codex: "codex".into(),
            flow: "flow-0.49".into(),
        }
    }
}
#[derive(Clone, Debug)]
pub struct Translator {
    pub place: Place,
    pub programs: Programs,
}
pub trait Translating {
    fn emit(&self, emission: Emission) -> Outcome;
    fn serve(&self) -> std::io::Result<()>;
}
impl Translating for Translator {
    fn emit(&self, emission: Emission) -> Outcome {
        match emission {
            Emission::CodexRateLimits => self
                .codex_call("account/rateLimits/read", json!({}))
                .ok()
                .and_then(|value| codex_reading(&value))
                .map_or(Outcome::Down, Outcome::Observed),
            Emission::CodexConsumeReset {
                reset_id,
                idempotency,
            } => self
                .codex_call(
                    "account/rateLimitResetCredit/consume",
                    json!({"creditId":reset_id.0,"idempotencyKey":idempotency.0}),
                )
                .map(|value| consume_outcome(&value, reset_id))
                .unwrap_or(Outcome::Down),
            Emission::ClaudeUsage => claude_reading().map_or(Outcome::Down, Outcome::Observed),
            Emission::Send {
                recipient,
                provider,
                level,
            } => {
                if send_reminder(&self.programs.flow, &recipient, &provider, &level) {
                    Outcome::Sent
                } else {
                    Outcome::Down
                }
            }
        }
    }
    fn serve(&self) -> std::io::Result<()> {
        std::fs::create_dir_all(&self.place.runtime)?;
        for path in [
            self.place.public_translator(),
            self.place.meta_translator(),
            self.place.emission(),
        ] {
            let _ = std::fs::remove_file(path);
        }
        let public = UnixListener::bind(self.place.public_translator())?;
        let meta = UnixListener::bind(self.place.meta_translator())?;
        let emission = UnixListener::bind(self.place.emission())?;
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
        let mut child = Command::new(&self.programs.codex)
            .arg("app-server")
            .current_dir(directory.path())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| e.to_string())?;
        let mut input = child.stdin.take().ok_or("app-server stdin")?;
        for request in [
            json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"clientInfo":{"name":"energy-nexus","title":"energy-nexus","version":"1"}}}),
            json!({"jsonrpc":"2.0","method":"initialized"}),
            json!({"jsonrpc":"2.0","id":2,"method":method,"params":params}),
        ] {
            writeln!(input, "{request}").map_err(|e| e.to_string())?;
        }
        drop(input);
        let output = child.stdout.take().ok_or("app-server stdout")?;
        for line in BufReader::new(output).lines().map_while(Result::ok) {
            let Ok(value) = serde_json::from_str::<Value>(&line) else {
                continue;
            };
            if value.get("id").and_then(Value::as_i64) == Some(2) {
                let _ = child.kill();
                return value
                    .get("result")
                    .cloned()
                    .ok_or_else(|| "app-server refused".into());
            }
        }
        let _ = child.kill();
        Err("app-server closed".into())
    }
}
impl Translator {
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
        let Ok(signal) = query.signalize() else {
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
            let text = shared_translator::encode(&response);
            if client.write_frame(text.as_bytes()).is_err() || !subscribe {
                return;
            }
        }
    }
    fn serve_meta(&self, mut client: UnixStream) {
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
        let Ok(mut nexus) = UnixStream::connect(self.place.meta()) else {
            return;
        };
        let Ok(signal) = query.signalize() else {
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
        let _ = client.write_frame(shared_translator::encode(&response).as_bytes());
    }
}
fn budget() -> datom_codec::Budget {
    datom_codec::Budget {
        remaining: 100_000,
        reader: protos::ReaderBudget { remaining: 100_000 },
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
fn claude_reading() -> Option<Reading> {
    let home = std::env::var("HOME").ok()?;
    let text =
        std::fs::read_to_string(std::path::Path::new(&home).join(".claude/.credentials.json"))
            .ok()?;
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
        address.aspect, address.topic.0, address.layer, provider, level
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
        let value = json!({"five_hour":{"utilization":8.0,"resets_at":"2026-11-04T10:00:00Z"},"seven_day":{"utilization":34.0,"resets_at":"2026-11-08T10:00:00Z"}});
        let reading = claude_document(&value).unwrap();
        assert_eq!(reading.window_vector[1].name, Name::Week);
        assert_eq!(reading.window_vector[1].used, Percent(34));
    }
    #[test]
    fn flow_reminder_fixture_records_one_soft_notice() {
        use std::os::unix::fs::PermissionsExt;
        let directory = tempfile::tempdir().unwrap();
        let log = directory.path().join("flow.log");
        let program = directory.path().join("flow-0.49");
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
            aspect: signal_energy_nexus::Aspect::Mind,
            topic: signal_energy_nexus::Topic("energyNexus".into()),
            layer: signal_energy_nexus::Layer::Secondary,
        });
        assert!(send_reminder(
            program.to_str().unwrap(),
            &recipient,
            &Provider::Codex,
            &Level::Low
        ));
        let transcript = std::fs::read_to_string(log).unwrap();
        println!("flow-0.49 reminder fixture: {}", transcript.trim());
        assert!(transcript.contains(
            "Send.{ { Mind energyNexus Secondary } Soft.Notice.«Energy Codex is Low.» }"
        ));
    }
}
