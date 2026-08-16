//! Build-time package capability metadata for supported Image Editor targets.
//!
//! This module describes package expectations only. The desktop host still
//! probes codecs and dialog services at startup before enabling an operation.

use std::{error::Error, fmt, fs, path::Path};

/// The Rust toolchain recorded in every distributable package manifest.
pub const LOCKED_RUST_TOOLCHAIN: &str = "1.85.0";

/// The target-specific package profiles supported by the release build.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PackageProfile {
    MacosAarch64,
    LinuxX86_64Portal,
}

impl PackageProfile {
    /// Parses the stable profile name used by release automation.
    pub fn parse(name: &str) -> Result<Self, ManifestError> {
        match name {
            "macos-aarch64" => Ok(Self::MacosAarch64),
            "linux-x86_64-portal" => Ok(Self::LinuxX86_64Portal),
            _ => Err(ManifestError::UnsupportedProfile(name.to_owned())),
        }
    }

    /// Returns the Cargo target triple recorded by this package profile.
    pub const fn target(self) -> &'static str {
        match self {
            Self::MacosAarch64 => "aarch64-apple-darwin",
            Self::LinuxX86_64Portal => "x86_64-unknown-linux-gnu",
        }
    }

    /// Returns the supported operating-system name recorded by this profile.
    pub const fn platform(self) -> &'static str {
        match self {
            Self::MacosAarch64 => "macos",
            Self::LinuxX86_64Portal => "linux",
        }
    }

    /// Generates machine-readable capability metadata for one package artifact.
    ///
    /// Build and runtime-provider expectations remain separate from capability
    /// truth. The desktop host always receives truth from startup probes.
    pub fn capabilities_json(self) -> String {
        let (artifact, dialog_backend, dialog_feature, dialog_dependencies) = match self {
            Self::MacosAarch64 => ("app-dmg-or-pkg", "rfd-macos", "macos-dialogs", &[][..]),
            Self::LinuxX86_64Portal => (
                "native-package-or-appimage",
                "rfd-xdg-portal",
                "xdg-portal",
                &[
                    "org.freedesktop.portal.Desktop",
                    "an XDG portal implementation",
                ][..],
            ),
        };

        format!(
            concat!(
                "{{\n",
                "  \"schema_version\": 1,\n",
                "  \"package\": {{\n",
                "    \"platform\": \"{}\",\n",
                "    \"target\": \"{}\",\n",
                "    \"artifact\": \"{}\"\n",
                "  }},\n",
                "  \"build\": {{\n",
                "    \"locked_rust_toolchain\": \"{}\",\n",
                "    \"locked_dependency_graph\": \"Cargo.lock\",\n",
                "    \"cargo_features\": {}\n",
                "  }},\n",
                "  \"compiled_portable_codecs\": {{\n",
                "    \"provider\": \"image-rs\",\n",
                "    \"formats\": [\"jpeg\", \"png\", \"tiff\"]\n",
                "  }},\n",
                "  \"optional_heic_provider\": {{\n",
                "    \"provider\": \"libheif-rs/libheif\",\n",
                "    \"compile_feature\": \"heic\",\n",
                "    \"runtime_dependencies\": [\"libheif\", \"libheif codec plugins\"],\n",
                "    \"bundled\": false,\n",
                "    \"optional\": true\n",
                "  }},\n",
                "  \"dialog_backend\": {{\n",
                "    \"provider\": \"{}\",\n",
                "    \"compile_feature\": \"{}\",\n",
                "    \"optional_runtime_dependencies\": {}\n",
                "  }},\n",
                "  \"runtime_capabilities\": {{\n",
                "    \"source\": \"startup-probe\",\n",
                "    \"package_metadata_is_startup_truth\": false\n",
                "  }}\n",
                "}}\n"
            ),
            self.platform(),
            self.target(),
            artifact,
            LOCKED_RUST_TOOLCHAIN,
            json_array(self.cargo_features()),
            dialog_backend,
            dialog_feature,
            json_array(dialog_dependencies),
        )
    }

    /// Writes this profile's manifest, creating only the requested output path.
    pub fn write_capabilities_json(self, output: &Path) -> Result<(), ManifestError> {
        if let Some(parent) = output.parent() {
            fs::create_dir_all(parent).map_err(|source| ManifestError::Write {
                output: output.to_owned(),
                source,
            })?;
        }
        fs::write(output, self.capabilities_json()).map_err(|source| ManifestError::Write {
            output: output.to_owned(),
            source,
        })
    }

    const fn cargo_features(self) -> &'static [&'static str] {
        match self {
            Self::MacosAarch64 => &["native-window", "portable-codecs", "macos-dialogs"],
            Self::LinuxX86_64Portal => &["native-window", "portable-codecs", "xdg-portal"],
        }
    }
}

