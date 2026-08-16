//! Mandatory bundled-font startup validation and `egui` registration.
//!
//! The desktop host invokes this module before constructing its editable
//! workspace. Its public failure value intentionally retains only a stable,
//! user-safe category: it never carries a path, parser error, or font bytes.

use std::{
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

/// Chooses the only permissible application state after font bootstrap.
///
/// Font failures are terminal for the normal workspace because rendering text
/// without the required bundled face could otherwise expose missing glyphs.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StartupRoute {
    InteractiveEditor,
    StartupAvailabilityError(FontBootstrapFailure),
}

impl StartupRoute {
    /// Maps the mandatory-font bootstrap result to a workspace-safe route.
    pub const fn from_bootstrap(result: Result<(), FontBootstrapFailure>) -> Self {
        match result {
            Ok(()) => Self::InteractiveEditor,
            Err(failure) => Self::StartupAvailabilityError(failure),
        }
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
    /// Resolves the resource for the running executable. Installable package
    /// builds always use an executable-relative resource. Ordinary Cargo builds
    /// may opt into the checked-in resource so both debug and release
    /// `cargo run` exercise the bundled bytes without a staging step.
    pub fn for_current_package() -> Result<Self, FontBootstrapFailure> {
        let executable =
            std::env::current_exe().map_err(|_| FontBootstrapFailure::ResourceUnavailable)?;
        resolve_resource_path(&executable, development_resource_path().as_deref())
            .map(Self::from_resource_path)
    }

    /// Resolves a font from an installable package layout only.
    ///
    /// Package smoke checks use this entry point to ensure a missing staged
    /// resource cannot be masked by a checked-out source tree.
    pub fn for_packaged_executable(
        executable: impl AsRef<Path>,
    ) -> Result<Self, FontBootstrapFailure> {
        resolve_resource_path(executable.as_ref(), None).map(Self::from_resource_path)
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
        self.font_definitions_with(
            |path| fs::read(path),
            |bytes| {
                ttf_parser::Face::parse(bytes, 0)
                    .map(|_| ())
                    .map_err(|_| ())
            },
        )
    }

    /// Registers the bundled face with the creation context. `egui` exposes
    /// registration as an infallible API, so a panic is contained and converted
    /// into the safe startup-error state rather than allowing workspace drawing.
    pub fn install(&self, context: &egui::Context) -> Result<(), FontBootstrapFailure> {
        self.install_with(
            |path| fs::read(path),
            |bytes| {
                ttf_parser::Face::parse(bytes, 0)
                    .map(|_| ())
                    .map_err(|_| ())
            },
            |definitions| {
                catch_unwind(AssertUnwindSafe(|| context.set_fonts(definitions)))
                    .map_err(|_| FontBootstrapFailure::RegistrationRejected)
            },
        )
    }

    // Keep I/O, parsing, and egui registration independently injectable so
    // every startup failure boundary can be tested without a native window.
    fn font_definitions_with(
        &self,
        read: impl FnOnce(&Path) -> std::io::Result<Vec<u8>>,
        validate: impl FnOnce(&[u8]) -> Result<(), ()>,
    ) -> Result<FontDefinitions, FontBootstrapFailure> {
        let bytes =
            read(&self.resource_path).map_err(|_| FontBootstrapFailure::ResourceUnavailable)?;
        validate(&bytes).map_err(|_| FontBootstrapFailure::MalformedData)?;

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

    fn install_with(
        &self,
        read: impl FnOnce(&Path) -> std::io::Result<Vec<u8>>,
        validate: impl FnOnce(&[u8]) -> Result<(), ()>,
        register: impl FnOnce(FontDefinitions) -> Result<(), FontBootstrapFailure>,
    ) -> Result<(), FontBootstrapFailure> {
        register(self.font_definitions_with(read, validate)?)
    }
}

#[cfg(feature = "development-font-fallback")]
fn development_resource_path() -> Option<PathBuf> {
    let resource = Path::new(env!("CARGO_MANIFEST_DIR")).join(BUNDLED_FONT_SOURCE_PATH);
    resource.is_file().then_some(resource)
}

#[cfg(not(feature = "development-font-fallback"))]
fn development_resource_path() -> Option<PathBuf> {
    None
}

fn resolve_resource_path(
    executable: &Path,
    development_resource: Option<&Path>,
) -> Result<PathBuf, FontBootstrapFailure> {
    let installed_resource = installed_resource_path(executable)?;
    if installed_resource.is_file() {
        return Ok(installed_resource);
    }

    development_resource
        .filter(|resource| resource.is_file())
        .map(Path::to_owned)
        .ok_or(FontBootstrapFailure::ResourceUnavailable)
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
    use std::{io, path::Path};

    use eframe::egui::FontFamily;

    use super::{
        BUNDLED_FONT_KEY, FontBootstrapFailure, FontBootstrapper, StartupRoute,
        installed_resource_path, resolve_resource_path,
    };

    // These are the Required_Text categories rendered by the workspace: UI
    // labels, a Chinese filename, availability text, safe Latin fallback text,
    // and representative navigation/layout symbols.
    const REQUIRED_TEXT_SAMPLES: &[&str] = &[
        "图像集合打开文件夹当前集合没有可显示的受支持图像可用性说明预览正在准备预览从集合中选择一张图像以开始编辑",
        "几何变换水平翻转垂直翻转顺时针旋转逆时针旋转开始裁剪确认取消调整亮度对比度增加减少提交撤销重做导出无格式请求正在处理",
        "示例图片.png",
        "当前平台没有可用的文件夹选择器",
        "A required application font is unavailable. The editor has not started.",
        "→…+-()[]{}:,.!?/\\|_#@&*%=<>←↑↓↔↕─│┌┐└┘",
    ];

    fn packaged_font_path() -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join(crate::BUNDLED_FONT_SOURCE_PATH)
    }

    fn packaged_font() -> FontBootstrapper {
        FontBootstrapper::from_resource_path(packaged_font_path())
    }

    fn packaged_font_bytes() -> Vec<u8> {
        std::fs::read(packaged_font_path()).expect("checked-in bundled font must be readable")
    }

    fn assert_startup_availability_error(
        result: Result<(), FontBootstrapFailure>,
        expected_failure: FontBootstrapFailure,
    ) {
        assert_eq!(result, Err(expected_failure));
        let route = StartupRoute::from_bootstrap(result);
        assert_eq!(
            route,
            StartupRoute::StartupAvailabilityError(expected_failure)
        );
        assert_ne!(route, StartupRoute::InteractiveEditor);
    }

    #[test]
    fn packaged_font_resolver_uses_the_platform_package_resource_location() {
        #[cfg(target_os = "macos")]
        let executable = Path::new("/Applications/Image Editor.app/Contents/MacOS/image-editor");
        #[cfg(target_os = "macos")]
        let expected = Path::new(
            "/Applications/Image Editor.app/Contents/Resources/resources/fonts/NotoSansCJKsc-Regular.otf",
        );

        #[cfg(not(target_os = "macos"))]
        let executable = Path::new("/opt/image-editor/bin/image-editor");
        #[cfg(not(target_os = "macos"))]
        let expected = Path::new("/opt/image-editor/bin/resources/fonts/NotoSansCJKsc-Regular.otf");

        assert_eq!(
            installed_resource_path(executable).expect("absolute binary path has a parent"),
            expected
        );
    }

    #[cfg(feature = "development-font-fallback")]
    #[test]
    fn release_development_resolver_uses_checked_in_font_without_package_staging() {
        let development_resource = packaged_font_path();
        let unstaged_release_executable =
            Path::new("/tmp/image-editor/target/release/image-editor");

        assert_eq!(
            resolve_resource_path(
                unstaged_release_executable,
                Some(development_resource.as_path())
            )
            .expect("release Cargo builds may use the checked-in font"),
            development_resource
        );
        assert_eq!(
            resolve_resource_path(unstaged_release_executable, None),
            Err(FontBootstrapFailure::ResourceUnavailable),
            "installable package resolution must never fall back to a source tree"
        );
    }

    #[test]
    fn packaged_font_covers_required_chinese_latin_and_symbol_text() {
        let bytes = packaged_font_bytes();
        let face = ttf_parser::Face::parse(&bytes, 0).expect("checked-in bundled font is valid");
        let missing = REQUIRED_TEXT_SAMPLES
            .iter()
            .flat_map(|sample| sample.chars())
            .filter(|character| face.glyph_index(*character).is_none())
            .map(|character| format!("U+{:04X}", character as u32))
            .collect::<Vec<_>>();

        assert!(
            missing.is_empty(),
            "the bundled font is missing required glyphs: {missing:?}"
        );
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
            assert!(
                choices[1..].iter().all(|choice| choice != BUNDLED_FONT_KEY),
                "the bundled font must have one first-priority entry per family"
            );
        }
    }

    #[test]
    fn injected_read_parse_and_registration_failures_never_start_an_interactive_editor() {
        let bootstrapper = packaged_font();

        assert_startup_availability_error(
            bootstrapper.install_with(
                |_| Err(io::Error::new(io::ErrorKind::NotFound, "font missing")),
                |_| Ok(()),
                |_| Ok(()),
            ),
            FontBootstrapFailure::ResourceUnavailable,
        );
        assert_startup_availability_error(
            bootstrapper.install_with(|_| Ok(packaged_font_bytes()), |_| Err(()), |_| Ok(())),
            FontBootstrapFailure::MalformedData,
        );
        assert_startup_availability_error(
            bootstrapper.install_with(
                |_| Ok(packaged_font_bytes()),
                |bytes| {
                    ttf_parser::Face::parse(bytes, 0)
                        .map(|_| ())
                        .map_err(|_| ())
                },
                |_| Err(FontBootstrapFailure::RegistrationRejected),
            ),
            FontBootstrapFailure::RegistrationRejected,
        );
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
