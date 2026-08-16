use std::collections::BTreeMap;

use image_editor_core::{
    AbsolutePath, ApplicationError, Availability, CanonicalImage, CapabilitySnapshot,
    CodecProvider, CollectionEntry, DirectoryEntry, DirectoryEntryKind, DirectoryEntryLocation,
    EditorCommand, EditorState, Effect, ErrorCategory, FolderEnumerationInput, FormatCapability,
    ImageFormat, NavigationDirection, NoticeSeverity, NoticeSubject, PlatformCapability,
    RequestToken, Rgba16, SafeError, Utf8FileName, plan_folder_enumeration, reduce,
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

fn image(value: u16) -> CanonicalImage {
    CanonicalImage::new(1, 1, vec![Rgba16::new(value, value, value, u16::MAX)])
        .expect("one pixel image is valid")
}

fn entry(index: usize) -> DirectoryEntry {
    let name = format!("{index:02}.png");
    DirectoryEntry::new(
        path(&format!("/photos/{name}")),
        Utf8FileName::new(name).expect("generated filename is valid"),
        DirectoryEntryLocation::Direct,
        DirectoryEntryKind::RegularFile,
        None,
    )
}

fn install_collection(entry_count: usize) -> EditorState {
    let initial = EditorState::new(capabilities());
    let requested = reduce(
        &initial,
        EditorCommand::BeginFolderEnumeration {
            folder: path("/photos"),
        },
    );
    let result = plan_folder_enumeration(
        &capabilities(),
        FolderEnumerationInput::Succeeded {
            folder: path("/photos"),
            entries: (0..entry_count).map(entry).collect(),
        },
    );
    let completed = reduce(
        &requested.state,
        EditorCommand::FolderEnumerated {
            token: token(&requested.effects[0]),
            result,
        },
    );
    completed.state
}

fn token(effect: &Effect) -> RequestToken {
    effect.token()
}

fn activate(state: &EditorState, candidate: CollectionEntry, value: u16) -> EditorState {
    let requested = reduce(state, EditorCommand::BeginDecode { candidate });
    let decoded = reduce(
        &requested.state,
        EditorCommand::ImageDecoded {
            token: token(&requested.effects[0]),
            image: image(value),
        },
    );
    let rendered = reduce(
        &decoded.state,
        EditorCommand::PreviewRendered {
            token: token(&decoded.effects[0]),
            image: image(value),
        },
    );
    rendered.state
}

fn request_for_route(
    state: &EditorState,
    entries: &[CollectionEntry],
    route: u8,
    seed: usize,
) -> (EditorState, EditorCommand, CollectionEntry) {
    let last = entries.len() - 1;
    match route % 5 {
        0 => {
            let active_index = seed % entries.len();
            let candidate_index = (active_index + 1) % entries.len();
            (
                activate(state, entries[active_index].clone(), active_index as u16),
                EditorCommand::SelectImage {
                    image_id: entries[candidate_index].id.clone(),
                },
                entries[candidate_index].clone(),
            )
        }
        1 => {
            let active_index = seed % last;
            let candidate_index = active_index + 1;
            (
                activate(state, entries[active_index].clone(), active_index as u16),
                EditorCommand::Navigate {
                    direction: NavigationDirection::Right,
                },
                entries[candidate_index].clone(),
            )
        }
        2 => {
            let active_index = 1 + (seed % last);
            let candidate_index = active_index - 1;
            (
                activate(state, entries[active_index].clone(), active_index as u16),
                EditorCommand::Navigate {
                    direction: NavigationDirection::Left,
                },
                entries[candidate_index].clone(),
            )
        }
        3 => {
            let active_index = 1 + (seed % last);
            (
                activate(state, entries[active_index].clone(), active_index as u16),
                EditorCommand::Navigate {
                    direction: NavigationDirection::Home,
                },
                entries[0].clone(),
            )
        }
        _ => {
            let active_index = seed % last;
            (
                activate(state, entries[active_index].clone(), active_index as u16),
                EditorCommand::Navigate {
                    direction: NavigationDirection::End,
                },
                entries[last].clone(),
            )
        }
    }
}

// Feature: macos-image-editor, Property 2: Candidate activation is atomic
// Validates: Requirements 1.5-1.7, 2.1-2.9
proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    #[test]
    fn candidate_activation_is_atomic_for_selection_and_navigation_completions(
        entry_count in 2usize..7,
        route in 0u8..5,
        seed in any::<usize>(),
        decode_succeeds in any::<bool>(),
        decoded_value in any::<u16>(),
    ) {
        let collection_state = install_collection(entry_count);
        let entries = collection_state.browsing().collection().entries();
        let (prior_state, command, candidate) =
            request_for_route(&collection_state, entries, route, seed);
        let prior_browsing = prior_state.browsing().clone();
        let prior_active = prior_browsing.active().cloned().expect("route installs an active image");
        let prior_document = prior_browsing
            .document(&prior_active)
            .cloned()
            .expect("active image has a retained document");

        let requested = reduce(&prior_state, command);
        prop_assert_eq!(requested.state.browsing(), &prior_browsing);
        prop_assert!(
            matches!(
                &requested.effects[..],
                [Effect::DecodeImage { candidate: requested_candidate, .. }]
                    if requested_candidate == &candidate
            ),
            "selection or navigation must request the expected candidate decode"
        );

        if decode_succeeds {
            let decoded = reduce(
                &requested.state,
                EditorCommand::ImageDecoded {
                    token: token(&requested.effects[0]),
                    image: image(decoded_value),
                },
            );

            prop_assert_eq!(decoded.state.browsing().source_folder(), prior_browsing.source_folder());
            prop_assert_eq!(decoded.state.browsing().collection(), prior_browsing.collection());
            prop_assert_eq!(decoded.state.browsing().active(), Some(&candidate.id));
            prop_assert_eq!(decoded.state.browsing().document(&prior_active), Some(&prior_document));
            prop_assert!(
                matches!(
                    decoded.state.browsing().preview(),
                    image_editor_core::PreviewState::Pending { image_id, .. }
                        if image_id == &candidate.id
                ),
                "successful decode must begin a preview for the activated candidate"
            );
            prop_assert!(
                matches!(&decoded.effects[..], [Effect::RenderPreview { .. }]),
                "successful decode must emit one preview render effect"
            );

            let rendered = reduce(
                &decoded.state,
                EditorCommand::PreviewRendered {
                    token: token(&decoded.effects[0]),
                    image: image(decoded_value),
                },
            );
            prop_assert!(
                matches!(
                    rendered.state.browsing().preview(),
                    image_editor_core::PreviewState::Rendered {
                        image_id,
                        image: rendered_image,
                        ..
                    } if image_id == &candidate.id && rendered_image == &image(decoded_value)
                ),
                "successful preview completion must render the activated candidate"
            );
        } else {
            let failed = reduce(
                &requested.state,
                EditorCommand::OperationFailed {
                    token: token(&requested.effects[0]),
                    error: ApplicationError::Decode {
                        file_name: Utf8FileName::new("untrusted-name.png").expect("valid test name"),
                        cause: SafeError::new(ErrorCategory::PortableCodec, "decode test failure"),
                    },
                },
            );

            prop_assert_eq!(failed.state.browsing(), &prior_browsing);
            prop_assert!(failed.effects.is_empty());
            let notice = failed.state.notices().last().expect("decode failure is visible");
            prop_assert_eq!(notice.severity, NoticeSeverity::Error);
            prop_assert_eq!(
                &notice.subject,
                &NoticeSubject::FileName(candidate.file_name.clone())
            );
            prop_assert_eq!(notice.message.summary(), "decode test failure");
        }
    }
}