/// A package profile lookup or manifest-write error.
#[derive(Debug)]
pub enum ManifestError {
    UnsupportedProfile(String),
    Write {
        output: std::path::PathBuf,
        source: std::io::Error,
    },
}

impl fmt::Display for ManifestError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedProfile(name) => {
                write!(formatter, "unsupported package profile: {name}")
            }
            Self::Write { output, source } => {
                write!(formatter, "could not write {}: {source}", output.display())
            }
        }
    }
}

impl Error for ManifestError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::UnsupportedProfile(_) => None,
            Self::Write { source, .. } => Some(source),
        }
    }
}

fn json_array(values: &[&str]) -> String {
    let joined = values
        .iter()
        .map(|value| format!("\"{value}\""))
        .collect::<Vec<_>>()
        .join(", ");
    format!("[{joined}]")
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::{LOCKED_RUST_TOOLCHAIN, PackageProfile};

    #[test]
    fn macos_manifest_records_target_and_optional_provider_without_claiming_runtime_capabilities() {
        let manifest = PackageProfile::MacosAarch64.capabilities_json();

        assert!(manifest.contains("\"target\": \"aarch64-apple-darwin\""));
        assert!(manifest.contains("\"provider\": \"rfd-macos\""));
        assert!(manifest.contains("\"compile_feature\": \"heic\""));
        assert!(manifest.contains("\"optional\": true"));
        assert!(manifest.contains("\"source\": \"startup-probe\""));
        assert!(manifest.contains("\"package_metadata_is_startup_truth\": false"));
    }

    #[test]
    fn linux_manifest_records_portal_dependencies_as_optional_runtime_dependencies() {
        let manifest = PackageProfile::LinuxX86_64Portal.capabilities_json();

        assert!(manifest.contains("\"target\": \"x86_64-unknown-linux-gnu\""));
        assert!(manifest.contains("\"provider\": \"rfd-xdg-portal\""));
        assert!(manifest.contains("org.freedesktop.portal.Desktop"));
        assert!(manifest.contains("\"formats\": [\"jpeg\", \"png\", \"tiff\"]"));
    }

    #[test]
    fn generated_manifest_uses_the_repository_locked_toolchain_and_writes_requested_path() {
        let toolchain_file = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../rust-toolchain.toml"
        ));
        assert!(toolchain_file.contains(&format!("channel = \"{LOCKED_RUST_TOOLCHAIN}\"")));

        let output = std::env::temp_dir().join(format!(
            "image-editor-capabilities-test-{}-{}.json",
            std::process::id(),
            std::thread::current().name().unwrap_or("unnamed")
        ));
        PackageProfile::LinuxX86_64Portal
            .write_capabilities_json(&output)
            .expect("the manifest should be generated at its requested path");
        let written = fs::read_to_string(&output).expect("generated manifest should be readable");
        assert_eq!(
            written,
            PackageProfile::LinuxX86_64Portal.capabilities_json()
        );
        fs::remove_file(output).expect("test-created manifest should be removable");
    }

    #[test]
    fn unsupported_profiles_are_rejected() {
        assert!(PackageProfile::parse("linux-arm64").is_err());
    }
}
