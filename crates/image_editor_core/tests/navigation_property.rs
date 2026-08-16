use std::collections::BTreeMap;

use image_editor_core::{
    AbsolutePath, Availability, CanonicalImage, CapabilitySnapshot, CodecProvider, DirectoryEntry,
    DirectoryEntryKind, DirectoryEntryLocation, EditorCommand, EditorState, Effect,
    FolderEnumerationInput, FormatCapability, ImageFormat, NavigationDirection, NavigationTarget,
    PlatformCapability, Rgba16, Utf8FileName, plan_folder_enumeration, plan_navigation, reduce,
};
use proptest::prelude::*;

fn path(value: &str) -> AbsolutePath {
    AbsolutePath::new(value).expect("test paths are absolute UTF-8")
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

fn entry(index: usize) -> DirectoryEntry {
    let file_name = format!("image-{index:03}.png");
    DirectoryEntry::new(
        path(&format!("/photos/{file_name}")),
        Utf8FileName::new(file_name).expect("generated filename is valid UTF-8"),
        DirectoryEntryLocation::Direct,
        DirectoryEntryKind::RegularFile,
        None,
    )
}

fn image() -> CanonicalImage {
    CanonicalImage::new(1, 1, vec![Rgba16::new(1, 2, 3, u16::MAX)]).expect("test image is valid")
}

fn install_collection(count: usize) -> EditorState {
    let initial = EditorState::new(capabilities());
    let request = reduce(
        &initial,
        EditorCommand::BeginFolderEnumeration {
            folder: path("/photos"),
        },
    );
    let plan = plan_folder_enumeration(
        &capabilities(),
        FolderEnumerationInput::Succeeded {
            folder: path("/photos"),
            // Reverse the injected listing so the test relies only on the
            // collection's defined filename ordering.
            entries: (0..count).rev().map(entry).collect(),
        },
    );
    let completion = reduce(
        &request.state,
        EditorCommand::FolderEnumerated {
            token: request.effects[0].token(),
            result: plan,
        },
    );
    completion.state
}

fn activate(state: &EditorState, index: usize) -> EditorState {
    let candidate = state.browsing().collection().entries()[index].clone();
    let request = reduce(
        state,
        EditorCommand::SelectImage {
            image_id: candidate.id,
        },
    );
    reduce(
        &request.state,
        EditorCommand::ImageDecoded {
            token: request.effects[0].token(),
            image: image(),
        },
    )
    .state
}

fn expected_target(
    entry_ids: &[image_editor_core::ImageId],
    active_index: usize,
    direction: NavigationDirection,
) -> NavigationTarget {
    let target_index = match direction {
        NavigationDirection::Left => active_index.checked_sub(1),
        NavigationDirection::Right => {
            (active_index + 1 < entry_ids.len()).then_some(active_index + 1)
        }
        NavigationDirection::Home => (active_index != 0).then_some(0),
        NavigationDirection::End => {
            (active_index + 1 != entry_ids.len()).then_some(entry_ids.len() - 1)
        }
    };
    target_index
        .map(|index| NavigationTarget::Candidate(entry_ids[index].clone()))
        .unwrap_or(NavigationTarget::NoTarget)
}

// Feature: macos-image-editor, Property 3: Navigation targets obey collection order and boundaries
// Validates: Requirements 2.10, 2.11, 2.12, 2.13
proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    #[test]
    fn navigation_targets_follow_order_and_retain_browsing_state(
        collection_size in 1usize..32,
        arbitrary_active_index in any::<usize>(),
    ) {
        let installed = install_collection(collection_size);
        let entry_ids = installed
            .browsing()
            .collection()
            .entries()
            .iter()
            .map(|entry| entry.id.clone())
            .collect::<Vec<_>>();
        let active_index = arbitrary_active_index % entry_ids.len();
        let active_state = activate(&installed, active_index);

        for direction in [
            NavigationDirection::Left,
            NavigationDirection::Right,
            NavigationDirection::Home,
            NavigationDirection::End,
        ] {
            let expected = expected_target(&entry_ids, active_index, direction);
            prop_assert_eq!(
                plan_navigation(
                    active_state.browsing().collection(),
                    active_state.browsing().active(),
                    direction,
                ),
                expected.clone(),
            );

            let before_browsing = active_state.browsing().clone();
            let reduction = reduce(&active_state, EditorCommand::Navigate { direction });
            prop_assert_eq!(reduction.state.browsing(), &before_browsing);
            match expected {
                NavigationTarget::Candidate(image_id) => {
                    assert!(matches!(
                        reduction.effects.as_slice(),
                        [Effect::DecodeImage { candidate, .. }] if candidate.id == image_id
                    ));
                }
                NavigationTarget::NoTarget => prop_assert!(reduction.effects.is_empty()),
                _ => unreachable!("nonempty collection with an active entry has a bounded target result"),
            }
        }

        for direction in [
            NavigationDirection::Left,
            NavigationDirection::Right,
            NavigationDirection::Home,
            NavigationDirection::End,
        ] {
            prop_assert_eq!(
                plan_navigation(
                    installed.browsing().collection(),
                    installed.browsing().active(),
                    direction,
                ),
                NavigationTarget::NoActiveImage,
            );
            let before_browsing = installed.browsing().clone();
            let reduction = reduce(&installed, EditorCommand::Navigate { direction });
            prop_assert_eq!(reduction.state.browsing(), &before_browsing);
            prop_assert!(reduction.effects.is_empty());
        }

        let empty_state = EditorState::new(capabilities());
        for direction in [
            NavigationDirection::Left,
            NavigationDirection::Right,
            NavigationDirection::Home,
            NavigationDirection::End,
        ] {
            prop_assert_eq!(
                plan_navigation(
                    empty_state.browsing().collection(),
                    empty_state.browsing().active(),
                    direction,
                ),
                NavigationTarget::EmptyCollection,
            );
            let before_browsing = empty_state.browsing().clone();
            let reduction = reduce(&empty_state, EditorCommand::Navigate { direction });
            prop_assert_eq!(reduction.state.browsing(), &before_browsing);
            prop_assert!(reduction.effects.is_empty());
        }
    }
}
