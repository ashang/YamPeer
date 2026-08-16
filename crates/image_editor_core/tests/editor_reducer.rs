use std::collections::BTreeMap;

use image_editor_core::{
    AbsolutePath, ApplicationError, Availability, CanonicalImage, CapabilitySnapshot,
    CodecProvider, CollectionEntry, CropDraft, CropRect, DirectoryEntry, DirectoryEntryKind,
    DirectoryEntryLocation, EditOperation, EditorCommand, EditorState, Effect, ErrorCategory,
    FolderEnumerationInput, FolderEnumerationPlan, FormatCapability, ImageFormat, InteractionMode,
    NavigationDirection, NavigationTarget, NoticeSeverity, NoticeSubject, PlatformCapability,
    RequestToken, Rgba16, SafeError, Utf8FileName, plan_folder_enumeration, plan_navigation,
    reduce,
};

fn path(value: &str) -> AbsolutePath {
    AbsolutePath::new(value).unwrap()
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
    formats.insert(ImageFormat::Jpeg, available());
    formats.insert(ImageFormat::Png, available());
    formats.insert(ImageFormat::Tiff, available());
    formats.insert(ImageFormat::Heic, available());
    CapabilitySnapshot::new(
        formats,
        PlatformCapability::available("test-folder-picker"),
        PlatformCapability::available("test-save-picker"),
    )
}

fn image(value: u16) -> CanonicalImage {
    CanonicalImage::new(1, 1, vec![Rgba16::new(value, value, value, u16::MAX)]).unwrap()
}

fn entry(full_path: &str, name: &str) -> DirectoryEntry {
    DirectoryEntry::new(
        path(full_path),
        Utf8FileName::new(name).unwrap(),
        DirectoryEntryLocation::Direct,
        DirectoryEntryKind::RegularFile,
        None,
    )
}

fn plan(folder: &str, entries: Vec<DirectoryEntry>) -> FolderEnumerationPlan {
    plan_folder_enumeration(
        &capabilities(),
        FolderEnumerationInput::Succeeded {
            folder: path(folder),
            entries,
        },
    )
}

fn token(effect: &Effect) -> RequestToken {
    effect.token()
}

fn install_collection(entries: Vec<DirectoryEntry>) -> EditorState {
    let state = EditorState::new(capabilities());
    let requested = reduce(
        &state,
        EditorCommand::BeginFolderEnumeration {
            folder: path("/photos"),
        },
    );
    let completed = reduce(
        &requested.state,
        EditorCommand::FolderEnumerated {
            token: token(&requested.effects[0]),
            result: plan("/photos", entries),
        },
    );
    completed.state
}

#[test]
fn stale_folder_completion_cannot_replace_newer_browsing_state() {
    let state = EditorState::new(capabilities());
    let first = reduce(
        &state,
        EditorCommand::BeginFolderEnumeration {
            folder: path("/older"),
        },
    );
    let second = reduce(
        &first.state,
        EditorCommand::BeginFolderEnumeration {
            folder: path("/newer"),
        },
    );
    let installed = reduce(
        &second.state,
        EditorCommand::FolderEnumerated {
            token: token(&second.effects[0]),
            result: plan("/newer", vec![entry("/newer/new.png", "new.png")]),
        },
    );
    let after_stale = reduce(
        &installed.state,
        EditorCommand::FolderEnumerated {
            token: token(&first.effects[0]),
            result: plan("/older", vec![entry("/older/old.png", "old.png")]),
        },
    );

    assert_eq!(
        after_stale.state.browsing().source_folder(),
        Some(&path("/newer"))
    );
    assert_eq!(
        after_stale.state.browsing().collection().entries()[0]
            .file_name
            .as_str(),
        "new.png"
    );
    assert!(after_stale.effects.is_empty());
}

