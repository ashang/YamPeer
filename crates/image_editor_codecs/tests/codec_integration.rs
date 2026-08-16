#![cfg(feature = "portable-codecs")]

//! Real-file integration coverage for the portable codec registry.

use std::{
    io::Cursor,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

use image_editor_codecs::{
    CodecError, CodecRegistry, DecodeLimits, StartupPlatformCapabilities, execute_export_request,
};
use image_editor_core::{
    AbsolutePath, ApplicationError, CanonicalImage, DraftAdjustments, EditOperation, ExportPlan,
    ExportRequest, ExportTargetResolution, ImageFormat, PlatformCapability, ResourceLimitKind,
    Revision, Rgba16, SourceIdentity, TargetConflict,
};

static NEXT_FIXTURE_ID: AtomicU64 = AtomicU64::new(0);

struct FixtureFile {
    path: AbsolutePath,
}

impl Drop for FixtureFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(self.path.as_str());
    }
}

struct TemporaryDirectory {
    path: PathBuf,
}

impl TemporaryDirectory {
    fn new(label: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "image-editor-{label}-{}-{}",
            std::process::id(),
            NEXT_FIXTURE_ID.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).expect("create temporary directory");
        Self { path }
    }

    fn path(&self) -> &std::path::Path {
        &self.path
    }
}

impl Drop for TemporaryDirectory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

fn available_platform() -> StartupPlatformCapabilities {
    StartupPlatformCapabilities::new(
        PlatformCapability::available("test-folder-picker"),
        PlatformCapability::available("test-save-picker"),
    )
}

fn registry() -> CodecRegistry {
    CodecRegistry::detect(available_platform())
}

fn fixture_image() -> CanonicalImage {
    CanonicalImage::new(
        3,
        2,
        vec![
            Rgba16::new(2_570, 2_570, 2_570, u16::MAX),
            Rgba16::new(12_850, 12_850, 12_850, u16::MAX),
            Rgba16::new(25_700, 25_700, 25_700, u16::MAX),
            Rgba16::new(38_550, 38_550, 38_550, u16::MAX),
            Rgba16::new(51_400, 51_400, 51_400, u16::MAX),
            Rgba16::new(64_250, 64_250, 64_250, u16::MAX),
        ],
    )
    .expect("fixed fixture pixels are valid")
}

fn image_format(format: ImageFormat) -> image::ImageFormat {
    match format {
        ImageFormat::Jpeg => image::ImageFormat::Jpeg,
        ImageFormat::Png => image::ImageFormat::Png,
        ImageFormat::Tiff => image::ImageFormat::Tiff,
        ImageFormat::Heic => unreachable!("HEIC fixture construction is runtime-gated"),
    }
}

fn extension(format: ImageFormat) -> &'static str {
    match format {
        ImageFormat::Jpeg => "jpg",
        ImageFormat::Png => "png",
        ImageFormat::Tiff => "tiff",
        ImageFormat::Heic => "heic",
    }
}

fn temporary_file(extension: &str, bytes: &[u8]) -> FixtureFile {
    let id = NEXT_FIXTURE_ID.fetch_add(1, Ordering::Relaxed);
    let path: PathBuf = std::env::temp_dir().join(format!(
        "image-editor-codec-integration-{}-{id}.{extension}",
        std::process::id()
    ));
    std::fs::write(&path, bytes).expect("write fixture file");
    FixtureFile {
        path: AbsolutePath::new(path.to_string_lossy().into_owned())
            .expect("temporary path is absolute UTF-8"),
    }
}

