use std::collections::BTreeMap;

use image_editor_core::{
    AbsolutePath, Availability, CanonicalImage, CapabilitySnapshot, CodecProvider, CropDraft,
    CropRect, DirectoryEntry, DirectoryEntryKind, DirectoryEntryLocation, EditOperation,
    EditorCommand, EditorState, Effect, FolderEnumerationInput, FolderEnumerationPlan,
    FormatCapability, ImageFormat, InteractionMode, PlatformCapability, Rgba16, Utf8FileName,
    plan_folder_enumeration, reduce, render_current_editing_result,
};
use proptest::prelude::*;

#[derive(Clone, Copy, Debug)]
struct CropCase {
    width: u32,
    height: u32,
    untrusted_draft: CropDraft,
    is_valid_after_clamping: bool,
}

fn capabilities() -> CapabilitySnapshot {
    let available = || {
        FormatCapability::new(
            Availability::Available,
            Availability::Available,
            Some(CodecProvider::PortableRust),
        )
    };
    let mut formats = BTreeMap::new();
    for format in [
        ImageFormat::Jpeg,
        ImageFormat::Png,
        ImageFormat::Tiff,
        ImageFormat::Heic,
    ] {
        formats.insert(format, available());
    }
    CapabilitySnapshot::new(
        formats,
        PlatformCapability::available("test-folder-picker"),
        PlatformCapability::available("test-save-picker"),
    )
}

fn activate(source: CanonicalImage) -> (EditorState, image_editor_core::ImageId) {
    let source_path = AbsolutePath::new("/photos/crop-source.png").unwrap();
    let entry = DirectoryEntry::new(
        source_path.clone(),
        Utf8FileName::new("crop-source.png").unwrap(),
        DirectoryEntryLocation::Direct,
        DirectoryEntryKind::RegularFile,
        None,
    );
    let initial = EditorState::new(capabilities());
    let enumerating = reduce(
        &initial,
        EditorCommand::BeginFolderEnumeration {
            folder: AbsolutePath::new("/photos").unwrap(),
        },
    );
    let plan = plan_folder_enumeration(
        &capabilities(),
        FolderEnumerationInput::Succeeded {
            folder: AbsolutePath::new("/photos").unwrap(),
            entries: vec![entry],
        },
    );
    let FolderEnumerationPlan::Ready(_) = plan else {
        panic!("the test source must produce a collection plan");
    };
    let collected = reduce(
        &enumerating.state,
        EditorCommand::FolderEnumerated {
            token: enumerating.effects[0].token(),
            result: plan,
        },
    );
    let candidate = collected.state.browsing().collection().entries()[0].clone();
    let image_id = candidate.id.clone();
    let decoding = reduce(&collected.state, EditorCommand::BeginDecode { candidate });
    let active = reduce(
        &decoding.state,
        EditorCommand::ImageDecoded {
            token: decoding.effects[0].token(),
            image: source,
        },
    );
    (active.state, image_id)
}

fn source_image(width: u32, height: u32) -> CanonicalImage {
    let pixels = (0..height)
        .flat_map(|y| {
            (0..width).map(move |x| {
                let sample = (y * width + x + 1) as u16;
                Rgba16::new(
                    sample,
                    sample.wrapping_mul(3),
                    sample.wrapping_mul(7),
                    u16::MAX,
                )
            })
        })
        .collect();
    CanonicalImage::new(width, height, pixels).unwrap()
}

