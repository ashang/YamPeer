//! Codec capability detection and adapter registration.
//!
//! `portable-codecs` enables the Rust JPEG/PNG/TIFF dependency. `heic` only
//! links the optional adapter; a runtime probe must separately confirm decoder
//! and encoder availability before either HEIC capability is exposed.

use std::collections::BTreeMap;

use image_editor_core::{
    Availability, AvailabilityReason, CapabilitySnapshot, CodecProvider, FormatCapability,
    ImageFormat, PlatformCapability,
};

pub use image_editor_core::{ApplicationError, ErrorCategory, Result, SafeError};

type ProbeResult<T> = std::result::Result<T, CodecProbeError>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CompiledCodecFeatures {
    pub portable_codecs: bool,
    pub heic_adapter: bool,
}

/// Compile-time linkage facts only; these are not runtime capability claims.
pub const COMPILED_FEATURES: CompiledCodecFeatures = CompiledCodecFeatures {
    portable_codecs: cfg!(feature = "portable-codecs"),
    heic_adapter: cfg!(feature = "heic"),
};

/// Platform capabilities obtained before open-folder or export commands are enabled.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StartupPlatformCapabilities {
    pub folder_picker: PlatformCapability,
    pub save_picker: PlatformCapability,
}

impl StartupPlatformCapabilities {
    pub fn new(folder_picker: PlatformCapability, save_picker: PlatformCapability) -> Self {
        Self {
            folder_picker,
            save_picker,
        }
    }
}

/// Error from a capability self-check or runtime codec probe.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CodecProbeError {
    message: String,
}

impl CodecProbeError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

impl std::fmt::Display for CodecProbeError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for CodecProbeError {}

/// In-process JPEG/PNG/TIFF self-check boundary.
///
/// Callers supply fixed fixture bytes to `check_decode`; `check_encode` writes
/// a bounded one-pixel image to an in-memory sink. Keeping the operations
/// separate prevents one direction from being inferred from the other.
pub trait PortableCodecSelfCheck: Send + Sync {
    fn check_decode(&self, format: ImageFormat, fixture: &[u8]) -> ProbeResult<()>;
    fn check_encode(&self, format: ImageFormat) -> ProbeResult<()>;
}

/// Runtime probe boundary for libheif and its independently installed plugins.
pub trait HeicRuntimeProbe: Send + Sync {
    fn check_decode(&self) -> ProbeResult<()>;
    fn check_encode(&self) -> ProbeResult<()>;
}

/// A capability registry whose immutable snapshot is constructed before callers
/// accept open-folder or export commands. Actual decode/encode dispatch is added
/// by the guarded codec adapter task; this registry deliberately only makes
/// availability claims supported by the startup probes.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CodecRegistry {
    snapshot: CapabilitySnapshot,
}

impl CodecRegistry {
    /// Runs the compiled portable self-check and records HEIC as unavailable
    /// until an application-specific libheif runtime probe is supplied.
    pub fn detect(platform: StartupPlatformCapabilities) -> Self {
        #[cfg(feature = "portable-codecs")]
        let portable = ImageRsPortableSelfCheck;
        #[cfg(feature = "portable-codecs")]
        let portable = Some(&portable as &dyn PortableCodecSelfCheck);
        #[cfg(not(feature = "portable-codecs"))]
        let portable: Option<&dyn PortableCodecSelfCheck> = None;

        Self::detect_with(platform, portable, &UnconfiguredHeicProbe)
    }

    /// Builds a complete immutable snapshot from real or test probe adapters.
    /// HEIC decode and encode are intentionally queried independently.
    pub fn detect_with(
        platform: StartupPlatformCapabilities,
        portable: Option<&dyn PortableCodecSelfCheck>,
        heic: &dyn HeicRuntimeProbe,
    ) -> Self {
        let mut formats = BTreeMap::new();
        for format in [ImageFormat::Jpeg, ImageFormat::Png, ImageFormat::Tiff] {
            formats.insert(format, portable_capability(format, portable));
        }
        formats.insert(ImageFormat::Heic, heic_capability(heic));

        Self {
            snapshot: CapabilitySnapshot::new(
                formats,
                platform.folder_picker,
                platform.save_picker,
            ),
        }
    }

