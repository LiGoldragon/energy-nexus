//! What an Energy CLI knows besides its datom (vision:nexus-v1): one
//! configuration datom at `$XDG_CONFIG_HOME/nexus/energy-nexus.datom`
//! (`~/.config/nexus/energy-nexus.datom` when the variable is unset or
//! empty), `CliConfiguration.{ PlaceName }`, e.g. `{ main }`: the place first,
//! more as the CLI gains options. Every Energy CLI, `energy-nexus`,
//! `energy-nexus-meta` and `energy-pace`, reads the same file; no place is
//! ever typed. A configuration that is missing, cannot be read or names no
//! place is a typed refusal naming the file, and the CLI reaches nothing.
use datom_codec::{Actualizing, Datomizable, Potential};
use energy_nexus_wire::{Place, PlaceName};
use std::ffi::OsStr;
use std::path::PathBuf;

#[derive(datom_codec::Datomizable, datom_codec::Composing, Clone, Debug, PartialEq, Eq)]
pub struct CliConfiguration {
    pub place_name: String,
}

/// Why a CLI reaches nothing: its configuration could not be found or read.
#[derive(datom_codec::Datomizable, datom_codec::Composing, Clone, Debug, PartialEq, Eq)]
pub enum ConfigurationRefusal {
    /// Neither XDG_CONFIG_HOME nor HOME gives an absolute directory.
    Unlocated(String),
    Missing(String),
    Unreadable { path: String, reason: String },
    Invalid { path: String, reason: String },
}

impl std::fmt::Display for ConfigurationRefusal {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        use protos::{Protosizable, Textualizable};
        let text = self.datomize(Vec::new()).protosize().textualize();
        formatter.write_str(&text)
    }
}

/// The file an Energy CLI's configuration is read from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConfigurationFile {
    pub path: PathBuf,
}

impl ConfigurationFile {
    /// The Nexus the configuration is named after.
    pub const NEXUS_NAME: &'static str = "energy-nexus";

    /// `<configuration home>/nexus/energy-nexus.datom`: the configuration
    /// home is `XDG_CONFIG_HOME`, else, unset or empty, `<HOME>/.config`;
    /// either must be absolute.
    pub fn under(
        configuration_home: Option<&OsStr>,
        home: Option<&OsStr>,
    ) -> Result<Self, ConfigurationRefusal> {
        let configuration_home = configuration_home.filter(|directory| !directory.is_empty());
        let directory = match (configuration_home, home) {
            (Some(directory), _) => PathBuf::from(directory),
            (None, Some(home)) => PathBuf::from(home).join(".config"),
            (None, None) => {
                return Err(ConfigurationRefusal::Unlocated(
                    "neither XDG_CONFIG_HOME nor HOME is set".into(),
                ))
            }
        };
        if !directory.is_absolute() {
            return Err(ConfigurationRefusal::Unlocated(format!(
                "the configuration home {} is not absolute",
                directory.display()
            )));
        }
        Ok(Self {
            path: directory
                .join("nexus")
                .join(format!("{}.datom", Self::NEXUS_NAME)),
        })
    }

    /// The file this process's environment names.
    pub fn of_process() -> Result<Self, ConfigurationRefusal> {
        Self::under(
            std::env::var_os("XDG_CONFIG_HOME").as_deref(),
            std::env::var_os("HOME").as_deref(),
        )
    }

    /// The configuration the file holds; its place name is one path
    /// component.
    pub fn read(&self) -> Result<CliConfiguration, ConfigurationRefusal> {
        let path = self.path.to_string_lossy().into_owned();
        let source = match std::fs::read_to_string(&self.path) {
            Ok(source) => source,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Err(ConfigurationRefusal::Missing(path))
            }
            Err(error) => {
                return Err(ConfigurationRefusal::Unreadable {
                    path,
                    reason: error.to_string(),
                })
            }
        };
        let mut budget = datom_codec::Budget {
            remaining: 65_536,
            reader: protos::ReaderBudget::try_from(65_536).expect("a constant reader budget"),
            depth: 0,
            maximum_depth: 64,
        };
        let configuration = Potential::<CliConfiguration>::from(source.trim())
            .actualize(&mut budget)
            .map_err(|error| ConfigurationRefusal::Invalid {
                path: path.clone(),
                reason: format!("{error:?}"),
            })?;
        let name = &configuration.place_name;
        if name.is_empty() || name == "." || name == ".." || name.contains('/') {
            return Err(ConfigurationRefusal::Invalid {
                path,
                reason: format!("{name:?} is not a place name"),
            });
        }
        Ok(configuration)
    }
}

/// The place an Energy CLI reaches: its configuration's, under the standard
/// XDG roots.
pub trait ConfiguredPlace: Sized {
    fn configured() -> Result<Self, String>;
}

impl ConfiguredPlace for Place {
    fn configured() -> Result<Self, String> {
        let configuration = ConfigurationFile::of_process()
            .and_then(|file| file.read())
            .map_err(|refusal| refusal.to_string())?;
        Place::resolve(PlaceName(configuration.place_name))
    }
}

#[cfg(test)]
mod tests {
    use super::{CliConfiguration, ConfigurationFile, ConfigurationRefusal};
    use std::ffi::OsStr;
    use std::path::Path;

    #[test]
    fn the_configuration_lies_under_the_configuration_home_else_under_home() {
        assert_eq!(
            ConfigurationFile::under(Some(OsStr::new("/c")), Some(OsStr::new("/home/li")))
                .unwrap()
                .path,
            Path::new("/c/nexus/energy-nexus.datom")
        );
        for unset in [None, Some(OsStr::new(""))] {
            assert_eq!(
                ConfigurationFile::under(unset, Some(OsStr::new("/home/li")))
                    .unwrap()
                    .path,
                Path::new("/home/li/.config/nexus/energy-nexus.datom")
            );
        }
        for (configuration_home, home) in [
            (None, None),
            (Some(OsStr::new("c")), Some(OsStr::new("/home/li"))),
        ] {
            assert!(matches!(
                ConfigurationFile::under(configuration_home, home),
                Err(ConfigurationRefusal::Unlocated(_))
            ));
        }
    }

    #[test]
    fn the_configuration_names_the_place_and_a_bad_one_is_refused_naming_its_path() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::create_dir(directory.path().join("nexus")).unwrap();
        let file = ConfigurationFile::under(Some(directory.path().as_os_str()), None).unwrap();
        let path = file.path.to_string_lossy().into_owned();
        assert_eq!(file.read(), Err(ConfigurationRefusal::Missing(path.clone())));
        let printed = ConfigurationRefusal::Missing(path.clone()).to_string();
        assert!(
            printed.starts_with("Missing.") && printed.contains(&path),
            "{printed}"
        );
        std::fs::write(&file.path, "{ main }\n").unwrap();
        assert_eq!(
            file.read(),
            Ok(CliConfiguration {
                place_name: "main".into()
            })
        );
        for invalid in ["{ main next }", "{ «..» }", "{ }", ""] {
            std::fs::write(&file.path, invalid).unwrap();
            assert!(
                matches!(file.read(), Err(ConfigurationRefusal::Invalid { path: named, .. }) if named == path),
                "{invalid:?}: {:?}",
                file.read()
            );
        }
        std::fs::remove_file(&file.path).unwrap();
        std::fs::create_dir(&file.path).unwrap();
        assert!(matches!(
            file.read(),
            Err(ConfigurationRefusal::Unreadable { path: named, .. }) if named == path
        ));
    }
}
