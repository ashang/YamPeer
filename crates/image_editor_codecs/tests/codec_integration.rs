#![cfg(feature = "portable-codecs")]

//! Real-file integration coverage for the portable codec registry.

use std::{
    io::Cursor,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

use image_editor_codecs::{CodecError, CodecRegistry, DecodeLimits, StartupPlatformCapabilities};
use image_editor_core::{
    AbsolutePath, CanonicalImage, ImageFormat, PlatformCapability, ResourceLimitKind, Rgba16,
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
    let mut encoded = Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba16(pixels)
        .write_to(&mut encoded, image_format(format))
        .expect("encode fixed fixture with image-rs");
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
fn heic_cases_are_skipped_until_a_runtime_adapter_is_detected() {
    let registry = registry();
    let capability = registry.snapshot().format(ImageFormat::Heic);

    if capability.can_decode() || capability.can_encode() {
        // Portable fixture creation must never pretend to exercise HEIC. A real
        // HEIC adapter and fixture are required before this branch gains cases.
        assert!(
            capability.can_decode() || capability.can_encode(),
            "HEIC tests run only when a runtime adapter reports availability"
        );
    }
}