#[test]
fn stale_decode_and_preview_completions_cannot_replace_newer_image_or_preview() {
    let state = install_collection(vec![
        entry("/photos/first.png", "first.png"),
        entry("/photos/second.png", "second.png"),
    ]);
    let first = state.browsing().collection().entries()[0].clone();
    let first_id = first.id.clone();
    let second = state.browsing().collection().entries()[1].clone();

    let first_decode = reduce(&state, EditorCommand::BeginDecode { candidate: first });
    let second_decode = reduce(
        &first_decode.state,
        EditorCommand::BeginDecode {
            candidate: second.clone(),
        },
    );
    let decoded = reduce(
        &second_decode.state,
        EditorCommand::ImageDecoded {
            token: token(&second_decode.effects[0]),
            image: image(2),
        },
    );
    let preview_first = token(&decoded.effects[0]);
    let newer_preview = reduce(
        &decoded.state,
        EditorCommand::RequestPreview {
            image_id: second.id.clone(),
        },
    );
    let preview_second = token(&newer_preview.effects[0]);
    let rendered = reduce(
        &newer_preview.state,
        EditorCommand::PreviewRendered {
            token: preview_second,
            image: image(22),
        },
    );
    let after_stale_preview = reduce(
        &rendered.state,
        EditorCommand::PreviewRendered {
            token: preview_first,
            image: image(11),
        },
    );
    let after_stale_decode = reduce(
        &after_stale_preview.state,
        EditorCommand::ImageDecoded {
            token: token(&first_decode.effects[0]),
            image: image(1),
        },
    );

    assert_eq!(
        after_stale_decode.state.browsing().active(),
        Some(&second.id)
    );
    assert_eq!(
        after_stale_decode.state.browsing().document(&first_id),
        None,
        "stale decode must not create a document"
    );
    assert!(matches!(
        after_stale_decode.state.browsing().preview(),
        image_editor_core::PreviewState::Rendered { image, .. }
            if image.pixels()[0].red == 22
    ));
}

#[test]
fn stale_export_completion_does_not_report_success_after_document_is_discarded() {
    let state = install_collection(vec![entry("/photos/photo.png", "photo.png")]);
    let candidate = state.browsing().collection().entries()[0].clone();
    let decoded = reduce(&state, EditorCommand::BeginDecode { candidate });
    let active = reduce(
        &decoded.state,
        EditorCommand::ImageDecoded {
            token: token(&decoded.effects[0]),
            image: image(3),
        },
    );
    let export = reduce(
        &active.state,
        EditorCommand::BeginExport {
            target: path("/exports/photo.png"),
            format: ImageFormat::Png,
        },
    );
    let replacement_request = reduce(
        &export.state,
        EditorCommand::BeginFolderEnumeration {
            folder: path("/replacement"),
        },
    );
    let replacement = reduce(
        &replacement_request.state,
        EditorCommand::FolderEnumerated {
            token: token(&replacement_request.effects[0]),
            result: plan("/replacement", vec![]),
        },
    );
    let after_stale_export = reduce(
        &replacement.state,
        EditorCommand::ExportWritten {
            token: token(&export.effects[0]),
        },
    );

    assert_eq!(
        after_stale_export.state.browsing().source_folder(),
        Some(&path("/replacement"))
    );
    assert!(
        after_stale_export
            .state
            .notices()
            .iter()
            .all(|notice| notice.message.summary() != "export completed")
    );
}

#[test]
fn edit_commands_without_an_active_image_are_errors_not_deferred_commands() {
    let state = EditorState::new(capabilities());
    for command in [
        EditorCommand::FlipHorizontal,
        EditorCommand::FlipVertical,
        EditorCommand::RotateClockwise90,
        EditorCommand::RotateCounterclockwise90,
        EditorCommand::EnterCrop,
        EditorCommand::FocusAdjustment(image_editor_core::AdjustmentKind::Brightness),
        EditorCommand::Undo,
        EditorCommand::Redo,
    ] {
        let before_browsing = state.browsing().clone();
        let reduction = reduce(&state, command);
        assert_eq!(reduction.state.browsing(), &before_browsing);
        assert!(reduction.effects.is_empty());
        assert_eq!(
            reduction.state.notices().last().unwrap().severity,
            NoticeSeverity::Error
        );
    }
}

