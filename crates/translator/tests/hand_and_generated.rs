//! The translator's records were hand-written until energy-nexus 0b9e6b4
//! and are generated from `ethos/programs.ethos` and `ethos/cli.ethos`
//! since. The hand declarations are frozen here verbatim (renamed by
//! prefix): `Programs` archives to the same bytes both ways, so
//! `energy-translator.rkyv` format version 1 is read and written as before,
//! and the CLI configuration and its refusals have the same datom text.
use datom_codec::{Actualizing, Datomizable, Potential};
use energy_nexus_translator::cli::{
    CliConfiguration, ConfigurationPath, ConfigurationRefusal, PlaceName, Reason, RefusedFile,
};
use energy_nexus_translator::{ClaudeCredentials, CodexProgram, FlowProgram, Programs};
use protos::{Protosizable, Textualizable};

#[derive(rkyv::Archive, rkyv::Serialize, rkyv::Deserialize, Clone, Debug, Default, PartialEq, Eq)]
pub struct HandPrograms {
    pub codex: Option<String>,
    pub flow: Option<String>,
    pub credentials: Option<String>,
}
#[derive(datom_codec::Datomizable, datom_codec::Composing, Clone, Debug, PartialEq, Eq)]
pub struct HandCliConfiguration {
    pub place_name: String,
}
#[derive(datom_codec::Datomizable, datom_codec::Composing, Clone, Debug, PartialEq, Eq)]
pub enum HandConfigurationRefusal {
    Unlocated(String),
    Missing(String),
    Unreadable { path: String, reason: String },
    Invalid { path: String, reason: String },
}

fn text<T: Datomizable>(value: &T) -> String {
    value.datomize(Vec::new()).protosize().textualize()
}
fn budget() -> datom_codec::Budget {
    datom_codec::Budget {
        remaining: 65_536,
        reader: protos::ReaderBudget::try_from(65_536).unwrap(),
        depth: 0,
        maximum_depth: 64,
    }
}

#[test]
fn programs_archive_as_the_hand_programs_both_ways() {
    let values = [
        (HandPrograms::default(), Programs::default()),
        (
            HandPrograms {
                codex: Some("/run/current-system/sw/bin/codex".into()),
                flow: None,
                credentials: Some("/home/li/.claude/.credentials.json".into()),
            },
            Programs {
                codex_program: CodexProgram(Some("/run/current-system/sw/bin/codex".into())),
                flow_program: FlowProgram(None),
                claude_credentials: ClaudeCredentials(Some(
                    "/home/li/.claude/.credentials.json".into(),
                )),
            },
        ),
        (
            HandPrograms {
                codex: Some("c".into()),
                flow: Some("/home/li/.local/bin/flow".into()),
                credentials: None,
            },
            Programs {
                codex_program: CodexProgram(Some("c".into())),
                flow_program: FlowProgram(Some("/home/li/.local/bin/flow".into())),
                claude_credentials: ClaudeCredentials(None),
            },
        ),
    ];
    for (hand, generated) in values {
        let through_hand = rkyv::to_bytes::<rkyv::rancor::Error>(&hand).unwrap();
        let through_generated = rkyv::to_bytes::<rkyv::rancor::Error>(&generated).unwrap();
        assert_eq!(through_hand.as_slice(), through_generated.as_slice());
        assert_eq!(
            rkyv::from_bytes::<Programs, rkyv::rancor::Error>(&through_hand).unwrap(),
            generated
        );
        assert_eq!(
            rkyv::from_bytes::<HandPrograms, rkyv::rancor::Error>(&through_generated).unwrap(),
            hand
        );
    }
}

#[test]
fn the_cli_configuration_and_its_refusals_read_and_print_as_the_hand_ones() {
    let hand = HandCliConfiguration { place_name: "main".into() };
    let generated = CliConfiguration { place_name: PlaceName("main".into()) };
    assert_eq!(text(&hand), text(&generated));
    assert_eq!(text(&generated), "{ main }");
    assert_eq!(
        Potential::<CliConfiguration>::from("{ main }").actualize(&mut budget()).unwrap(),
        generated
    );
    let refused = |path: &str, reason: &str| RefusedFile {
        configuration_path: ConfigurationPath(path.into()),
        reason: Reason(reason.into()),
    };
    for (hand, generated) in [
        (
            HandConfigurationRefusal::Unlocated("neither XDG_CONFIG_HOME nor HOME is set".into()),
            ConfigurationRefusal::Unlocated("neither XDG_CONFIG_HOME nor HOME is set".into()),
        ),
        (
            HandConfigurationRefusal::Missing("/c/nexus/energy-nexus.datom".into()),
            ConfigurationRefusal::Missing("/c/nexus/energy-nexus.datom".into()),
        ),
        (
            HandConfigurationRefusal::Unreadable {
                path: "/c/nexus/energy-nexus.datom".into(),
                reason: "Is a directory (os error 21)".into(),
            },
            ConfigurationRefusal::Unreadable(refused(
                "/c/nexus/energy-nexus.datom",
                "Is a directory (os error 21)",
            )),
        ),
        (
            HandConfigurationRefusal::Invalid {
                path: "/c/nexus/energy-nexus.datom".into(),
                reason: "\"..\" is not a place name".into(),
            },
            ConfigurationRefusal::Invalid(refused(
                "/c/nexus/energy-nexus.datom",
                "\"..\" is not a place name",
            )),
        ),
    ] {
        assert_eq!(text(&hand), text(&generated));
        assert_eq!(generated.to_string(), text(&hand));
    }
}
