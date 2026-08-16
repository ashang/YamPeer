use std::collections::BTreeMap;

use image_editor_core::{
    AbsolutePath, Availability, CanonicalImage, CapabilitySnapshot, CodecProvider, DirectoryEntry,
    DirectoryEntryKind, DirectoryEntryLocation, EditorCommand, EditorState, EffectiveKeybindingMap,
    FormatCapability, ImageFormat, KeyModifiers, KeybindingAction, KeybindingGesture, LogicalSize,
    PanDirection, PlatformCapability, RationalScale, Rgba16, RuntimePlatform, ShortcutKey,
    ViewState, ZoomDirection, ZoomMode, built_in_keybinding_map, plan_folder_enumeration, reduce,
};

fn path(value: &str) -> AbsolutePath {
    AbsolutePath::new(value).unwrap()
}

fn capabilities() -> CapabilitySnapshot {
    let capability = || {
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
        formats.insert(format, capability());
    }
    CapabilitySnapshot::new(
        formats,
        PlatformCapability::available("test-folder-picker"),
        PlatformCapability::available("test-save-picker"),
    )
}

fn active_state() -> EditorState {
    let initial = EditorState::new(capabilities());
    let folder = path("/images");
    let enumeration = reduce(
        &initial,
        EditorCommand::BeginFolderEnumeration {
            folder: folder.clone(),
        },
    );
    let entry = DirectoryEntry::new(
        path("/images/photo.png"),
        image_editor_core::Utf8FileName::new("photo.png").unwrap(),
        DirectoryEntryLocation::Direct,
        DirectoryEntryKind::RegularFile,
        None,
    );
    let installed = reduce(
        &enumeration.state,
        EditorCommand::FolderEnumerated {
            token: enumeration.effects[0].token(),
            result: plan_folder_enumeration(
                &capabilities(),
                image_editor_core::FolderEnumerationInput::Succeeded {
                    folder,
                    entries: vec![entry],
                },
            ),
        },
    );
    let candidate = installed.state.browsing().collection().entries()[0].clone();
    let decoded = reduce(&installed.state, EditorCommand::BeginDecode { candidate });
    reduce(
        &decoded.state,
        EditorCommand::ImageDecoded {
            token: decoded.effects[0].token(),
            image: CanonicalImage::new(100, 100, vec![Rgba16::new(5, 7, 11, u16::MAX); 10_000])
                .unwrap(),
        },
    )
    .state
}

#[test]
fn built_in_keybindings_cover_required_view_navigation_and_platform_aliases() {
    let macos = built_in_keybinding_map(RuntimePlatform::MacOs);
    let linux = built_in_keybinding_map(RuntimePlatform::Linux);
    let plain = KeyModifiers::default();

    assert_eq!(
        macos.action_for(KeybindingGesture::new(ShortcutKey::Character('0'), plain)),
        Some(KeybindingAction::FitToWindow)
    );
    assert_eq!(
        macos.action_for(KeybindingGesture::new(ShortcutKey::Character('+'), plain)),
        Some(KeybindingAction::ZoomIn)
    );
    assert_eq!(
        macos.action_for(KeybindingGesture::new(ShortcutKey::PageUp, plain)),
        Some(KeybindingAction::PreviousImage)
    );
    assert_eq!(
        linux.action_for(KeybindingGesture::new(ShortcutKey::Space, plain)),
        Some(KeybindingAction::NextImage)
    );
    assert_eq!(
        macos.gestures_for(KeybindingAction::ToggleFullscreen),
        &[
            KeybindingGesture::new(
                ShortcutKey::Character('f'),
                KeyModifiers {
                    command: true,
                    control: true,
                    option: false,
                    alt: false,
                    shift: false,
                }
            ),
            KeybindingGesture::new(ShortcutKey::F11, plain),
        ]
    );
    assert_eq!(
        linux.gestures_for(KeybindingAction::ToggleFullscreen),
        &[KeybindingGesture::new(ShortcutKey::F11, plain)]
    );
}