#[test]
fn geometric_commands_append_once_clear_redo_advance_revision_and_request_mapped_preview() {
    let source = CanonicalImage::new(
        2,
        3,
        (1..=6)
            .map(|value| Rgba16::new(value, 0, 0, u16::MAX))
            .collect(),
    )
    .unwrap();
    let cases = [
        (
            EditorCommand::FlipHorizontal,
            EditOperation::FlipHorizontal,
            (2, 3),
            vec![2, 1, 4, 3, 6, 5],
        ),
        (
            EditorCommand::FlipVertical,
            EditOperation::FlipVertical,
            (2, 3),
            vec![5, 6, 3, 4, 1, 2],
        ),
        (
            EditorCommand::RotateClockwise90,
            EditOperation::RotateClockwise90,
            (3, 2),
            vec![5, 3, 1, 6, 4, 2],
        ),
        (
            EditorCommand::RotateCounterclockwise90,
            EditOperation::RotateCounterclockwise90,
            (3, 2),
            vec![2, 4, 6, 1, 3, 5],
        ),
    ];

    for (command, operation, dimensions, expected_red_samples) in cases {
        let state = install_collection(vec![entry("/photos/photo.png", "photo.png")]);
        let candidate = state.browsing().collection().entries()[0].clone();
        let decoded = reduce(&state, EditorCommand::BeginDecode { candidate });
        let active = reduce(
            &decoded.state,
            EditorCommand::ImageDecoded {
                token: token(&decoded.effects[0]),
                image: source.clone(),
            },
        );
        let image_id = active.state.browsing().active().unwrap().clone();
        let edited = reduce(&active.state, command);

        let document = edited.state.browsing().document(&image_id).unwrap();
        assert_eq!(document.history(), &[operation.clone()]);
        assert!(document.redo().is_empty());
        assert_eq!(document.revision().get(), 1);
        let Effect::RenderPreview { request, .. } = &edited.effects[0] else {
            panic!("a geometric edit must request a preview render");
        };
        assert_eq!(request.image_id, image_id);
        assert_eq!(request.revision, document.revision());
        assert_eq!(request.history, &[operation]);

        let rendered = image_editor_core::render_current_editing_result(
            &request.source,
            &request.history,
            &request.draft,
        )
        .unwrap();
        assert_eq!((rendered.width(), rendered.height()), dimensions);
        assert_eq!(
            rendered
                .pixels()
                .iter()
                .map(|pixel| pixel.red)
                .collect::<Vec<_>>(),
            expected_red_samples
        );
    }
}

#[test]
fn decode_completion_reuses_the_existing_per_image_document() {
    let state = install_collection(vec![entry("/photos/photo.png", "photo.png")]);
    let candidate: CollectionEntry = state.browsing().collection().entries()[0].clone();
    let image_id = candidate.id.clone();
    let initial = reduce(
        &state,
        EditorCommand::BeginDecode {
            candidate: candidate.clone(),
        },
    );
    let active = reduce(
        &initial.state,
        EditorCommand::ImageDecoded {
            token: token(&initial.effects[0]),
            image: image(7),
        },
    );
    let document = active.state.browsing().document(&image_id).unwrap().clone();
    let repeated = reduce(&active.state, EditorCommand::BeginDecode { candidate });
    let completed = reduce(
        &repeated.state,
        EditorCommand::ImageDecoded {
            token: token(&repeated.effects[0]),
            image: image(99),
        },
    );

    assert_eq!(
        completed.state.browsing().documents().len(),
        1,
        "one source identity owns one retained document"
    );
    assert_eq!(
        completed.state.browsing().document(&image_id),
        Some(&document)
    );
}

#[test]
fn selection_activates_only_after_decode_success_and_failures_name_the_candidate() {
    let state = install_collection(vec![
        entry("/photos/first.png", "first.png"),
        entry("/photos/second.png", "second.png"),
    ]);
    let first = state.browsing().collection().entries()[0].clone();
    let second = state.browsing().collection().entries()[1].clone();

    let selection = reduce(
        &state,
        EditorCommand::SelectImage {
            image_id: first.id.clone(),
        },
    );
    assert_eq!(selection.state.browsing().active(), None);
    assert!(matches!(
        &selection.effects[..],
        [Effect::DecodeImage { candidate, .. }] if candidate == &first
    ));

    let activated = reduce(
        &selection.state,
        EditorCommand::ImageDecoded {
            token: token(&selection.effects[0]),
            image: image(1),
        },
    );
    assert_eq!(activated.state.browsing().active(), Some(&first.id));

    let before_failure = activated.state.browsing().clone();
    let failed_selection = reduce(
        &activated.state,
        EditorCommand::SelectImage {
            image_id: second.id.clone(),
        },
    );
    let after_failure = reduce(
        &failed_selection.state,
        EditorCommand::OperationFailed {
            token: token(&failed_selection.effects[0]),
            error: ApplicationError::Decode {
                file_name: first.file_name.clone(),
                cause: SafeError::new(ErrorCategory::PortableCodec, "malformed image data"),
            },
        },
    );

    assert_eq!(after_failure.state.browsing(), &before_failure);
    assert!(matches!(
        after_failure.state.notices().last(),
        Some(notice) if notice.subject == NoticeSubject::FileName(second.file_name)
            && notice.message.summary() == "malformed image data"
    ));
}