fn crop_case() -> impl Strategy<Value = CropCase> {
    (1_u32..=8, 1_u32..=8).prop_flat_map(|(width, height)| {
        let valid = (0_u32..width, 0_u32..height).prop_flat_map(move |(left, top)| {
            ((left + 1)..=width, (top + 1)..=height)
                .prop_map(move |(right, bottom)| CropDraft::new(left, top, right, bottom))
        });
        let invalid = prop_oneof![
            (0_u32..=width, 0_u32..=height)
                .prop_map(|(left, top)| CropDraft::new(left, top, left, top)),
            (1_u32..=width, 0_u32..height).prop_flat_map(move |(left, top)| {
                ((top + 1)..=height)
                    .prop_map(move |bottom| CropDraft::new(left, top, left - 1, bottom))
            }),
            (0_u32..=width, 0_u32..=height, 0_u32..=width, 0_u32..=height).prop_map(
                |(right, top, _left, bottom)| CropDraft::new(u32::MAX, top, right, bottom)
            ),
        ];
        prop_oneof![
            valid.prop_map(move |untrusted_draft| CropCase {
                width,
                height,
                untrusted_draft,
                is_valid_after_clamping: true,
            }),
            invalid.prop_map(move |untrusted_draft| CropCase {
                width,
                height,
                untrusted_draft,
                is_valid_after_clamping: false,
            }),
        ]
    })
}

fn expected_pixels(source: &CanonicalImage, crop: CropRect) -> Vec<Rgba16> {
    (crop.top()..crop.bottom())
        .flat_map(|y| {
            (crop.left()..crop.right())
                .map(move |x| source.pixels()[(y as usize * source.width() as usize) + x as usize])
        })
        .collect()
}

// Feature: macos-image-editor, Property 5: Crop is bounded, exact, and transactional
// Validates: Requirements 4.3, 4.4, 4.5, 4.6, 4.7
proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    #[test]
    fn crop_transaction_clamps_untrusted_bounds_and_preserves_or_commits_atomically(case in crop_case()) {
        let source = source_image(case.width, case.height);
        let (active, image_id) = activate(source.clone());
        let entered = reduce(&active, EditorCommand::EnterCrop);
        let selected = reduce(
            &entered.state,
            EditorCommand::UpdateCropDraft {
                draft: case.untrusted_draft,
            },
        );
        let clamped = case.untrusted_draft.clamped(case.width, case.height);

        prop_assert_eq!(selected.state.mode(), InteractionMode::Crop(clamped));

        let before_cancel_browsing = selected.state.browsing().clone();
        let cancelled = reduce(&selected.state, EditorCommand::CancelCrop);
        prop_assert_eq!(cancelled.state.browsing(), &before_cancel_browsing);
        prop_assert_eq!(cancelled.state.mode(), InteractionMode::Browse);
        prop_assert!(cancelled.effects.is_empty());

        let before_confirmation_browsing = selected.state.browsing().clone();
        let before_confirmation_mode = selected.state.mode();
        let confirmed = reduce(&selected.state, EditorCommand::ConfirmCrop);

        if case.is_valid_after_clamping {
            let crop = CropRect::new(
                case.width,
                case.height,
                clamped.left,
                clamped.top,
                clamped.right,
                clamped.bottom,
            )
            .expect("the valid generator must remain valid after clamping");
            let document = confirmed
                .state
                .browsing()
                .document(&image_id)
                .expect("the active document must be retained");
            prop_assert_eq!(document.history(), &[EditOperation::Crop(crop)]);
            prop_assert!(document.redo().is_empty());
            prop_assert_eq!(confirmed.state.mode(), InteractionMode::Browse);
            let Effect::RenderPreview { request, .. } = &confirmed.effects[0] else {
                prop_assert!(false, "a valid crop must request exactly one preview render");
                unreachable!();
            };
            prop_assert_eq!(confirmed.effects.len(), 1);
            let rendered = render_current_editing_result(
                &request.source,
                &request.history,
                &request.draft,
            )
            .expect("reducer-produced crop history must render");
            prop_assert_eq!((rendered.width(), rendered.height()), (crop.width(), crop.height()));
            prop_assert_eq!(rendered.pixels(), expected_pixels(&source, crop));
        } else {
            prop_assert_eq!(confirmed.state.browsing(), &before_confirmation_browsing);
            prop_assert_eq!(confirmed.state.mode(), before_confirmation_mode);
            prop_assert!(confirmed.effects.is_empty());
            prop_assert!(confirmed.state.notices().last().is_some());
        }
    }
}
