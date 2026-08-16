//! Build-time package capability and bundled-resource metadata for supported
//! Image Editor targets.
//!
//! This module describes package expectations only. The desktop host still
//! probes codecs and dialog services at startup before enabling an operation.

use std::{
    error::Error,
    fmt, fs,
    path::{Path, PathBuf},
};

#[cfg(feature = "native-window")]
pub mod font_bootstrap;

/// The Rust toolchain recorded in every distributable package manifest.
pub const LOCKED_RUST_TOOLCHAIN: &str = "1.85.0";

/// The repository-relative source location of the font shipped in every package.
pub const BUNDLED_FONT_SOURCE_PATH: &str = "resources/fonts/NotoSansCJKsc-Regular.otf";
/// The approved name table family of the mandatory Chinese-capable font.
pub const BUNDLED_FONT_NAME: &str = "Noto Sans CJK SC";
/// The vetted upstream font version represented by the checked-in resource.
pub const BUNDLED_FONT_VERSION: &str = "2.004";
/// The SPDX identifier governing the bundled font resource.
pub const BUNDLED_FONT_LICENSE: &str = "OFL-1.1";
/// SHA-256 of the exact checked-in Noto Sans CJK SC Regular OTF resource.
pub const BUNDLED_FONT_SHA256: &str =
    "2c76254f6fc379fddfce0a7e84fb5385bb135d3e399294f6eeb6680d0365b74b";

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

    /// Returns the resource path inside this target's installable package.
    pub const fn bundled_font_resource_path(self) -> &'static str {
        match self {
            Self::MacosAarch64 => {
                "Image Editor.app/Contents/Resources/resources/fonts/NotoSansCJKsc-Regular.otf"
            }
            Self::LinuxX86_64Portal => "resources/fonts/NotoSansCJKsc-Regular.otf",
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
                "  \"bundled_font\": {},\n",
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
            self.bundled_font_json(),
            dialog_backend,
            dialog_feature,
            json_array(dialog_dependencies),
        )
    }

    /// Generates the package-level resource inventory shipped beside the
    /// capability manifest.
    pub fn package_metadata_json(self) -> String {
        format!(
            concat!(
                "{{\n",
                "  \"schema_version\": 1,\n",
                "  \"platform\": \"{}\",\n",
                "  \"target\": \"{}\",\n",
                "  \"resources\": [{}]\n",
                "}}\n"
            ),
            self.platform(),
            self.target(),
            self.bundled_font_json(),
        )
    }

    /// Generates the release license inventory for all mandatory resources.
    pub fn release_licenses_json(self) -> String {
        format!(
            concat!(
                "{{\n",
                "  \"schema_version\": 1,\n",
                "  \"licenses\": [\n",
                "    {}\n",
                "  ]\n",
                "}}\n"
            ),
            self.bundled_font_json(),
        )
    }

    /// Generates a minimal SPDX 2.3 SBOM for the bundled font file.
    pub fn sbom_json(self) -> String {
        format!(
            concat!(
                "{{\n",
                "  \"spdxVersion\": \"SPDX-2.3\",\n",
                "  \"dataLicense\": \"CC0-1.0\",\n",
                "  \"SPDXID\": \"SPDXRef-DOCUMENT\",\n",
                "  \"name\": \"image-editor-bundled-resources-{}\",\n",
                "  \"documentNamespace\": \"https://example.invalid/image-editor/sbom/{}\",\n",
                "  \"creationInfo\": {{\n",
                "    \"creators\": [\"Tool: image_editor_desktop generate-capabilities\"]\n",
                "  }},\n",
                "  \"files\": [\n",
                "    {{\n",
                "      \"SPDXID\": \"SPDXRef-File-NotoSansCJKsc-Regular\",\n",
                "      \"fileName\": \"./{}\",\n",
                "      \"checksums\": [{{\"algorithm\": \"SHA256\", \"checksumValue\": \"{}\"}}],\n",
                "      \"licenseConcluded\": \"{}\",\n",
                "      \"licenseInfoInFiles\": [\"{}\"],\n",
                "      \"comment\": \"font_name={}; font_version={}; mandatory_bundled_resource=true\"\n",
                "    }}\n",
                "  ]\n",
                "}}\n"
            ),
            self.target(),
            self.target(),
            self.bundled_font_resource_path(),
            BUNDLED_FONT_SHA256,
            BUNDLED_FONT_LICENSE,
            BUNDLED_FONT_LICENSE,
            BUNDLED_FONT_NAME,
            BUNDLED_FONT_VERSION,
        )
    }

    /// Writes the full package metadata set and stages the mandatory font into
    /// the profile-specific application resource path beneath `output`'s parent.
    pub fn write_package_metadata(self, output: &Path) -> Result<(), ManifestError> {
        let package_root = output.parent().unwrap_or_else(|| Path::new("."));
        fs::create_dir_all(package_root).map_err(|source| ManifestError::Write {
            output: package_root.to_owned(),
            source,
        })?;

        self.stage_bundled_font(package_root)?;
        fs::write(output, self.capabilities_json()).map_err(|source| ManifestError::Write {
            output: output.to_owned(),
            source,
        })?;
        write_metadata_file(
            &package_root.join("package-metadata.json"),
            self.package_metadata_json(),
        )?;
        write_metadata_file(
            &package_root.join("release-licenses.json"),
            self.release_licenses_json(),
        )?;
        write_metadata_file(&package_root.join("sbom.spdx.json"), self.sbom_json())
    }

    /// Backwards-compatible name for release automation that requests a
    /// capability manifest. It also emits companion metadata and stages fonts.
    pub fn write_capabilities_json(self, output: &Path) -> Result<(), ManifestError> {
        self.write_package_metadata(output)
    }

    fn bundled_font_json(self) -> String {
        format!(
            concat!(
                "{{\"resource_path\": \"{}\", ",
                "\"name\": \"{}\", ",
                "\"version\": \"{}\", ",
                "\"sha256\": \"{}\", ",
                "\"license\": \"{}\", ",
                "\"system_font_fallback_required\": false}}"
            ),
            self.bundled_font_resource_path(),
            BUNDLED_FONT_NAME,
            BUNDLED_FONT_VERSION,
            BUNDLED_FONT_SHA256,
            BUNDLED_FONT_LICENSE,
        )
    }

    fn stage_bundled_font(self, package_root: &Path) -> Result<(), ManifestError> {
        let source = Path::new(env!("CARGO_MANIFEST_DIR")).join(BUNDLED_FONT_SOURCE_PATH);
        let destination = package_root.join(self.bundled_font_resource_path());
        let destination_parent = destination
            .parent()
            .expect("font resource path has a parent");
        fs::create_dir_all(destination_parent).map_err(|source_error| ManifestError::Write {
            output: destination_parent.to_owned(),
            source: source_error,
        })?;
        fs::copy(&source, &destination).map_err(|source_error| ManifestError::ResourceCopy {
            source,
            destination,
            source_error,
        })?;
        Ok(())
    }

    const fn cargo_features(self) -> &'static [&'static str] {
        match self {
            Self::MacosAarch64 => &["native-window", "portable-codecs", "macos-dialogs"],
            Self::LinuxX86_64Portal => &["native-window", "portable-codecs", "xdg-portal"],
        }
    }
}