#[test]
fn navigation_plans_ordered_nonwrapping_targets_and_retains_no_active_state() {
    let initial = install_collection(vec![
        entry("/photos/first.png", "first.png"),
        entry("/photos/second.png", "second.png"),
    ]);
    let first = initial.browsing().collection().entries()[0].clone();
    let second = initial.browsing().collection().entries()[1].clone();

    for direction in [
        NavigationDirection::Left,
        NavigationDirection::Right,
        NavigationDirection::Home,
        NavigationDirection::End,
    ] {
        assert_eq!(
            plan_navigation(
                initial.browsing().collection(),
                initial.browsing().active(),
                direction,
            ),
            NavigationTarget::NoActiveImage
        );
        let no_active = reduce(&initial, EditorCommand::Navigate { direction });
        assert_eq!(no_active.state.browsing(), initial.browsing());
        assert!(no_active.effects.is_empty());
    }

    let first_request = reduce(
        &initial,
        EditorCommand::SelectImage {
            image_id: first.id.clone(),
        },
    );
    let first_active = reduce(
        &first_request.state,
        EditorCommand::ImageDecoded {
            token: token(&first_request.effects[0]),
            image: image(1),
        },
    );

    for direction in [NavigationDirection::Left, NavigationDirection::Home] {
        assert_eq!(
            plan_navigation(
                first_active.state.browsing().collection(),
                first_active.state.browsing().active(),
                direction,
            ),
            NavigationTarget::NoTarget
        );
        let boundary = reduce(&first_active.state, EditorCommand::Navigate { direction });
        assert_eq!(boundary.state.browsing(), first_active.state.browsing());
        assert!(boundary.effects.is_empty());
    }

    for direction in [NavigationDirection::Right, NavigationDirection::End] {
        assert_eq!(
            plan_navigation(
                first_active.state.browsing().collection(),
                first_active.state.browsing().active(),
                direction,
            ),
            NavigationTarget::Candidate(second.id.clone())
        );
    }

    let navigation = reduce(
        &first_active.state,
        EditorCommand::Navigate {
            direction: NavigationDirection::Right,
        },
    );
    assert!(matches!(
        &navigation.effects[..],
        [Effect::DecodeImage { candidate, .. }] if candidate == &second
    ));
    let second_active = reduce(
        &navigation.state,
        EditorCommand::ImageDecoded {
            token: token(&navigation.effects[0]),
            image: image(2),
        },
    );
    assert_eq!(second_active.state.browsing().active(), Some(&second.id));
    for direction in [NavigationDirection::Right, NavigationDirection::End] {
        assert_eq!(
            plan_navigation(
                second_active.state.browsing().collection(),
                second_active.state.browsing().active(),
                direction,
            ),
            NavigationTarget::NoTarget
        );
    }
}

#[test]
fn empty_collection_navigation_exposes_an_empty_preview_without_effects() {
    let state = EditorState::new(capabilities());
    assert_eq!(
        plan_navigation(
            state.browsing().collection(),
            state.browsing().active(),
            NavigationDirection::Right,
        ),
        NavigationTarget::EmptyCollection
    );

    let reduction = reduce(
        &state,
        EditorCommand::Navigate {
            direction: NavigationDirection::Right,
        },
    );
    assert_eq!(reduction.state.browsing(), state.browsing());
    assert!(reduction.effects.is_empty());
}

fn activate(source: CanonicalImage) -> (EditorState, image_editor_core::ImageId) {
    let state = install_collection(vec![entry("/photos/photo.png", "photo.png")]);
    let candidate = state.browsing().collection().entries()[0].clone();
    let image_id = candidate.id.clone();
    let requested = reduce(&state, EditorCommand::BeginDecode { candidate });
    let active = reduce(
        &requested.state,
        EditorCommand::ImageDecoded {
            token: token(&requested.effects[0]),
            image: source,
        },
    );
    (active.state, image_id)
}

fn crop_source() -> CanonicalImage {
    CanonicalImage::new(
        3,
        2,
        (1..=6)
            .map(|value| Rgba16::new(value, 0, 0, u16::MAX))
            .collect(),
    )
    .unwrap()
}

