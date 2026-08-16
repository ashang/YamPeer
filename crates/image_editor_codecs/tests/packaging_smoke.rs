#![cfg(feature = "portable-codecs")]

//! Package-startup smoke coverage for optional runtime capability providers.

use image_editor_codecs::{
    CodecProbeError, CodecRegistry, HeicRuntimeProbe, PortableCodecSelfCheck,
    StartupPlatformCapabilities,
};
use image_editor_core::{
    CapabilityName, DependentOperation, ImageFormat, NoticeSubject, PlatformCapability,
    project_capabilities,
};

struct PortableCodecsPresent;

impl PortableCodecSelfCheck for PortableCodecsPresent {
    fn check_decode(&self, _: ImageFormat, _: &[u8]) -> std::result::Result<(), CodecProbeError> {
        Ok(())
    }

    fn check_encode(&self, _: ImageFormat) -> std::result::Result<(), CodecProbeError> {
        Ok(())
    }
}

struct HeicRuntimePresent;

impl HeicRuntimeProbe for HeicRuntimePresent {
    fn check_decode(&self) -> std::result::Result<(), CodecProbeError> {
        Ok(())
    }

    fn check_encode(&self) -> std::result::Result<(), CodecProbeError> {
        Ok(())
    }
}

struct HeicRuntimeMissing;

impl HeicRuntimeProbe for HeicRuntimeMissing {
    fn check_decode(&self) -> std::result::Result<(), CodecProbeError> {
        Err(CodecProbeError::new(
            "libheif runtime is intentionally absent",
        ))
    }

    fn check_encode(&self) -> std::result::Result<(), CodecProbeError> {
        Err(CodecProbeError::new(
            "libheif runtime is intentionally absent",
        ))
    }
}

fn available_dialogs() -> StartupPlatformCapabilities {
    StartupPlatformCapabilities::new(
        PlatformCapability::available("packaging-smoke-folder-picker"),
        PlatformCapability::available("packaging-smoke-save-picker"),
    )
}

#[test]
fn package_startup_keeps_one_capability_aware_workspace_for_optional_dependencies_present_or_missing()
 {
    for (scenario, heic_probe, heic_available) in [
        (
            "optional HEIC runtime present",
            &HeicRuntimePresent as &dyn HeicRuntimeProbe,
            true,
        ),
        (
            "optional HEIC runtime missing",
            &HeicRuntimeMissing as &dyn HeicRuntimeProbe,
            false,
        ),
    ] {
        let registry = CodecRegistry::detect_with(
            available_dialogs(),
            Some(&PortableCodecsPresent),
            heic_probe,
        );
        let snapshot = registry.snapshot();
        let projection = project_capabilities(snapshot, None);

        assert!(
            projection.is_operation_enabled(DependentOperation::OpenFolder),
            "{scenario} must still start with folder browsing available"
        );
        assert!(
            projection.is_operation_enabled(DependentOperation::Export),
            "{scenario} must still start with portable export available"
        );
        for format in [ImageFormat::Jpeg, ImageFormat::Png, ImageFormat::Tiff] {
            assert!(
                projection.export_formats().contains(&format),
                "{scenario} must retain portable {format:?} export"
            );
        }

        assert_eq!(
            snapshot.format(ImageFormat::Heic).can_decode(),
            heic_available,
            "{scenario} must expose the detected HEIC decoder state"
        );
        assert_eq!(
            snapshot.format(ImageFormat::Heic).can_encode(),
            heic_available,
            "{scenario} must expose the detected HEIC encoder state"
        );
        assert_eq!(
            projection.export_formats().contains(&ImageFormat::Heic),
            heic_available,
            "{scenario} must project HEIC export only when the runtime probe succeeds"
        );

        let has_heic_notice = projection.availability_messages().iter().any(|notice| {
            notice.subject
                == NoticeSubject::Capability(CapabilityName::FormatDecode(ImageFormat::Heic))
        });
        assert_eq!(
            has_heic_notice, !heic_available,
            "{scenario} must surface only the corresponding HEIC availability message"
        );
    }
}