#[test]
fn gestures_are_normalized_and_effective_maps_reject_cross_action_collisions() {
    let gesture = KeybindingGesture::new(ShortcutKey::Character('H'), KeyModifiers::default());
    assert_eq!(gesture.key, ShortcutKey::Character('h'));

    let mut bindings = BTreeMap::new();
    bindings.insert(KeybindingAction::PanLeft, vec![gesture]);
    bindings.insert(KeybindingAction::PanRight, vec![gesture]);
    assert_eq!(
        EffectiveKeybindingMap::try_from_bindings(bindings),
        Err(gesture)
    );
}

#[test]
fn view_state_fits_exact_manual_scales_steps_and_clamps_pan_to_image_bounds() {
    let image = LogicalSize::new(400, 100);
    let preview = LogicalSize::new(200, 200);
    let fitted = ViewState::for_preview_size(preview).fit_to_window(image);
    assert_eq!(fitted.zoom, ZoomMode::FitToWindow);
    assert_eq!(
        fitted.effective_scale(image),
        RationalScale::new(1, 2).unwrap()
    );

    let actual = fitted.set_manual_zoom(100, image);
    assert_eq!(actual.zoom, ZoomMode::Manual);
    assert_eq!(actual.manual_scale, RationalScale::ONE);
    let double = actual.set_manual_zoom(200, image);
    assert_eq!(double.manual_scale, RationalScale::TWO);
    assert_eq!(
        actual.zoom_by_step(ZoomDirection::In, image).manual_scale,
        RationalScale::new(5, 4).unwrap()
    );
    assert_eq!(
        actual
            .zoom_by_step(ZoomDirection::In, image)
            .zoom_by_step(ZoomDirection::Out, image)
            .manual_scale,
        RationalScale::ONE
    );

    let mut maximum = actual.clone();
    for _ in 0..20 {
        maximum = maximum.zoom_by_step(ZoomDirection::In, image);
    }
    assert_eq!(maximum.manual_scale, RationalScale::MAX);
    let mut minimum = actual;
    for _ in 0..20 {
        minimum = minimum.zoom_by_step(ZoomDirection::Out, image);
    }
    assert_eq!(minimum.manual_scale, RationalScale::MIN);

    let pan_image = LogicalSize::new(100, 100);
    let pan_preview = LogicalSize::new(100, 100);
    let mut panned = ViewState::for_preview_size(pan_preview).set_manual_zoom(200, pan_image);
    for _ in 0..10 {
        panned = panned.pan(PanDirection::Left, pan_image);
    }
    assert_eq!(panned.canvas_offset.x, -50);
    for _ in 0..10 {
        panned = panned.pan(PanDirection::Right, pan_image);
    }
    assert_eq!(panned.canvas_offset.x, 50);
    assert_eq!(
        ViewState::for_preview_size(pan_preview).pan(PanDirection::Down, pan_image),
        ViewState::for_preview_size(pan_preview)
    );
}

#[test]
fn view_commands_preserve_document_history_and_source_pixels_and_are_noops_without_active_image() {
    let inactive = EditorState::new(capabilities());
    for command in [
        EditorCommand::SetFitToWindow,
        EditorCommand::SetManualZoom { percent: 200 },
        EditorCommand::ZoomByStep {
            direction: ZoomDirection::In,
        },
        EditorCommand::PanCanvas {
            direction: PanDirection::Right,
        },
    ] {
        assert_eq!(reduce(&inactive, command).state, inactive);
    }

    let active = active_state();
    let image_id = active.browsing().active().unwrap().clone();
    let original = active.browsing().document(&image_id).unwrap().clone();
    let sized = reduce(
        &active,
        EditorCommand::SetPreviewSize {
            preview_size: LogicalSize::new(100, 100),
        },
    );
    let zoomed = reduce(&sized.state, EditorCommand::SetManualZoom { percent: 200 });
    let panned = reduce(
        &zoomed.state,
        EditorCommand::PanCanvas {
            direction: PanDirection::Right,
        },
    );

    let after = panned.state.browsing().document(&image_id).unwrap();
    assert_eq!(after.history(), original.history());
    assert_eq!(after.redo(), original.redo());
    assert_eq!(after.source(), original.source());
    assert_eq!(after.revision(), original.revision());
    assert_eq!(panned.effects, Vec::new());
    assert_eq!(panned.state.view_state().canvas_offset.x, 10);
}
