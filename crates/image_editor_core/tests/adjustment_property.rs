use std::collections::BTreeMap;

use image_editor_core::{
    AbsolutePath, AdjustmentKind, Availability, CanonicalImage, CapabilitySnapshot, CodecProvider,
    DirectoryEntry, DirectoryEntryKind, DirectoryEntryLocation, EditorCommand, EditorState,
    FormatCapability, ImageFormat, PlatformCapability, Rgba16, Utf8FileName,
    plan_folder_enumeration, reduce, render_current_editing_result,
};
use proptest::prelude::*;

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
    let folder = AbsolutePath::new("/photos").expect("test folder is absolute");
    let entry = DirectoryEntry::new(
        AbsolutePath::new("/photos/photo.png").expect("test image path is absolute"),
        Utf8FileName::new("photo.png").expect("test filename is valid"),
        DirectoryEntryLocation::Direct,
        DirectoryEntryKind::RegularFile,
        None,
    );
    let initial = EditorState::new(capabilities());
    let enumerating = reduce(
        &initial,
        EditorCommand::BeginFolderEnumeration {
            folder: folder.clone(),
        },
    );
    let collection = plan_folder_enumeration(
        &capabilities(),
        image_editor_core::FolderEnumerationInput::Succeeded {
            folder,
            entries: vec![entry],
        },
    );
    let installed = reduce(
        &enumerating.state,
        EditorCommand::FolderEnumerated {
            token: enumerating.effects[0].token(),
            result: collection,
        },
    );
    let candidate = installed.state.browsing().collection().entries()[0].clone();
    let image_id = candidate.id.clone();
    let decoding = reduce(&installed.state, EditorCommand::BeginDecode { candidate });
    let active = reduce(
        &decoding.state,
        EditorCommand::ImageDecoded {
            token: decoding.effects[0].token(),
            image: source,
        },
    );
    (active.state, image_id)
}

fn apply(state: &mut EditorState, command: EditorCommand) {
    *state = reduce(state, command).state;
}

fn source(samples: [u16; 16]) -> CanonicalImage {
    CanonicalImage::new(
        2,
        2,
        samples
            .chunks_exact(4)
            .map(|channels| Rgba16::new(channels[0], channels[1], channels[2], channels[3]))
            .collect(),
    )
    .expect("generated sample count matches dimensions")
}

fn expected_operation(kind: AdjustmentKind, value: i16) -> image_editor_core::EditOperation {
    match kind {
        AdjustmentKind::Brightness => image_editor_core::EditOperation::brightness(value),
        AdjustmentKind::Contrast => image_editor_core::EditOperation::contrast(value),
    }
    .expect("reference values are clamped to the adjustment range")
}

fn value_for(kind: AdjustmentKind, brightness: i16, contrast: i16) -> i16 {
    match kind {
        AdjustmentKind::Brightness => brightness,
        AdjustmentKind::Contrast => contrast,
    }
}

// Feature: macos-image-editor, Property 6: Adjustment commands are clamped and commit exactly their draft
// Validates: Requirements 5.1-5.11
proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    #[test]
    fn adjustment_sequences_clamp_and_commit_without_changing_rendered_pixels(
        samples in prop::array::uniform16(any::<u16>()),
        commands in prop::collection::vec(0_u8..4, 0..300),
        commit_brightness in any::<bool>(),
    ) {
        let image = source(samples);
        let (mut state, image_id) = activate(image.clone());
        let mut brightness = 0_i16;
        let mut contrast = 0_i16;
        let mut focused = None;

        for command in commands {
            match command {
                0 => {
                    apply(&mut state, EditorCommand::FocusAdjustment(AdjustmentKind::Brightness));
                    focused = Some(AdjustmentKind::Brightness);
                }
                1 => {
                    apply(&mut state, EditorCommand::FocusAdjustment(AdjustmentKind::Contrast));
                    focused = Some(AdjustmentKind::Contrast);
                }
                2 => {
                    apply(&mut state, EditorCommand::IncreaseAdjustment);
                    match focused {
                        Some(AdjustmentKind::Brightness) => brightness = (brightness + 1).min(100),
                        Some(AdjustmentKind::Contrast) => contrast = (contrast + 1).min(100),
                        None => {}
                    }
                }
                3 => {
                    apply(&mut state, EditorCommand::DecreaseAdjustment);
                    match focused {
                        Some(AdjustmentKind::Brightness) => brightness = (brightness - 1).max(-100),
                        Some(AdjustmentKind::Contrast) => contrast = (contrast - 1).max(-100),
                        None => {}
                    }
                }
                _ => unreachable!("generator range is limited to four commands"),
            }

            let draft = state
                .browsing()
                .document(&image_id)
                .expect("active image retains its document")
                .draft();
            prop_assert_eq!(draft.brightness().get(), brightness);
            prop_assert_eq!(draft.contrast().get(), contrast);
            prop_assert_eq!(draft.focused(), focused);
            prop_assert!((-100..=100).contains(&brightness));
            prop_assert!((-100..=100).contains(&contrast));
        }

        let committed_kind = if commit_brightness {
            AdjustmentKind::Brightness
        } else {
            AdjustmentKind::Contrast
        };
        apply(&mut state, EditorCommand::FocusAdjustment(committed_kind));
        focused = Some(committed_kind);

        let before_commit = {
            let document = state
                .browsing()
                .document(&image_id)
                .expect("active image retains its document");
            render_current_editing_result(document.source(), document.history(), document.draft())
                .expect("reducer-produced document is renderable")
        };
        let history_len = state
            .browsing()
            .document(&image_id)
            .expect("active image retains its document")
            .history()
            .len();
        let committed_value = value_for(committed_kind, brightness, contrast);
        let retained_kind = match committed_kind {
            AdjustmentKind::Brightness => AdjustmentKind::Contrast,
            AdjustmentKind::Contrast => AdjustmentKind::Brightness,
        };
        let retained_value = value_for(retained_kind, brightness, contrast);

        apply(&mut state, EditorCommand::CommitAdjustment);
        let document = state
            .browsing()
            .document(&image_id)
            .expect("active image retains its document");
        prop_assert_eq!(document.history().len(), history_len + 1);
        prop_assert_eq!(document.history().last(), Some(&expected_operation(committed_kind, committed_value)));
        prop_assert_eq!(document.draft().focused(), None);
        prop_assert_eq!(value_for(committed_kind, document.draft().brightness().get(), document.draft().contrast().get()), 0);
        prop_assert_eq!(value_for(retained_kind, document.draft().brightness().get(), document.draft().contrast().get()), retained_value);
        let after_commit = render_current_editing_result(document.source(), document.history(), document.draft())
            .expect("reducer-produced document is renderable");
        prop_assert_eq!(after_commit, before_commit);

        // A zero-valued focused draft must commit as an identity operation whose
        // rendered result retains every source channel, including alpha.
        let (mut zero_state, zero_image_id) = activate(image.clone());
        apply(
            &mut zero_state,
            EditorCommand::FocusAdjustment(committed_kind),
        );
        apply(&mut zero_state, EditorCommand::CommitAdjustment);
        let zero_document = zero_state
            .browsing()
            .document(&zero_image_id)
            .expect("active image retains its document");
        prop_assert_eq!(zero_document.history(), &[expected_operation(committed_kind, 0)]);
        prop_assert_eq!(zero_document.draft().focused(), None);
        let zero_result = render_current_editing_result(
            zero_document.source(),
            zero_document.history(),
            zero_document.draft(),
        )
        .expect("zero adjustment history is renderable");
        prop_assert_eq!(zero_result, image);
    }
}