/// Creates encoded image bytes independently of the registry under test, then
/// materializes them as real files so decode dispatch sees a normal source path.
fn real_fixture(format: ImageFormat) -> FixtureFile {
    let source = fixture_image();
    let mut encoded = Cursor::new(Vec::new());
    match format {
        ImageFormat::Jpeg => {
            let samples = source
                .pixels()
                .iter()
                .flat_map(|pixel| {
                    [
                        (pixel.red >> 8) as u8,
                        (pixel.green >> 8) as u8,
                        (pixel.blue >> 8) as u8,
                    ]
                })
                .collect();
            let pixels = image::RgbImage::from_raw(source.width(), source.height(), samples)
                .expect("fixed fixture dimensions match RGB samples");
            image::DynamicImage::ImageRgb8(pixels)
                .write_to(&mut encoded, image_format(format))
                .expect("encode JPEG fixture with image-rs");
        }
        ImageFormat::Png | ImageFormat::Tiff => {
            let samples = source
                .pixels()
                .iter()
                .flat_map(|pixel| [pixel.red, pixel.green, pixel.blue, pixel.alpha])
                .collect();
            let pixels = image::ImageBuffer::<image::Rgba<u16>, Vec<u16>>::from_raw(
                source.width(),
                source.height(),
                samples,
            )
            .expect("fixed fixture dimensions match samples");
            image::DynamicImage::ImageRgba16(pixels)
                .write_to(&mut encoded, image_format(format))
                .expect("encode fixed fixture with image-rs");
        }
        ImageFormat::Heic => unreachable!("HEIC fixture construction is runtime-gated"),
    }
    temporary_file(extension(format), &encoded.into_inner())
}

fn assert_jpeg_tolerance(actual: &CanonicalImage, source: &CanonicalImage) {
    assert_eq!(
        (actual.width(), actual.height()),
        (source.width(), source.height())
    );
    for (decoded, expected) in actual.pixels().iter().zip(source.pixels()) {
        assert_eq!(
            decoded.alpha,
            u16::MAX,
            "JPEG decode must restore opaque alpha"
        );
        for (actual_sample, expected_sample) in [decoded.red, decoded.green, decoded.blue]
            .into_iter()
            .zip([expected.red, expected.green, expected.blue])
        {
            assert!(
                actual_sample.abs_diff(expected_sample) <= 4_096,
                "JPEG channel difference ({}) exceeds the accepted lossy tolerance from {expected_sample} to {actual_sample}",
                actual_sample.abs_diff(expected_sample),
            );
        }
    }
}

#[test]
fn decodes_real_jpeg_png_and_tiff_fixtures() {
    let registry = registry();
    let expected = fixture_image();

    for format in [ImageFormat::Png, ImageFormat::Tiff] {
        let fixture = real_fixture(format);
        assert_eq!(
            registry
                .decode(format, &fixture.path, DecodeLimits::DEFAULT)
                .unwrap(),
            expected,
            "{format:?} fixture must decode with identical RGBA16 samples"
        );
    }

    let jpeg = real_fixture(ImageFormat::Jpeg);
    let decoded = registry
        .decode(ImageFormat::Jpeg, &jpeg.path, DecodeLimits::DEFAULT)
        .expect("decode JPEG fixture");
    assert_jpeg_tolerance(&decoded, &expected);
}

#[test]
fn encodes_and_reopens_portable_formats_with_format_appropriate_equivalence() {
    let registry = registry();
    let source = fixture_image();

    for format in [ImageFormat::Png, ImageFormat::Tiff, ImageFormat::Jpeg] {
        let mut encoded = Vec::new();
        registry
            .encode(&source, format, &mut encoded)
            .expect("encode portable fixture");
        let output = temporary_file(extension(format), &encoded);
        let reopened = registry
            .decode(format, &output.path, DecodeLimits::DEFAULT)
            .expect("reopen encoded output");

        if matches!(format, ImageFormat::Png | ImageFormat::Tiff) {
            assert_eq!(
                reopened, source,
                "{format:?} output must preserve all RGBA16 samples"
            );
        } else {
            assert_jpeg_tolerance(&reopened, &source);
        }
    }
}

#[test]
fn malformed_supported_files_remain_content_errors_for_the_requested_format() {
    let registry = registry();

    for format in [ImageFormat::Jpeg, ImageFormat::Png, ImageFormat::Tiff] {
        let malformed = temporary_file(extension(format), b"not an image fixture");
        assert!(
            matches!(
                registry.decode(format, &malformed.path, DecodeLimits::DEFAULT),
                Err(CodecError::Content { format: error_format, .. }) if error_format == format
            ),
            "malformed {format:?} content must be reported as a content error, not an unavailable capability"
        );
    }
}