fn write_metadata_file(output: &Path, contents: String) -> Result<(), ManifestError> {
    fs::write(output, contents).map_err(|source| ManifestError::Write {
        output: output.to_owned(),
        source,
    })
}

/// A package profile lookup or package-metadata-write error.
#[derive(Debug)]
pub enum ManifestError {
    UnsupportedProfile(String),
    Write {
        output: PathBuf,
        source: std::io::Error,
    },
    ResourceCopy {
        source: PathBuf,
        destination: PathBuf,
        source_error: std::io::Error,
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
            Self::ResourceCopy {
                source,
                destination,
                source_error,
            } => write!(
                formatter,
                "could not stage bundled font from {} to {}: {source_error}",
                source.display(),
                destination.display()
            ),
        }
    }
}

impl Error for ManifestError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::UnsupportedProfile(_) => None,
            Self::Write { source, .. } => Some(source),
            Self::ResourceCopy { source_error, .. } => Some(source_error),
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

    use super::{
        BUNDLED_FONT_LICENSE, BUNDLED_FONT_NAME, BUNDLED_FONT_SHA256, BUNDLED_FONT_SOURCE_PATH,
        BUNDLED_FONT_VERSION, LOCKED_RUST_TOOLCHAIN, PackageProfile,
    };

    #[test]
    fn macos_manifest_records_target_optional_provider_and_mandatory_bundled_font() {
        let manifest = PackageProfile::MacosAarch64.capabilities_json();

        assert!(manifest.contains("\"target\": \"aarch64-apple-darwin\""));
        assert!(manifest.contains("\"provider\": \"rfd-macos\""));
        assert!(manifest.contains("\"compile_feature\": \"heic\""));
        assert!(manifest.contains("\"optional\": true"));
        assert!(manifest.contains("\"source\": \"startup-probe\""));
        assert!(manifest.contains("\"package_metadata_is_startup_truth\": false"));
        assert!(manifest.contains(BUNDLED_FONT_NAME));
        assert!(manifest.contains(BUNDLED_FONT_VERSION));
        assert!(manifest.contains(BUNDLED_FONT_SHA256));
        assert!(manifest.contains(BUNDLED_FONT_LICENSE));
        assert!(manifest.contains(
            "Image Editor.app/Contents/Resources/resources/fonts/NotoSansCJKsc-Regular.otf"
        ));
        assert!(manifest.contains("\"system_font_fallback_required\": false"));
    }

    #[test]
    fn linux_manifest_records_portal_dependencies_and_mandatory_bundled_font() {
        let manifest = PackageProfile::LinuxX86_64Portal.capabilities_json();

        assert!(manifest.contains("\"target\": \"x86_64-unknown-linux-gnu\""));
        assert!(manifest.contains("\"provider\": \"rfd-xdg-portal\""));
        assert!(manifest.contains("org.freedesktop.portal.Desktop"));
        assert!(manifest.contains("\"formats\": [\"jpeg\", \"png\", \"tiff\"]"));
        assert!(
            manifest.contains("\"resource_path\": \"resources/fonts/NotoSansCJKsc-Regular.otf\"")
        );
    }

    #[test]
    fn metadata_generation_stages_font_and_records_it_in_every_required_output() {
        let root = std::env::temp_dir().join(format!(
            "image-editor-package-metadata-test-{}-{}",
            std::process::id(),
            std::thread::current().name().unwrap_or("unnamed")
        ));
        let capabilities = root.join("capabilities.json");
        let profile = PackageProfile::LinuxX86_64Portal;

        profile
            .write_package_metadata(&capabilities)
            .expect("package metadata should be generated");

        let staged_font = root.join(profile.bundled_font_resource_path());
        let source_font =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(BUNDLED_FONT_SOURCE_PATH);
        assert_eq!(
            fs::metadata(&staged_font)
                .expect("staged font should be readable")
                .len(),
            fs::metadata(&source_font)
                .expect("checked-in font should be readable")
                .len()
        );
        for output in [
            capabilities,
            root.join("package-metadata.json"),
            root.join("release-licenses.json"),
            root.join("sbom.spdx.json"),
        ] {
            let contents = fs::read_to_string(&output).expect("metadata output should be readable");
            assert!(contents.contains(profile.bundled_font_resource_path()));
            assert!(contents.contains(BUNDLED_FONT_NAME));
            assert!(contents.contains(BUNDLED_FONT_VERSION));
            assert!(contents.contains(BUNDLED_FONT_SHA256));
            assert!(contents.contains(BUNDLED_FONT_LICENSE));
        }
        fs::remove_dir_all(root).expect("test-created package metadata should be removable");
    }

    #[test]
    fn generated_manifest_uses_the_repository_locked_toolchain() {
        let toolchain_file = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../rust-toolchain.toml"
        ));
        assert!(toolchain_file.contains(&format!("channel = \"{LOCKED_RUST_TOOLCHAIN}\"")));
    }

    #[test]
    fn unsupported_profiles_are_rejected() {
        assert!(PackageProfile::parse("linux-arm64").is_err());
    }
}