#[test]
fn crop_entry_uses_current_source_dimensions_and_clamps_draft_coordinates() {
    let (active, _) = activate(crop_source());
    let entered = reduce(&active, EditorCommand::EnterCrop);
    assert_eq!(
        entered.state.mode(),
        InteractionMode::Crop(CropDraft::new(0, 0, 3, 2))
    );
    assert!(entered.effects.is_empty());

    let updated = reduce(
        &entered.state,
        EditorCommand::UpdateCropDraft {
            draft: CropDraft::new(99, 1, 2, 99),
        },
    );
    assert_eq!(
        updated.state.mode(),
        InteractionMode::Crop(CropDraft::new(3, 1, 2, 2))
    );
    assert!(updated.effects.is_empty());
}

#[test]
fn valid_crop_commits_once_requests_a_preview_and_copies_the_exact_half_open_rectangle() {
    let (active, image_id) = activate(crop_source());
    let entered = reduce(&active, EditorCommand::EnterCrop);
    let selected = reduce(
        &entered.state,
        EditorCommand::UpdateCropDraft {
            draft: CropDraft::new(1, 0, 3, 2),
        },
    );
    let confirmed = reduce(&selected.state, EditorCommand::ConfirmCrop);

    let crop = CropRect::new(3, 2, 1, 0, 3, 2).unwrap();
    let document = confirmed.state.browsing().document(&image_id).unwrap();
    assert_eq!(document.history(), &[EditOperation::Crop(crop)]);
    assert!(document.redo().is_empty());
    assert_eq!(document.revision().get(), 1);
    assert_eq!(confirmed.state.mode(), InteractionMode::Browse);
    let Effect::RenderPreview { request, .. } = &confirmed.effects[0] else {
        panic!("confirming a valid crop must request a preview");
    };
    assert_eq!(request.history, vec![EditOperation::Crop(crop)]);
    let rendered = image_editor_core::render_current_editing_result(
        &request.source,
        &request.history,
        &request.draft,
    )
    .unwrap();
    assert_eq!((rendered.width(), rendered.height()), (2, 2));
    assert_eq!(
        rendered
            .pixels()
            .iter()
            .map(|pixel| pixel.red)
            .collect::<Vec<_>>(),
        vec![2, 3, 5, 6]
    );
}

#[test]
fn invalid_crop_confirmation_retains_the_draft_history_and_preview() {
    let (active, _) = activate(crop_source());
    let entered = reduce(&active, EditorCommand::EnterCrop);
    let invalid = reduce(
        &entered.state,
        EditorCommand::UpdateCropDraft {
            draft: CropDraft::new(2, 0, 2, 2),
        },
    );
    let prior_browsing = invalid.state.browsing().clone();
    let prior_mode = invalid.state.mode();

    let rejected = reduce(&invalid.state, EditorCommand::ConfirmCrop);

    assert_eq!(rejected.state.browsing(), &prior_browsing);
    assert_eq!(rejected.state.mode(), prior_mode);
    assert!(rejected.effects.is_empty());
    assert!(matches!(
        rejected.state.notices().last(),
        Some(notice) if notice.severity == NoticeSeverity::Error
            && notice.message.summary().contains("invalid crop")
    ));
}

#[test]
fn cancelling_crop_exits_the_mode_without_changing_document_or_preview() {
    let (active, _) = activate(crop_source());
    let entered = reduce(&active, EditorCommand::EnterCrop);
    let selected = reduce(
        &entered.state,
        EditorCommand::UpdateCropDraft {
            draft: CropDraft::new(1, 0, 3, 2),
        },
    );
    let prior_browsing = selected.state.browsing().clone();

    let cancelled = reduce(&selected.state, EditorCommand::CancelCrop);

    assert_eq!(cancelled.state.browsing(), &prior_browsing);
    assert_eq!(cancelled.state.mode(), InteractionMode::Browse);
    assert!(cancelled.effects.is_empty());
}