    pub fn snapshot(&self) -> &CapabilitySnapshot {
        &self.snapshot
    }

    pub fn into_snapshot(self) -> CapabilitySnapshot {
        self.snapshot
    }
}

fn portable_capability(
    format: ImageFormat,
    portable: Option<&dyn PortableCodecSelfCheck>,
) -> FormatCapability {
    let Some(portable) = portable else {
        let reason = AvailabilityReason::new(format!(
            "{} decoding and encoding are unavailable because portable codecs were not compiled",
            format.display_name()
        ));
        return FormatCapability::new(
            Availability::Unavailable {
                reason: reason.clone(),
            },
            Availability::Unavailable { reason },
            None,
        );
    };

    let fixture = portable_fixture(format);
    let decode = match fixture {
        Some(fixture) => availability_from(portable.check_decode(format, &fixture)),
        None => unavailable(format!(
            "{} decoding self-check fixture is not available",
            format.display_name()
        )),
    };
    let encode = availability_from(portable.check_encode(format));
    FormatCapability::new(decode, encode, Some(CodecProvider::PortableRust))
}

fn heic_capability(heic: &dyn HeicRuntimeProbe) -> FormatCapability {
    FormatCapability::new(
        availability_from(heic.check_decode()),
        availability_from(heic.check_encode()),
        Some(CodecProvider::Libheif),
    )
}

fn availability_from(result: ProbeResult<()>) -> Availability {
    match result {
        Ok(()) => Availability::Available,
        Err(error) => unavailable(error.message),
    }
}

fn unavailable(reason: impl Into<String>) -> Availability {
    Availability::Unavailable {
        reason: AvailabilityReason::new(reason),
    }
}

/// The default state is deliberately conservative. Linking libheif is not proof
/// that the process can initialize it or that its decoder/encoder plugins exist.
struct UnconfiguredHeicProbe;

impl HeicRuntimeProbe for UnconfiguredHeicProbe {
    fn check_decode(&self) -> ProbeResult<()> {
        Err(CodecProbeError::new(
            "HEIC decoding is unavailable because no libheif runtime probe was configured",
        ))
    }

    fn check_encode(&self) -> ProbeResult<()> {
        Err(CodecProbeError::new(
            "HEIC encoding is unavailable because no libheif runtime probe was configured",
        ))
    }
}

#[cfg(feature = "portable-codecs")]
struct ImageRsPortableSelfCheck;

#[cfg(feature = "portable-codecs")]
impl PortableCodecSelfCheck for ImageRsPortableSelfCheck {
    fn check_decode(&self, format: ImageFormat, fixture: &[u8]) -> ProbeResult<()> {
        image::load_from_memory_with_format(fixture, image_format(format))
            .map(|_| ())
            .map_err(|error| {
                CodecProbeError::new(format!(
                    "{} decoding self-check failed: {error}",
                    format.display_name()
                ))
            })
    }

    fn check_encode(&self, format: ImageFormat) -> ProbeResult<()> {
        use std::io::Cursor;

        let image = image::DynamicImage::new_rgb8(1, 1);
        image
            .write_to(&mut Cursor::new(Vec::new()), image_format(format))
            .map_err(|error| {
                CodecProbeError::new(format!(
                    "{} encoding self-check failed: {error}",
                    format.display_name()
                ))
            })
    }
}

/// Generates bounded, deterministic bytes through the same encoder registered
/// for the format. They are passed to the independent decode self-check.
#[cfg(feature = "portable-codecs")]
fn portable_fixture(format: ImageFormat) -> Option<Vec<u8>> {
    use std::io::Cursor;

    let image = image::DynamicImage::new_rgb8(1, 1);
    let mut sink = Cursor::new(Vec::new());
    image.write_to(&mut sink, image_format(format)).ok()?;
    Some(sink.into_inner())
}

