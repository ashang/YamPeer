//! Platform integration boundary.
//!
//! Linux dialog backends are opt-in Cargo features. Their presence here does
//! not assert that a portal service or GTK runtime will be usable at startup;
//! later capability probes own that decision.

use std::{fs, io, path::Path};

#[cfg(unix)]
use std::os::unix::fs::MetadataExt;

use image_editor_core::{AbsolutePath, ExportTargetResolution, FileIdentity, SourceIdentity};
pub use image_editor_core::{ApplicationError, ErrorCategory, Result, SafeError};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CompiledPlatformFeatures {
    pub xdg_portal_backend: bool,
    pub gtk_backend: bool,
}

/// Compile-time linkage facts only; adapters must still probe runtime support.
pub const COMPILED_FEATURES: CompiledPlatformFeatures = CompiledPlatformFeatures {
    xdg_portal_backend: cfg!(feature = "xdg-portal"),
    gtk_backend: cfg!(feature = "gtk"),
};

/// Resolves stable local file identities before an export writer may open.
///
/// On supported macOS and Linux targets the identity is the filesystem device
/// and inode pair, allowing hard links and symlink-resolved aliases to compare
/// equal even when their absolute paths differ.
#[derive(Clone, Copy, Debug, Default)]
pub struct LocalFileIdentityResolver;

impl LocalFileIdentityResolver {
    pub fn resolve_source_identity(&self, path: AbsolutePath) -> Result<SourceIdentity> {
        let metadata = fs::metadata(Path::new(path.as_str()))
            .map_err(|error| identity_error("source identity resolution", error))?;
        if !metadata.is_file() {
            return Err(ApplicationError::boundary(
                "source identity resolution",
                SafeError::new(
                    ErrorCategory::FileSystem,
                    "source image is not a regular file",
                ),
            ));
        }
        Ok(SourceIdentity::new(path, platform_file_identity(&metadata)))
    }

    pub fn resolve_export_target(&self, path: &AbsolutePath) -> Result<ExportTargetResolution> {
        match fs::metadata(Path::new(path.as_str())) {
            Ok(metadata) if metadata.is_file() => Ok(ExportTargetResolution::existing_regular(
                platform_file_identity(&metadata),
            )),
            Ok(_) => Ok(ExportTargetResolution::existing_other()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                Ok(ExportTargetResolution::missing())
            }
            Err(error) => Err(ApplicationError::ExportWrite {
                path: path.clone(),
                cause: SafeError::new(
                    ErrorCategory::FileSystem,
                    format!("could not inspect export target: {}", error.kind()),
                ),
            }),
        }
    }
}

#[cfg(unix)]
fn platform_file_identity(metadata: &fs::Metadata) -> Option<FileIdentity> {
    FileIdentity::new(format!("unix:{}:{}", metadata.dev(), metadata.ino())).ok()
}

#[cfg(not(unix))]
fn platform_file_identity(_: &fs::Metadata) -> Option<FileIdentity> {
    None
}

fn identity_error(operation: &'static str, error: io::Error) -> ApplicationError {
    ApplicationError::boundary(
        operation,
        SafeError::new(
            ErrorCategory::FileSystem,
            format!("could not inspect source image: {}", error.kind()),
        ),
    )
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        sync::atomic::{AtomicUsize, Ordering},
    };

    use image_editor_core::{AbsolutePath, ExportPlan, ImageFormat, Revision, TargetConflict};

    use super::LocalFileIdentityResolver;

    static NEXT_TEST_ID: AtomicUsize = AtomicUsize::new(0);

    fn temporary_path(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "image-editor-platform-{name}-{}-{}",
            std::process::id(),
            NEXT_TEST_ID.fetch_add(1, Ordering::Relaxed)
        ))
    }

    fn absolute_path(path: &std::path::Path) -> AbsolutePath {
        AbsolutePath::new(path.to_string_lossy().into_owned()).expect("temporary path is absolute")
    }

    #[test]
    fn resolver_detects_source_identity_aliases_and_existing_targets_before_planning() {
        let source_path = temporary_path("source");
        let alias_path = temporary_path("alias");
        let existing_path = temporary_path("existing");
        fs::write(&source_path, b"source bytes").unwrap();
        fs::hard_link(&source_path, &alias_path).unwrap();
        fs::write(&existing_path, b"existing bytes").unwrap();

        let resolver = LocalFileIdentityResolver;
        let source = resolver
            .resolve_source_identity(absolute_path(&source_path))
            .unwrap();
        let alias_target = resolver
            .resolve_export_target(&absolute_path(&alias_path))
            .unwrap();
        let existing_target = resolver
            .resolve_export_target(&absolute_path(&existing_path))
            .unwrap();

        let alias_error = ExportPlan::validate(
            source.clone(),
            Revision::INITIAL,
            absolute_path(&alias_path),
            ImageFormat::Png,
            alias_target,
        )
        .expect_err("a hard link to the source must be rejected before writing");
        assert!(matches!(
            alias_error,
            image_editor_core::ApplicationError::ExportTargetConflict {
                kind: TargetConflict::SourceImage,
                ..
            }
        ));

        let existing_error = ExportPlan::validate(
            source,
            Revision::INITIAL,
            absolute_path(&existing_path),
            ImageFormat::Png,
            existing_target,
        )
        .expect_err("an existing regular target must be rejected before writing");
        assert!(matches!(
            existing_error,
            image_editor_core::ApplicationError::ExportTargetConflict {
                kind: TargetConflict::ExistingLocalFile,
                ..
            }
        ));

        fs::remove_file(source_path).unwrap();
        fs::remove_file(alias_path).unwrap();
        fs::remove_file(existing_path).unwrap();
    }
}