#[test]
fn crop_commands_without_an_active_image_are_visible_errors_without_deferred_work() {
    let state = EditorState::new(capabilities());
    for command in [
        EditorCommand::EnterCrop,
        EditorCommand::UpdateCropDraft {
            draft: CropDraft::new(0, 0, 1, 1),
        },
        EditorCommand::ConfirmCrop,
        EditorCommand::CancelCrop,
    ] {
        let reduction = reduce(&state, command);
        assert_eq!(reduction.state.browsing(), state.browsing());
        assert_eq!(reduction.state.mode(), InteractionMode::Browse);
        assert!(reduction.effects.is_empty());
        assert_eq!(
            reduction.state.notices().last().unwrap().severity,
            NoticeSeverity::Error
        );
    }
}

#[test]
fn adjustment_commands_focus_step_clamp_and_commit_only_the_focused_draft() {
    let source =
        CanonicalImage::new(1, 1, vec![Rgba16::new(32_768, 32_768, 32_768, 4_242)]).unwrap();
    let (active, image_id) = activate(source);

    let focused = reduce(
        &active,
        EditorCommand::FocusAdjustment(image_editor_core::AdjustmentKind::Brightness),
    );
    assert_eq!(
        focused.state.mode(),
        InteractionMode::Adjust(image_editor_core::AdjustmentKind::Brightness)
    );
    assert_eq!(
        focused
            .state
            .browsing()
            .document(&image_id)
            .unwrap()
            .draft()
            .focused(),
        Some(image_editor_core::AdjustmentKind::Brightness)
    );
    assert!(matches!(
        focused.effects.as_slice(),
        [Effect::RenderPreview { .. }]
    ));

    let mut stepped = focused;
    for _ in 0..100 {
        stepped = reduce(&stepped.state, EditorCommand::IncreaseAdjustment);
    }
    let document = stepped.state.browsing().document(&image_id).unwrap();
    assert_eq!(document.draft().brightness().get(), 100);
    assert_eq!(document.draft().contrast().get(), 0);

    let endpoint = reduce(&stepped.state, EditorCommand::IncreaseAdjustment);
    assert_eq!(endpoint.state, stepped.state, "an endpoint step is a no-op");
    assert!(endpoint.effects.is_empty());

    let contrast = reduce(
        &endpoint.state,
        EditorCommand::FocusAdjustment(image_editor_core::AdjustmentKind::Contrast),
    );
    let decremented = reduce(&contrast.state, EditorCommand::DecreaseAdjustment);
    let document = decremented.state.browsing().document(&image_id).unwrap();
    assert_eq!(document.draft().brightness().get(), 100);
    assert_eq!(document.draft().contrast().get(), -1);

    let committed = reduce(&decremented.state, EditorCommand::CommitAdjustment);
    let document = committed.state.browsing().document(&image_id).unwrap();
    assert_eq!(
        document.history(),
        &[
            EditOperation::brightness(100).unwrap(),
            EditOperation::contrast(-1).unwrap(),
        ]
    );
    assert!(document.redo().is_empty());
    assert_eq!(document.revision().get(), 1);
    assert_eq!(document.draft().brightness().get(), 0);
    assert_eq!(document.draft().contrast().get(), 0);
    assert_eq!(document.draft().focused(), None);
    assert_eq!(committed.state.mode(), InteractionMode::Browse);
    assert!(matches!(
        committed.effects.as_slice(),
        [Effect::RenderPreview { .. }]
    ));
}

#[test]
fn committing_a_focused_zero_adjustment_records_an_identity_operation() {
    let (active, image_id) = activate(image(7));
    let focused = reduce(
        &active,
        EditorCommand::FocusAdjustment(image_editor_core::AdjustmentKind::Brightness),
    );
    let committed = reduce(&focused.state, EditorCommand::CommitAdjustment);

    let document = committed.state.browsing().document(&image_id).unwrap();
    assert_eq!(document.history(), &[EditOperation::brightness(0).unwrap()]);
    assert_eq!(document.draft().brightness().get(), 0);
    assert_eq!(document.draft().focused(), None);
    assert_eq!(committed.state.mode(), InteractionMode::Browse);
    assert!(matches!(
        committed.effects.as_slice(),
        [Effect::RenderPreview { .. }]
    ));
}

