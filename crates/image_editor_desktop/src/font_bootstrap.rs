//! Mandatory bundled-font startup validation and `egui` registration.
//!
//! The desktop host invokes this module before constructing its editable
//! workspace. Its public failure value intentionally retains only a stable,
//! user-safe category: it never carries a path, parser error, or font bytes.

use std::{
    collections::BTreeMap,
    fs,
    panic::{AssertUnwindSafe, catch_unwind},
    path::{Path, PathBuf},
    sync::Arc,
};

use eframe::egui::{self, FontDefinitions, FontFamily};

use crate::BUNDLED_FONT_SOURCE_PATH;

const BUNDLED_FONT_KEY: &str = "image-editor-bundled-cjk";
const SAFE_STARTUP_ERROR_MESSAGE: &str =
    "A required application font is unavailable. The editor has not started.";

/// Categorizes a mandatory-font startup failure without exposing sensitive or
/// untrusted resource details to the startup UI.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FontBootstrapFailure {
    ResourceUnavailable,
    MalformedData,
    RegistrationRejected,
}

impl FontBootstrapFailure {
    /// Returns the ASCII-only message rendered by the non-editable fallback
    /// application, so it remains safe even when the packaged font is absent.
    pub const fn safe_message(self) -> &'static str {
        SAFE_STARTUP_ERROR_MESSAGE
    }
}

/// Resolves, parses, and registers the mandatory packaged UI font.
///
/// `install` is deliberately the only operation the native creation callback
/// needs before it constructs `DesktopApp`.
pub struct FontBootstrapper {
    resource_path: PathBuf,
}

impl FontBootstrapper {
    /// Resolves the resource relative to the installed executable. Debug builds
    /// may use the checked-in resource so `cargo run` exercises the same bytes
    /// without requiring a package staging step.
    pub fn for_current_package() -> Result<Self, FontBootstrapFailure> {
        let executable =
            std::env::current_exe().map_err(|_| FontBootstrapFailure::ResourceUnavailable)?;
        let installed_resource = installed_resource_path(&executable)?;
        if installed_resource.is_file() {
            return Ok(Self::from_resource_path(installed_resource));
        }

        #[cfg(debug_assertions)]
        {
            let development_resource =
                Path::new(env!("CARGO_MANIFEST_DIR")).join(BUNDLED_FONT_SOURCE_PATH);
            if development_resource.is_file() {
                return Ok(Self::from_resource_path(development_resource));
            }
        }

        Err(FontBootstrapFailure::ResourceUnavailable)
    }

    /// Creates a bootstrapper for one already-resolved package resource.
    /// This is public to permit package smoke harnesses to inject a staged
    /// resource without changing process-global executable discovery.
    pub fn from_resource_path(resource_path: impl Into<PathBuf>) -> Self {
        Self {
            resource_path: resource_path.into(),
        }
    }

    /// Reads and validates the complete resource before constructing the
    /// definitions supplied to `egui`.
    pub fn font_definitions(&self) -> Result<FontDefinitions, FontBootstrapFailure> {
        let bytes =
            fs::read(&self.resource_path).map_err(|_| FontBootstrapFailure::ResourceUnavailable)?;
        ttf_parser::Face::parse(&bytes, 0).map_err(|_| FontBootstrapFailure::MalformedData)?;

        let mut definitions = FontDefinitions::default();
        definitions.font_data.insert(
            BUNDLED_FONT_KEY.to_owned(),
            Arc::new(egui::FontData::from_owned(bytes)),
        );
        for family in [FontFamily::Proportional, FontFamily::Monospace] {
            let choices = definitions.families.entry(family).or_default();
            choices.retain(|choice| choice != BUNDLED_FONT_KEY);
            // Every entry following this one remains the normal egui fallback
            // chain for code points outside the mandatory packaged coverage.
            choices.insert(0, BUNDLED_FONT_KEY.to_owned());
        }
        Ok(definitions)
    }

    /// Registers the bundled face with the creation context. `egui` exposes
    /// registration as an infallible API, so a panic is contained and converted
    /// into the safe startup-error state rather than allowing workspace drawing.
    pub fn install(&self, context: &egui::Context) -> Result<(), FontBootstrapFailure> {
        let definitions = self.font_definitions()?;
        catch_unwind(AssertUnwindSafe(|| context.set_fonts(definitions)))
            .map_err(|_| FontBootstrapFailure::RegistrationRejected)
    }
}

fn installed_resource_path(executable: &Path) -> Result<PathBuf, FontBootstrapFailure> {
    let binary_directory = executable
        .parent()
        .ok_or(FontBootstrapFailure::ResourceUnavailable)?;

    #[cfg(target_os = "macos")]
    {
        let contents_directory = binary_directory
            .parent()
            .ok_or(FontBootstrapFailure::ResourceUnavailable)?;
        Ok(contents_directory
            .join("Resources")
            .join(BUNDLED_FONT_SOURCE_PATH))
    }

    #[cfg(not(target_os = "macos"))]
    {
        Ok(binary_directory.join(BUNDLED_FONT_SOURCE_PATH))
    }
}

#[cfg(test)]
mod tests {
    use super::{BUNDLED_FONT_KEY, FontBootstrapFailure, FontBootstrapper};
    use eframe::egui::FontFamily;

    fn packaged_font() -> FontBootstrapper {
        FontBootstrapper::from_resource_path(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(crate::BUNDLED_FONT_SOURCE_PATH),
        )
    }

    #[test]
    fn packaged_font_is_registered_before_normal_family_fallbacks() {
        let definitions = packaged_font()
            .font_definitions()
            .expect("checked-in bundled font must be parseable");

        assert!(definitions.font_data.contains_key(BUNDLED_FONT_KEY));
        for family in [FontFamily::Proportional, FontFamily::Monospace] {
            let choices = definitions
                .families
                .get(&family)
                .expect("egui default family must be configured");
            assert_eq!(choices.first().map(String::as_str), Some(BUNDLED_FONT_KEY));
            assert!(
                choices.len() > 1,
                "normal egui fallbacks must remain available"
            );
        }
    }

    #[test]
    fn malformed_resource_maps_to_safe_startup_failure_without_details() {
        let path =
            std::env::temp_dir().join(format!("image-editor-invalid-font-{}", std::process::id()));
        std::fs::write(&path, b"not an OpenType font").expect("test resource is written");
        let result = FontBootstrapper::from_resource_path(&path).font_definitions();
        std::fs::remove_file(&path).expect("test resource is removed");

        assert_eq!(result, Err(FontBootstrapFailure::MalformedData));
        assert_eq!(
            FontBootstrapFailure::MalformedData.safe_message(),
            "A required application font is unavailable. The editor has not started."
        );
    }
}
