use std::collections::BTreeMap;

use image_editor_core::{
    AbsolutePath, Availability, CanonicalImage, CapabilitySnapshot, CodecProvider, DirectoryEntry,
    DirectoryEntryKind, DirectoryEntryLocation, EditOperation, EditorCommand, EditorState,
    FormatCapability, ImageFormat, PlatformCapability, Rgba16, Utf8FileName,
    plan_folder_enumeration, reduce, render_current_editing_result,
};
use proptest::prelude::*;

#[derive(Clone, Debug, Default)]
struct ReferenceHistory {
    history: Vec<EditOperation>,
    redo: Vec<EditOperation>,
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

fn source_image(seed: u16) -> CanonicalImage {
    CanonicalImage::new(
        2,
        3,
        (0..6)
            .map(|offset| {
                let value = seed.wrapping_add(offset);
                Rgba16::new(
                    value,
                    value.wrapping_mul(3),
                    value.wrapping_mul(7),
                    u16::MAX,
                )
            })
            .collect(),
    )
    .expect("fixed source dimensions match the generated pixel count")
}

fn activate_document(
    state: &EditorState,
    candidate: image_editor_core::CollectionEntry,
    source: CanonicalImage,
) -> EditorState {
    let decoding = reduce(state, EditorCommand::BeginDecode { candidate });
    reduce(
        &decoding.state,
        EditorCommand::ImageDecoded {
            token: decoding.effects[0].token(),
            image: source,
        },
    )
    .state
}

fn two_document_state() -> (
    EditorState,
    Vec<image_editor_core::ImageId>,
    Vec<CanonicalImage>,
) {
    let folder = AbsolutePath::new("/photos").expect("test folder is absolute");
    let entries = ["first.png", "second.png"]
        .into_iter()
        .map(|name| {
            DirectoryEntry::new(
                AbsolutePath::new(format!("/photos/{name}")).expect("test path is absolute"),
                Utf8FileName::new(name).expect("test filename is valid"),
                DirectoryEntryLocation::Direct,
                DirectoryEntryKind::RegularFile,
                None,
            )
        })
        .collect();
    let initial = EditorState::new(capabilities());
    let enumerating = reduce(
        &initial,
        EditorCommand::BeginFolderEnumeration {
            folder: folder.clone(),
        },
    );
    let plan = plan_folder_enumeration(
        &capabilities(),
        image_editor_core::FolderEnumerationInput::Succeeded { folder, entries },
    );
    let collected = reduce(
        &enumerating.state,
        EditorCommand::FolderEnumerated {
            token: enumerating.effects[0].token(),
            result: plan,
        },
    );
    let candidates = collected.state.browsing().collection().entries().to_vec();
    let sources = vec![source_image(10), source_image(100)];
    let first_active =
        activate_document(&collected.state, candidates[0].clone(), sources[0].clone());
    let state = activate_document(&first_active, candidates[1].clone(), sources[1].clone());
    let ids = candidates
        .into_iter()
        .map(|candidate| candidate.id)
        .collect();
    (state, ids, sources)
}

fn operation(action: u8) -> EditOperation {
    match action {
        2 => EditOperation::FlipHorizontal,
        3 => EditOperation::FlipVertical,
        4 => EditOperation::RotateClockwise90,
        5 => EditOperation::RotateCounterclockwise90,
        _ => unreachable!("only edit actions use this helper"),
    }
}

fn assert_matches_reference(
    state: &EditorState,
    ids: &[image_editor_core::ImageId],
    sources: &[CanonicalImage],
    reference: &[ReferenceHistory],
) -> Result<(), TestCaseError> {
    for ((id, source), expected) in ids.iter().zip(sources).zip(reference) {
        let document = state
            .browsing()
            .document(id)
            .expect("both decoded documents remain retained");
        prop_assert_eq!(document.history(), expected.history.as_slice());
        prop_assert_eq!(document.redo(), expected.redo.as_slice());
        let actual_result =
            render_current_editing_result(document.source(), document.history(), document.draft())
                .expect("reducer-produced history is renderable");
        let expected_result =
            render_current_editing_result(source, &expected.history, document.draft())
                .expect("reference history contains only valid operations");
        prop_assert_eq!(actual_result, expected_result);
    }
    Ok(())
}

fn select_document(state: &EditorState, index: usize, sources: &[CanonicalImage]) -> EditorState {
    let candidate = state.browsing().collection().entries()[index].clone();
    activate_document(state, candidate, sources[index].clone())
}

// Feature: macos-image-editor, Property 7: Per-image history is reversible and branch-safe
// Validates: Requirements 6.1, 6.2, 6.3, 6.4, 6.5, 6.6
proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    #[test]
    fn multi_document_history_matches_a_reversible_branch_safe_reference_model(
        actions in prop::collection::vec(0_u8..8, 0..200),
    ) {
        let (mut state, ids, sources) = two_document_state();
        let mut reference = vec![ReferenceHistory::default(), ReferenceHistory::default()];
        let mut active = 1_usize;

        // Both stacks start empty. Their commands must leave the complete state unchanged.
        for command in [EditorCommand::Undo, EditorCommand::Redo] {
            let before = state.clone();
            let reduction = reduce(&state, command);
            prop_assert_eq!(&reduction.state, &before);
            prop_assert!(reduction.effects.is_empty());
            state = reduction.state;
        }

        for action in actions {
            match action {
                0 | 1 => {
                    active = action as usize;
                    state = select_document(&state, active, &sources);
                }
                2..=5 => {
                    let edit = operation(action);
                    state = reduce(&state, match &edit {
                        EditOperation::FlipHorizontal => EditorCommand::FlipHorizontal,
                        EditOperation::FlipVertical => EditorCommand::FlipVertical,
                        EditOperation::RotateClockwise90 => EditorCommand::RotateClockwise90,
                        EditOperation::RotateCounterclockwise90 => EditorCommand::RotateCounterclockwise90,
                        _ => unreachable!("the generator only creates geometric edits"),
                    }).state;
                    reference[active].history.push(edit);
                    reference[active].redo.clear();
                }
                6 => {
                    let before_state = state.clone();
                    let before_document = before_state
                        .browsing()
                        .document(&ids[active])
                        .expect("active document remains retained")
                        .clone();
                    let undone = reduce(&state, EditorCommand::Undo);
                    if let Some(operation) = reference[active].history.pop() {
                        reference[active].redo.push(operation);
                        // Undo followed immediately by redo must reconstruct the exact stack and image.
                        let redone = reduce(&undone.state, EditorCommand::Redo);
                        let restored = redone
                            .state
                            .browsing()
                            .document(&ids[active])
                            .expect("active document remains retained");
                        prop_assert_eq!(restored.history(), before_document.history());
                        prop_assert_eq!(restored.redo(), before_document.redo());
                        let before_result = render_current_editing_result(
                            before_document.source(),
                            before_document.history(),
                            before_document.draft(),
                        ).expect("pre-undo reducer history is renderable");
                        let restored_result = render_current_editing_result(
                            restored.source(), restored.history(), restored.draft(),
                        ).expect("redone reducer history is renderable");
                        prop_assert_eq!(restored_result, before_result);
                    } else {
                        prop_assert_eq!(&undone.state, &before_state);
                        prop_assert!(undone.effects.is_empty());
                    }
                    state = undone.state;
                }
                7 => {
                    let before_state = state.clone();
                    let redone = reduce(&state, EditorCommand::Redo);
                    if let Some(operation) = reference[active].redo.pop() {
                        reference[active].history.push(operation);
                    } else {
                        prop_assert_eq!(&redone.state, &before_state);
                        prop_assert!(redone.effects.is_empty());
                    }
                    state = redone.state;
                }
                _ => unreachable!("generator range is limited to eight actions"),
            }
            assert_matches_reference(&state, &ids, &sources, &reference)?;
        }

        // Force an undo-then-new-edit branch and prove it clears only its active document's redo.
        active = 0;
        state = select_document(&state, active, &sources);
        for edit in [EditOperation::FlipHorizontal, EditOperation::RotateClockwise90] {
            let command = match edit {
                EditOperation::FlipHorizontal => EditorCommand::FlipHorizontal,
                EditOperation::RotateClockwise90 => EditorCommand::RotateClockwise90,
                _ => unreachable!("fixed branch setup uses geometric edits"),
            };
            state = reduce(&state, command).state;
            reference[active].history.push(edit);
            reference[active].redo.clear();
        }
        state = reduce(&state, EditorCommand::Undo).state;
        let undone = reference[active].history.pop().expect("branch setup created history");
        reference[active].redo.push(undone);
        let other_before_branch = reference[1].clone();
        state = reduce(&state, EditorCommand::FlipVertical).state;
        reference[active].history.push(EditOperation::FlipVertical);
        reference[active].redo.clear();

        prop_assert!(state.browsing().document(&ids[active]).unwrap().redo().is_empty());
        prop_assert_eq!(
            state.browsing().document(&ids[1]).unwrap().history(),
            other_before_branch.history.as_slice(),
        );
        prop_assert_eq!(
            state.browsing().document(&ids[1]).unwrap().redo(),
            other_before_branch.redo.as_slice(),
        );
        assert_matches_reference(&state, &ids, &sources, &reference)?;
    }
}