#[test]
fn undo_and_redo_are_lifo_per_active_document_and_branch_without_touching_other_documents() {
    let initial = install_collection(vec![
        entry("/photos/first.png", "first.png"),
        entry("/photos/second.png", "second.png"),
    ]);
    let first = initial.browsing().collection().entries()[0].clone();
    let second = initial.browsing().collection().entries()[1].clone();

    let first_decode = reduce(
        &initial,
        EditorCommand::BeginDecode {
            candidate: first.clone(),
        },
    );
    let first_active = reduce(
        &first_decode.state,
        EditorCommand::ImageDecoded {
            token: token(&first_decode.effects[0]),
            image: image(1),
        },
    );
    let first_flipped = reduce(&first_active.state, EditorCommand::FlipHorizontal);
    let first_rotated = reduce(&first_flipped.state, EditorCommand::RotateClockwise90);

    let first_undone = reduce(&first_rotated.state, EditorCommand::Undo);
    let first_document = first_undone.state.browsing().document(&first.id).unwrap();
    assert_eq!(first_document.history(), &[EditOperation::FlipHorizontal]);
    assert_eq!(first_document.redo(), &[EditOperation::RotateClockwise90]);
    assert_eq!(first_document.revision().get(), 3);
    assert!(matches!(
        first_undone.effects.as_slice(),
        [Effect::RenderPreview { .. }]
    ));

    let first_redone = reduce(&first_undone.state, EditorCommand::Redo);
    let first_document = first_redone.state.browsing().document(&first.id).unwrap();
    assert_eq!(
        first_document.history(),
        &[
            EditOperation::FlipHorizontal,
            EditOperation::RotateClockwise90
        ]
    );
    assert!(first_document.redo().is_empty());
    assert_eq!(first_document.revision().get(), 4);
    assert!(matches!(
        first_redone.effects.as_slice(),
        [Effect::RenderPreview { .. }]
    ));

    let first_undone_again = reduce(&first_redone.state, EditorCommand::Undo);
    let second_decode = reduce(
        &first_undone_again.state,
        EditorCommand::BeginDecode {
            candidate: second.clone(),
        },
    );
    let second_active = reduce(
        &second_decode.state,
        EditorCommand::ImageDecoded {
            token: token(&second_decode.effects[0]),
            image: image(2),
        },
    );
    let second_flipped = reduce(&second_active.state, EditorCommand::FlipVertical);
    let second_rotated = reduce(
        &second_flipped.state,
        EditorCommand::RotateCounterclockwise90,
    );
    let second_undone = reduce(&second_rotated.state, EditorCommand::Undo);
    let retained_second = second_undone
        .state
        .browsing()
        .document(&second.id)
        .unwrap()
        .clone();

    let first_reselected = reduce(
        &second_undone.state,
        EditorCommand::BeginDecode {
            candidate: first.clone(),
        },
    );
    let first_active_again = reduce(
        &first_reselected.state,
        EditorCommand::ImageDecoded {
            token: token(&first_reselected.effects[0]),
            image: image(99),
        },
    );
    let first_branched = reduce(&first_active_again.state, EditorCommand::FlipVertical);

    let first_document = first_branched.state.browsing().document(&first.id).unwrap();
    assert_eq!(
        first_document.history(),
        &[EditOperation::FlipHorizontal, EditOperation::FlipVertical]
    );
    assert!(
        first_document.redo().is_empty(),
        "a new edit clears the active redo stack"
    );
    assert_eq!(
        first_branched.state.browsing().document(&second.id),
        Some(&retained_second),
        "editing one document must not modify another document's history or redo stack"
    );
}

#[test]
fn undo_and_redo_with_empty_stacks_are_exact_no_ops() {
    let (active, _) = activate(image(7));

    for command in [EditorCommand::Undo, EditorCommand::Redo] {
        let reduction = reduce(&active, command);
        assert_eq!(reduction.state, active);
        assert!(reduction.effects.is_empty());
    }
}

#[test]
fn adjustment_commands_without_an_active_image_preserve_browsing_state() {
    let state = EditorState::new(capabilities());
    for command in [
        EditorCommand::FocusAdjustment(image_editor_core::AdjustmentKind::Brightness),
        EditorCommand::FocusAdjustment(image_editor_core::AdjustmentKind::Contrast),
        EditorCommand::IncreaseAdjustment,
        EditorCommand::DecreaseAdjustment,
        EditorCommand::CommitAdjustment,
    ] {
        let reduction = reduce(&state, command);
        assert_eq!(reduction.state.browsing(), state.browsing());
        assert_eq!(reduction.state.mode(), InteractionMode::Browse);
        assert!(reduction.effects.is_empty());
        assert_eq!(
            reduction.state.notices().last().unwrap().severity,
            NoticeSeverity::Error
        );
    }
}
