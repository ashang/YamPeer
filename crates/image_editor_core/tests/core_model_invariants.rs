use image_editor_core::{
    AbsolutePath, AdjustmentKind, AdjustmentValidationError, AdjustmentValue, ApplicationError,
    CanonicalImage, CapabilityName, CollectionEntry, CollectionEntryError, CommandName, CropRect,
    CropValidationError, ErrorCategory, FileNameValidationError, ImageFormat, ImageValidationError,
    NoticeSeverity, NoticeSubject, SafeError, SourceIdentity, Utf8FileName,
};

#[test]
fn canonical_image_rejects_dimensions_that_overflow_rgba16_allocation() {
    assert_eq!(
        CanonicalImage::checked_pixel_count(u32::MAX, u32::MAX),
        Err(ImageValidationError::BufferSizeOverflow {
            width: u32::MAX,
            height: u32::MAX,
        })
    );
}

#[test]
fn model_constructors_reject_malformed_and_unsupported_values() {
    assert!(AbsolutePath::new("relative/image.png").is_err());
    assert_eq!(
        Utf8FileName::new("nested/image.png"),
        Err(FileNameValidationError::ContainsSeparatorOrNul)
    );
    assert_eq!(ImageFormat::from_extension("webp"), None);

    let path = AbsolutePath::new("/images/photo.png").unwrap();
    let identity = SourceIdentity::new(path.clone(), None);
    assert_eq!(
        CollectionEntry::new(
            identity,
            path,
            Utf8FileName::new("photo.png").unwrap(),
            ImageFormat::Jpeg,
        ),
        Err(CollectionEntryError::FormatMismatch)
    );
}

#[test]
fn crop_rect_accepts_half_open_image_edges_and_rejects_invalid_bounds() {
    let crop = CropRect::new(8, 6, 0, 0, 8, 6).unwrap();
    assert_eq!(
        (crop.left(), crop.top(), crop.right(), crop.bottom()),
        (0, 0, 8, 6)
    );
    assert_eq!(
        CropRect::new(8, 6, 4, 1, 4, 5),
        Err(CropValidationError::EmptyOrReversedHorizontal { left: 4, right: 4 })
    );
    assert_eq!(
        CropRect::new(8, 6, 1, 5, 4, 5),
        Err(CropValidationError::EmptyOrReversedVertical { top: 5, bottom: 5 })
    );
    assert!(matches!(
        CropRect::new(8, 6, 0, 0, 9, 6),
        Err(CropValidationError::OutOfBounds { .. })
    ));
    assert!(matches!(
        CropRect::new(8, 6, 0, 0, 8, 7),
        Err(CropValidationError::OutOfBounds { .. })
    ));
}

#[test]
fn adjustment_values_reject_out_of_range_inputs_and_clamp_at_endpoints() {
    for value in [-101, 101] {
        assert_eq!(
            AdjustmentValue::new(value),
            Err(AdjustmentValidationError::OutOfRange { value })
        );
    }

    let mut draft = image_editor_core::DraftAdjustments::new();
    draft.focus(AdjustmentKind::Brightness);
    draft.set(AdjustmentKind::Brightness, 100).unwrap();
    draft.increase_focused();
    assert_eq!(draft.brightness().get(), 100);

    draft.focus(AdjustmentKind::Contrast);
    draft.set(AdjustmentKind::Contrast, -100).unwrap();
    draft.decrease_focused();
    assert_eq!(draft.contrast().get(), -100);
}

#[test]
fn visible_errors_are_sanitized_and_keep_a_structured_subject() {
    let error = ApplicationError::PlatformOperation {
        capability: CapabilityName::FormatDecode(ImageFormat::Heic),
        cause: SafeError::new(ErrorCategory::OptionalDependency, "missing\ncodec\u{1b}[2J"),
    };

    let notice = error.to_notice();
    assert_eq!(notice.severity, NoticeSeverity::Error);
    assert_eq!(
        notice.subject,
        NoticeSubject::Capability(CapabilityName::FormatDecode(ImageFormat::Heic))
    );
    assert_eq!(notice.message.category(), ErrorCategory::OptionalDependency);
    assert_eq!(notice.message.summary(), "missing codec [2J");
    assert!(!notice.message.summary().chars().any(char::is_control));

    let no_active = ApplicationError::MissingActiveImage {
        command: CommandName::EnterCrop,
    }
    .to_notice();
    assert_eq!(no_active.severity, NoticeSeverity::Error);
    assert_eq!(
        no_active.subject,
        NoticeSubject::Command(CommandName::Export)
    );
    assert_eq!(
        no_active.message.summary(),
        "cannot crop without an active image"
    );
}
