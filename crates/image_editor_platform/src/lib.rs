//! Platform integration boundary.
//!
//! Linux dialog backends are opt-in Cargo features. Their presence here does
//! not assert that a portal service or GTK runtime will be usable at startup;
//! runtime probes own that decision.

use std::{
    fs, io,
    path::{Path, PathBuf},
    sync::Mutex,
};

#[cfg(unix)]
use std::os::unix::fs::MetadataExt;

use image_editor_core::{
    AbsolutePath, CapabilityName, ExportTargetResolution, FileIdentity, ImageFormat,
    PlatformCapability, SourceIdentity,
};
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

/// An error returned by a platform dialog backend without exposing native details.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DialogFailure {
    summary: String,
}

impl DialogFailure {
    pub fn new(summary: impl Into<String>) -> Self {
        Self {
            summary: summary.into(),
        }
    }

    pub fn summary(&self) -> &str {
        &self.summary
    }
}

/// Configuration for a native local-folder picker.
///
/// The application never uses this adapter for multi-folder selection because a
/// selected folder replaces the complete browsing collection atomically.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FolderDialogRequest {
    allow_multiple: bool,
}

impl FolderDialogRequest {
    pub const fn single_folder() -> Self {
        Self {
            allow_multiple: false,
        }
    }

    pub const fn allow_multiple(self) -> bool {
        self.allow_multiple
    }
}

/// Configuration for a native local export-path picker.
///
/// A request carries one already-capability-checked format, so a dialog cannot
/// accidentally advertise a format that the current session cannot encode.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SaveDialogRequest {
    format: ImageFormat,
    allow_multiple: bool,
}

impl SaveDialogRequest {
    pub const fn single_path(format: ImageFormat) -> Self {
        Self {
            format,
            allow_multiple: false,
        }
    }

    pub const fn format(self) -> ImageFormat {
        self.format
    }

    pub const fn allow_multiple(self) -> bool {
        self.allow_multiple
    }
}

/// Native dialog operations supplied by a selected platform backend.
///
/// Probes are deliberately separate from presentation: a backend can disappear
/// after startup, and `PlatformDialogs` then downgrades only the failed
/// operation for the rest of the session.
pub trait PlatformDialogBackend: Send + Sync {
    fn probe_folder_picker(&self) -> std::result::Result<String, DialogFailure>;
    fn probe_save_picker(&self) -> std::result::Result<String, DialogFailure>;
    fn pick_folder(
        &self,
        request: FolderDialogRequest,
    ) -> std::result::Result<Option<PathBuf>, DialogFailure>;
    fn pick_export_target(
        &self,
        request: SaveDialogRequest,
    ) -> std::result::Result<Option<PathBuf>, DialogFailure>;
}

/// Capability-aware adapter for the native folder and export dialogs.
///
/// Detection always probes the folder picker before the save picker. A failed
/// invocation downgrades only its corresponding capability; unrelated editing
/// and dialog operations remain available.
pub struct PlatformDialogs<B> {
    backend: B,
    capabilities: Mutex<DialogCapabilities>,
}

#[derive(Clone, Debug)]
struct DialogCapabilities {
    folder_picker: PlatformCapability,
    save_picker: PlatformCapability,
}

impl<B: PlatformDialogBackend> PlatformDialogs<B> {
    pub fn detect(backend: B) -> Self {
        // This order is part of the startup contract: callers receive both
        // capability values before they can enable either dependent command.
        let folder_picker = probe_capability(backend.probe_folder_picker());
        let save_picker = probe_capability(backend.probe_save_picker());
        Self {
            backend,
            capabilities: Mutex::new(DialogCapabilities {
                folder_picker,
                save_picker,
            }),
        }
    }

    pub fn folder_picker_available(&self) -> PlatformCapability {
        self.capabilities().folder_picker
    }

    pub fn save_picker_available(&self) -> PlatformCapability {
        self.capabilities().save_picker
    }