#[cfg(not(feature = "portable-codecs"))]
fn portable_fixture(_: ImageFormat) -> Option<Vec<u8>> {
    None
}

#[cfg(feature = "portable-codecs")]
fn image_format(format: ImageFormat) -> image::ImageFormat {
    match format {
        ImageFormat::Jpeg => image::ImageFormat::Jpeg,
        ImageFormat::Png => image::ImageFormat::Png,
        ImageFormat::Tiff => image::ImageFormat::Tiff,
        ImageFormat::Heic => unreachable!("HEIC is never sent to image-rs portable codecs"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image_editor_core::{CapabilityName, NoticeSubject};

    fn available_platform() -> StartupPlatformCapabilities {
        StartupPlatformCapabilities::new(
            PlatformCapability::available("native-folder-dialog"),
            PlatformCapability::available("native-save-dialog"),
        )
    }

    struct PassingPortable;

    impl PortableCodecSelfCheck for PassingPortable {
        fn check_decode(&self, _: ImageFormat, _: &[u8]) -> ProbeResult<()> {
            Ok(())
        }

        fn check_encode(&self, _: ImageFormat) -> ProbeResult<()> {
            Ok(())
        }
    }

    struct IndependentHeic;

    impl HeicRuntimeProbe for IndependentHeic {
        fn check_decode(&self) -> ProbeResult<()> {
            Ok(())
        }

        fn check_encode(&self) -> ProbeResult<()> {
            Err(CodecProbeError::new(
                "libheif HEIC encoder plugin is missing",
            ))
        }
    }

    #[test]
    fn registry_records_heic_decode_and_encode_independently() {
        let snapshot = CodecRegistry::detect_with(
            available_platform(),
            Some(&PassingPortable),
            &IndependentHeic,
        )
        .into_snapshot();

        assert!(snapshot.format(ImageFormat::Heic).can_decode());
        assert!(!snapshot.format(ImageFormat::Heic).can_encode());
        assert_eq!(
            snapshot.format(ImageFormat::Heic).provider,
            Some(CodecProvider::Libheif)
        );
        assert!(snapshot.diagnostics().iter().any(|notice| {
            notice.subject
                == NoticeSubject::Capability(CapabilityName::FormatEncode(ImageFormat::Heic))
                && notice
                    .message
                    .summary()
                    .contains("encoder plugin is missing")
        }));
    }

    #[test]
    fn registry_preserves_portable_formats_when_heic_is_unavailable() {
        let snapshot = CodecRegistry::detect_with(
            available_platform(),
            Some(&PassingPortable),
            &UnconfiguredHeicProbe,
        )
        .into_snapshot();

        for format in [ImageFormat::Jpeg, ImageFormat::Png, ImageFormat::Tiff] {
            assert!(snapshot.format(format).can_decode());
            assert!(snapshot.format(format).can_encode());
            assert_eq!(
                snapshot.format(format).provider,
                Some(CodecProvider::PortableRust)
            );
        }
        assert!(!snapshot.format(ImageFormat::Heic).can_decode());
        assert!(!snapshot.format(ImageFormat::Heic).can_encode());
    }

    #[cfg(feature = "portable-codecs")]
    #[test]
    fn image_rs_portable_self_check_registers_all_portable_codecs() {
        let snapshot = CodecRegistry::detect(available_platform()).into_snapshot();

        for format in [ImageFormat::Jpeg, ImageFormat::Png, ImageFormat::Tiff] {
            assert!(
                snapshot.format(format).can_decode(),
                "{format:?} decode unavailable"
            );
            assert!(
                snapshot.format(format).can_encode(),
                "{format:?} encode unavailable"
            );
        }
    }
}
