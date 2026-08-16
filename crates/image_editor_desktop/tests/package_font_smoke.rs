#![cfg(feature = "native-window")]

//! Target-package smoke coverage for the mandatory bundled CJK font.

use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use eframe::egui;
use image_editor_desktop::{
    BUNDLED_FONT_LICENSE, BUNDLED_FONT_NAME, BUNDLED_FONT_SHA256, BUNDLED_FONT_VERSION,
    PackageProfile,
    font_bootstrap::{FontBootstrapFailure, FontBootstrapper, StartupRoute},
};

static NEXT_FIXTURE_ID: AtomicU64 = AtomicU64::new(0);

struct PackageFixture {
    root: PathBuf,
    profile: PackageProfile,
}

impl PackageFixture {
    fn staged(profile: PackageProfile) -> Self {
        let root = std::env::temp_dir().join(format!(
            "image-editor-package-font-smoke-{}-{}",
            std::process::id(),
            NEXT_FIXTURE_ID.fetch_add(1, Ordering::Relaxed),
        ));
        fs::create_dir(&root).expect("create package fixture directory");
        profile
            .write_package_metadata(&root.join("capabilities.json"))
            .expect("stage target package font and metadata");
        let executable = match profile {
            PackageProfile::MacosAarch64 => root
                .join("Image Editor.app")
                .join("Contents")
                .join("MacOS")
                .join("image-editor"),
            PackageProfile::LinuxX86_64Portal => root.join("image-editor"),
        };
        fs::create_dir_all(
            executable
                .parent()
                .expect("packaged executable location has a parent"),
        )
        .expect("create packaged executable directory");
        fs::write(&executable, b"package smoke executable")
            .expect("create package smoke executable");
        Self { root, profile }
    }

    fn executable_path(&self) -> PathBuf {
        match self.profile {
            PackageProfile::MacosAarch64 => self
                .root
                .join("Image Editor.app")
                .join("Contents")
                .join("MacOS")
                .join("image-editor"),
            PackageProfile::LinuxX86_64Portal => self.root.join("image-editor"),
        }
    }

    fn font_path(&self) -> PathBuf {
        self.root.join(self.profile.bundled_font_resource_path())
    }

    fn metadata_paths(&self) -> [PathBuf; 4] {
        [
            self.root.join("capabilities.json"),
            self.root.join("package-metadata.json"),
            self.root.join("release-licenses.json"),
            self.root.join("sbom.spdx.json"),
        ]
    }
}

impl Drop for PackageFixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn assert_declared_font_metadata(fixture: &PackageFixture) {
    let font = fixture.font_path();
    let bytes = fs::read(&font).expect("declared package font resource is readable");
    assert!(
        !bytes.is_empty(),
        "declared package font resource must not be empty: {}",
        font.display()
    );

    for metadata_path in fixture.metadata_paths() {
        let metadata = fs::read_to_string(&metadata_path)
            .unwrap_or_else(|error| panic!("read {}: {error}", metadata_path.display()));
        for expected in [
            fixture.profile.bundled_font_resource_path(),
            BUNDLED_FONT_NAME,
            BUNDLED_FONT_VERSION,
            BUNDLED_FONT_SHA256,
            BUNDLED_FONT_LICENSE,
        ] {
            assert!(
                metadata.contains(expected),
                "{} must declare bundled-font metadata {expected:?}",
                metadata_path.display()
            );
        }
    }
}

fn startup_route_for_packaged_executable(executable: &Path) -> StartupRoute {
    let context = egui::Context::default();
    StartupRoute::from_bootstrap(
        FontBootstrapper::for_packaged_executable(executable)
            .and_then(|bootstrapper| bootstrapper.install(&context)),
    )
}

fn current_platform_profile() -> PackageProfile {
    #[cfg(target_os = "macos")]
    {
        PackageProfile::MacosAarch64
    }

    #[cfg(not(target_os = "macos"))]
    {
        PackageProfile::LinuxX86_64Portal
    }
}

#[test]
fn current_target_package_declares_and_registers_its_font_before_the_one_window_startup_path() {
    let profile = current_platform_profile();
    let fixture = PackageFixture::staged(profile);
    assert_declared_font_metadata(&fixture);

    // InteractiveEditor is the only branch through the native creation
    // callback that constructs the normal DesktopApp and its sole window.
    assert_eq!(
        startup_route_for_packaged_executable(&fixture.executable_path()),
        StartupRoute::InteractiveEditor,
        "{} package must register its staged font before the normal one-window startup path",
        profile.platform()
    );
}

#[test]
fn unavailable_current_target_package_font_enters_safe_startup_availability_error_without_workspace()
 {
    let profile = current_platform_profile();
    let fixture = PackageFixture::staged(profile);
    let font = fixture.font_path();
    fs::remove_file(&font).expect("remove staged font from unavailable-resource fixture");

    let route = startup_route_for_packaged_executable(&fixture.executable_path());
    assert_eq!(
        route,
        StartupRoute::StartupAvailabilityError(FontBootstrapFailure::ResourceUnavailable),
        "{} package without its mandatory font must not construct a normal workspace",
        profile.platform()
    );
    assert_ne!(route, StartupRoute::InteractiveEditor);
    assert!(
        FontBootstrapFailure::ResourceUnavailable
            .safe_message()
            .is_ascii(),
        "safe startup error must not depend on the unavailable CJK font"
    );
    assert!(
        !FontBootstrapFailure::ResourceUnavailable
            .safe_message()
            .contains('□'),
        "safe startup error must not render a missing-glyph box"
    );
}