    pub fn pick_folder(&self) -> Result<Option<AbsolutePath>> {
        if !self.folder_picker_available().is_available() {
            return Err(unavailable_dialog_error(CapabilityName::FolderPicker));
        }

        match self
            .backend
            .pick_folder(FolderDialogRequest::single_folder())
        {
            Ok(selection) => selection
                .map(|path| absolute_path_from_native(path, CapabilityName::FolderPicker))
                .transpose(),
            Err(error) => {
                self.downgrade(CapabilityName::FolderPicker, &error);
                Err(dialog_error(CapabilityName::FolderPicker, error))
            }
        }
    }

    pub fn pick_export_target(&self, format: ImageFormat) -> Result<Option<AbsolutePath>> {
        if !self.save_picker_available().is_available() {
            return Err(unavailable_dialog_error(CapabilityName::SavePicker));
        }

        match self
            .backend
            .pick_export_target(SaveDialogRequest::single_path(format))
        {
            Ok(selection) => selection
                .map(|path| absolute_path_from_native(path, CapabilityName::SavePicker))
                .transpose(),
            Err(error) => {
                self.downgrade(CapabilityName::SavePicker, &error);
                Err(dialog_error(CapabilityName::SavePicker, error))
            }
        }
    }

    fn capabilities(&self) -> DialogCapabilities {
        self.capabilities
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    fn downgrade(&self, capability: CapabilityName, error: &DialogFailure) {
        let unavailable = PlatformCapability::unavailable(error.summary().to_owned());
        let mut capabilities = self
            .capabilities
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        match capability {
            CapabilityName::FolderPicker => capabilities.folder_picker = unavailable,
            CapabilityName::SavePicker => capabilities.save_picker = unavailable,
            CapabilityName::FormatDecode(_) | CapabilityName::FormatEncode(_) => {
                unreachable!("native dialog adapters only manage dialog capabilities")
            }
        }
    }
}

fn probe_capability(result: std::result::Result<String, DialogFailure>) -> PlatformCapability {
    match result {
        Ok(backend) if !backend.is_empty() => PlatformCapability::available(backend),
        Ok(_) => PlatformCapability::unavailable("dialog backend probe returned no backend name"),
        Err(error) => PlatformCapability::unavailable(error.summary().to_owned()),
    }
}

fn absolute_path_from_native(path: PathBuf, capability: CapabilityName) -> Result<AbsolutePath> {
    let path =
        path.into_os_string()
            .into_string()
            .map_err(|_| ApplicationError::PlatformOperation {
                capability,
                cause: SafeError::new(
                    ErrorCategory::PlatformIntegration,
                    "native dialog returned a non-UTF-8 path",
                ),
            })?;
    AbsolutePath::new(path).map_err(|error| ApplicationError::PlatformOperation {
        capability,
        cause: SafeError::new(
            ErrorCategory::PlatformIntegration,
            format!("native dialog returned an invalid path: {error}"),
        ),
    })
}

fn unavailable_dialog_error(capability: CapabilityName) -> ApplicationError {
    ApplicationError::PlatformOperation {
        capability,
        cause: SafeError::new(
            ErrorCategory::PlatformIntegration,
            "platform dialog capability is unavailable",
        ),
    }
}

fn dialog_error(capability: CapabilityName, error: DialogFailure) -> ApplicationError {
    ApplicationError::PlatformOperation {
        capability,
        cause: SafeError::new(ErrorCategory::PlatformIntegration, error.summary),
    }
}

/// `rfd` implementation enabled only for a selected native dialog backend.
#[cfg(any(feature = "macos-dialogs", feature = "xdg-portal", feature = "gtk"))]
#[derive(Clone, Copy, Debug, Default)]
pub struct RfdDialogBackend;

#[cfg(any(feature = "macos-dialogs", feature = "xdg-portal", feature = "gtk"))]
impl PlatformDialogBackend for RfdDialogBackend {
    fn probe_folder_picker(&self) -> std::result::Result<String, DialogFailure> {
        native_backend_name().map(str::to_owned)
    }