#[test]
fn real_fixture_decode_enforces_input_and_pixel_resource_limits() {
    let registry = registry();
    let fixture = real_fixture(ImageFormat::Png);

    assert!(matches!(
        registry.decode(
            ImageFormat::Png,
            &fixture.path,
            DecodeLimits {
                max_input_bytes: 1,
                ..DecodeLimits::DEFAULT
            }
        ),
        Err(CodecError::ResourceLimit(ResourceLimitKind::InputBytes))
    ));
    assert!(matches!(
        registry.decode(
            ImageFormat::Png,
            &fixture.path,
            DecodeLimits {
                max_total_pixels: 5,
                ..DecodeLimits::DEFAULT
            }
        ),
        Err(CodecError::ResourceLimit(ResourceLimitKind::TotalPixels))
    ));
}

#[test]
fn export_execution_renders_the_snapshot_then_publishes_one_new_lossless_file() {
    let registry = registry();
    let directory = std::env::temp_dir().join(format!(
        "image-editor-export-execution-{}-{}",
        std::process::id(),
        NEXT_FIXTURE_ID.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir(&directory).expect("create export directory");
    let target_path = directory.join("result.png");
    let target = AbsolutePath::new(target_path.to_string_lossy().into_owned()).unwrap();
    let source_path =
        AbsolutePath::new(directory.join("source.png").to_string_lossy().into_owned()).unwrap();
    let request = ExportRequest {
        image_id: SourceIdentity::new(source_path, None),
        revision: image_editor_core::Revision::INITIAL,
        source: fixture_image(),
        history: vec![EditOperation::FlipHorizontal],
        draft: DraftAdjustments::new(),
        target: target.clone(),
        format: ImageFormat::Png,
    };

    execute_export_request(&registry, &request).expect("export immutable snapshot");
    let reopened = registry
        .decode(ImageFormat::Png, &target, DecodeLimits::DEFAULT)
        .expect("reopen published export");
    assert_eq!(
        reopened
            .pixels()
            .iter()
            .map(|pixel| pixel.red)
            .collect::<Vec<_>>(),
        vec![25_700, 12_850, 2_570, 64_250, 51_400, 38_550],
        "the exported file must contain the full-resolution rendered edit result"
    );
    assert!(target_path.exists());
    assert_eq!(
        std::fs::read_dir(&directory)
            .unwrap()
            .filter_map(Result::ok)
            .count(),
        1,
        "successful publication must not leave a sibling temporary file"
    );
    std::fs::remove_file(target_path).unwrap();
    std::fs::remove_dir(directory).unwrap();
}

#[test]
fn export_execution_rejects_a_target_that_appears_after_planning_without_replacement() {
    let registry = registry();
    let directory = std::env::temp_dir().join(format!(
        "image-editor-export-race-{}-{}",
        std::process::id(),
        NEXT_FIXTURE_ID.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir(&directory).expect("create export directory");
    let target_path = directory.join("existing.png");
    let existing_bytes = b"existing target bytes";
    std::fs::write(&target_path, existing_bytes).unwrap();
    let target = AbsolutePath::new(target_path.to_string_lossy().into_owned()).unwrap();
    let request = ExportRequest {
        image_id: SourceIdentity::new(
            AbsolutePath::new(directory.join("source.png").to_string_lossy().into_owned()).unwrap(),
            None,
        ),
        revision: image_editor_core::Revision::INITIAL,
        source: fixture_image(),
        history: Vec::new(),
        draft: DraftAdjustments::new(),
        target: target.clone(),
        format: ImageFormat::Png,
    };

    let error = execute_export_request(&registry, &request)
        .expect_err("exclusive publication must reject a target created after planning");
    assert!(matches!(
        error,
        image_editor_core::ApplicationError::ExportWrite { path, .. } if path == target
    ));
    assert_eq!(std::fs::read(&target_path).unwrap(), existing_bytes);
    assert_eq!(
        std::fs::read_dir(&directory)
            .unwrap()
            .filter_map(Result::ok)
            .count(),
        1,
        "failed publication must remove only its own temporary file"
    );
    std::fs::remove_file(target_path).unwrap();
    std::fs::remove_dir(directory).unwrap();
}

#[test]
fn detected_heic_adapter_round_trips_a_real_file_with_lossy_tolerance() {
    let registry = registry();
    let capability = registry.snapshot().format(ImageFormat::Heic);
    if !(capability.can_decode() && capability.can_encode()) {
        return;
    }

    let source = fixture_image();
    let mut encoded = Vec::new();
    registry
        .encode(&source, ImageFormat::Heic, &mut encoded)
        .expect("the detected HEIC encoder must encode the real fixture");
    assert!(!encoded.is_empty(), "HEIC encoder must produce file bytes");

    let fixture = temporary_file(extension(ImageFormat::Heic), &encoded);
    let decoded = registry
        .decode(ImageFormat::Heic, &fixture.path, DecodeLimits::DEFAULT)
        .expect("the detected HEIC decoder must reopen the encoded fixture");
    assert_eq!(
        (decoded.width(), decoded.height()),
        (source.width(), source.height()),
        "HEIC must preserve the orientation-normalized dimensions"
    );
    for (actual, expected) in decoded.pixels().iter().zip(source.pixels()) {
        for (actual_sample, expected_sample) in [actual.red, actual.green, actual.blue]
            .into_iter()
            .zip([expected.red, expected.green, expected.blue])
        {
            assert!(
                actual_sample.abs_diff(expected_sample) <= 8_192,
                "HEIC channel difference ({}) exceeds the accepted lossy tolerance from {expected_sample} to {actual_sample}",
                actual_sample.abs_diff(expected_sample),
            );
        }
    }
}

#[test]
fn detected_heic_decoder_reports_damaged_content_without_disabling_capability() {
    let registry = registry();
    if !registry.snapshot().format(ImageFormat::Heic).can_decode() {
        return;
    }

    let malformed = temporary_file(extension(ImageFormat::Heic), b"not a HEIC fixture");
    assert!(
        matches!(
            registry.decode(ImageFormat::Heic, &malformed.path, DecodeLimits::DEFAULT),
            Err(CodecError::Content {
                format: ImageFormat::Heic,
                ..
            })
        ),
        "damaged HEIC content must remain a content error when the decoder is available"
    );
}

#[test]
fn export_conflicts_and_write_failures_preserve_source_and_existing_target_bytes() {
    let registry = registry();
    let directory = TemporaryDirectory::new("export-preservation");
    let source_path = directory.path().join("source.png");
    let source = AbsolutePath::new(source_path.to_string_lossy().into_owned()).unwrap();
    let source_identity = SourceIdentity::new(source.clone(), None);
    let mut source_bytes = Vec::new();
    registry
        .encode(&fixture_image(), ImageFormat::Png, &mut source_bytes)
        .expect("encode source fixture");
    std::fs::write(&source_path, &source_bytes).expect("write source fixture");

    let source_conflict = ExportPlan::validate(
        source_identity.clone(),
        Revision::INITIAL,
        source.clone(),
        ImageFormat::Png,
        ExportTargetResolution::existing_regular(None),
    )
    .expect_err("an export must never replace its source file");
    assert!(matches!(
        source_conflict,
        ApplicationError::ExportTargetConflict {
            kind: TargetConflict::SourceImage,
            ..
        }
    ));
    assert_eq!(
        std::fs::read(&source_path).unwrap(),
        source_bytes,
        "source bytes must survive a source-path conflict"
    );

    let existing_target_path = directory.path().join("existing.png");
    let existing_bytes = b"existing target bytes";
    std::fs::write(&existing_target_path, existing_bytes).expect("write existing target");
    let existing_target =
        AbsolutePath::new(existing_target_path.to_string_lossy().into_owned()).unwrap();
    let existing_conflict = ExportPlan::validate(
        source_identity.clone(),
        Revision::INITIAL,
        existing_target,
        ImageFormat::Png,
        ExportTargetResolution::existing_regular(None),
    )
    .expect_err("an export must never replace an existing target");
    assert!(matches!(
        existing_conflict,
        ApplicationError::ExportTargetConflict {
            kind: TargetConflict::ExistingLocalFile,
            ..
        }
    ));
    assert_eq!(
        std::fs::read(&source_path).unwrap(),
        source_bytes,
        "source bytes must survive an existing-target conflict"
    );
    assert_eq!(
        std::fs::read(&existing_target_path).unwrap(),
        existing_bytes,
        "existing target bytes must survive a rejected export"
    );

    let missing_parent = directory.path().join("missing-parent");
    let failed_target_path = missing_parent.join("result.png");
    let failed_target =
        AbsolutePath::new(failed_target_path.to_string_lossy().into_owned()).unwrap();
    let request = ExportRequest {
        image_id: source_identity,
        revision: Revision::INITIAL,
        source: fixture_image(),
        history: vec![EditOperation::FlipHorizontal],
        draft: DraftAdjustments::new(),
        target: failed_target.clone(),
        format: ImageFormat::Png,
    };
    let error = execute_export_request(&registry, &request)
        .expect_err("a target in a missing parent directory must fail safely");
    assert!(matches!(
        error,
        ApplicationError::ExportWrite { path, .. } if path == failed_target
    ));
    assert_eq!(
        std::fs::read(&source_path).unwrap(),
        source_bytes,
        "source bytes must survive an export write failure"
    );
    assert_eq!(
        std::fs::read(&existing_target_path).unwrap(),
        existing_bytes,
        "an unrelated existing target must survive an export write failure"
    );
    assert!(
        !missing_parent.exists(),
        "a failed export must not create its missing parent directory"
    );
}

#[test]
fn portable_export_execution_reopens_with_format_appropriate_equivalence() {
    let registry = registry();
    let directory = TemporaryDirectory::new("portable-export-equivalence");
    let source_image = fixture_image();
    let source_path = directory.path().join("source.png");
    let source = AbsolutePath::new(source_path.to_string_lossy().into_owned()).unwrap();
    let source_identity = SourceIdentity::new(source.clone(), None);
    let mut source_bytes = Vec::new();
    registry
        .encode(&source_image, ImageFormat::Png, &mut source_bytes)
        .expect("encode source fixture");
    std::fs::write(&source_path, &source_bytes).expect("write source fixture");
    let history = vec![EditOperation::FlipHorizontal];
    let expected = image_editor_core::render_current_editing_result(
        &source_image,
        &history,
        &DraftAdjustments::new(),
    )
    .expect("render fixed export snapshot");

    for format in [ImageFormat::Png, ImageFormat::Tiff, ImageFormat::Jpeg] {
        let target_path = directory
            .path()
            .join(format!("edited.{}", extension(format)));
        let target = AbsolutePath::new(target_path.to_string_lossy().into_owned()).unwrap();
        let request = ExportRequest {
            image_id: source_identity.clone(),
            revision: Revision::INITIAL,
            source: source_image.clone(),
            history: history.clone(),
            draft: DraftAdjustments::new(),
            target: target.clone(),
            format,
        };

        execute_export_request(&registry, &request)
            .unwrap_or_else(|error| panic!("{format:?} export must succeed: {error:?}"));
        assert_eq!(
            std::fs::read(&source_path).unwrap(),
            source_bytes,
            "{format:?} export must preserve source bytes"
        );

        let reopened = registry
            .decode(format, &target, DecodeLimits::DEFAULT)
            .unwrap_or_else(|error| panic!("{format:?} export must reopen: {error:?}"));
        match format {
            ImageFormat::Png | ImageFormat::Tiff => assert_eq!(
                reopened, expected,
                "{format:?} must preserve dimensions and every RGBA16 sample"
            ),
            ImageFormat::Jpeg => assert_jpeg_tolerance(&reopened, &expected),
            ImageFormat::Heic => unreachable!("the portable-format test excludes HEIC"),
        }
    }
}