    fn probe_save_picker(&self) -> std::result::Result<String, DialogFailure> {
        native_backend_name().map(str::to_owned)
    }

    fn pick_folder(
        &self,
        request: FolderDialogRequest,
    ) -> std::result::Result<Option<PathBuf>, DialogFailure> {
        debug_assert!(!request.allow_multiple());
        Ok(rfd::FileDialog::new().pick_folder())
    }

    fn pick_export_target(
        &self,
        request: SaveDialogRequest,
    ) -> std::result::Result<Option<PathBuf>, DialogFailure> {
        debug_assert!(!request.allow_multiple());
        Ok(file_dialog_for(request.format()).save_file())
    }
}

#[cfg(any(feature = "macos-dialogs", feature = "xdg-portal", feature = "gtk"))]
fn native_backend_name() -> std::result::Result<&'static str, DialogFailure> {
    #[cfg(all(target_os = "macos", feature = "macos-dialogs"))]
    {
        return Ok("rfd-macos");
    }
    // rfd selects the portal backend whenever its `xdg-portal` feature is
    // linked, so do not claim that a simultaneously linked GTK feature is a
    // fallback. The portal must own the FileChooser interface before either
    // request can be enabled.
    #[cfg(all(target_os = "linux", feature = "xdg-portal"))]
    {
        probe_xdg_portal_file_chooser()?;
        return Ok("rfd-xdg-portal");
    }
    #[cfg(all(target_os = "linux", not(feature = "xdg-portal"), feature = "gtk"))]
    {
        // GTK is linked by rfd at process startup. Reaching this branch means
        // the intentionally packaged GTK adapter and its required libraries
        // are available to the selected backend.
        return Ok("rfd-gtk3");
    }
    #[allow(unreachable_code)]
    Err(DialogFailure::new(
        "no native dialog backend is linked for this platform",
    ))
}

/// Verifies both the session portal service and its file-chooser interface.
///
/// A live D-Bus session alone is insufficient: the portal service can be
/// present while no implementation provides `org.freedesktop.portal.FileChooser`.
/// This runs during startup capability detection, before any picker request is
/// accepted, and all details are collapsed into a safe availability message.
#[cfg(all(target_os = "linux", feature = "xdg-portal"))]
fn probe_xdg_portal_file_chooser() -> std::result::Result<(), DialogFailure> {
    let connection = zbus::blocking::Connection::session().map_err(|_| {
        DialogFailure::new("could not connect to the XDG Desktop Portal session service")
    })?;
    let reply = connection
        .call_method(
            Some("org.freedesktop.portal.Desktop"),
            "/org/freedesktop/portal/desktop",
            Some("org.freedesktop.DBus.Introspectable"),
            "Introspect",
            &(),
        )
        .map_err(|_| {
            DialogFailure::new("the XDG Desktop Portal file chooser service is unavailable")
        })?;
    let introspection: String = reply.body().deserialize().map_err(|_| {
        DialogFailure::new("the XDG Desktop Portal returned an invalid file chooser probe")
    })?;
    if introspection.contains("org.freedesktop.portal.FileChooser") {
        Ok(())
    } else {
        Err(DialogFailure::new(
            "the XDG Desktop Portal has no file chooser implementation",
        ))
    }
}

#[cfg(any(feature = "macos-dialogs", feature = "xdg-portal", feature = "gtk"))]
fn file_dialog_for(format: ImageFormat) -> rfd::FileDialog {
    match format {
        ImageFormat::Jpeg => rfd::FileDialog::new().add_filter("JPEG image", &["jpg", "jpeg"]),
        ImageFormat::Png => rfd::FileDialog::new().add_filter("PNG image", &["png"]),
        ImageFormat::Tiff => rfd::FileDialog::new().add_filter("TIFF image", &["tif", "tiff"]),
        ImageFormat::Heic => rfd::FileDialog::new().add_filter("HEIC image", &["heic"]),
    }
}

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
