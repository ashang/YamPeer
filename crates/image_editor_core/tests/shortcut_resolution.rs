use std::collections::BTreeMap;

use image_editor_core::{
    AdjustmentKind, EditorCommand, EffectiveKeybindingMap, KeyModifiers, KeybindingAction,
    KeybindingGesture, NavigationDirection, PanDirection, RawKeyEvent, RuntimePlatform,
    ShortcutKey, ShortcutResolver, ZoomDirection, built_in_keybinding_map,
    keybinding_action_for_command, shortcut_label,
};

fn event(key: ShortcutKey, modifiers: KeyModifiers) -> RawKeyEvent {
    RawKeyEvent::press(key, modifiers)
}

#[test]
fn built_in_platform_maps_resolve_the_same_history_and_adjustment_commands() {
    let cases = [
        (
            event(ShortcutKey::Character('Z'), KeyModifiers::command()),
            event(ShortcutKey::Character('z'), KeyModifiers::control()),
            EditorCommand::Undo,
        ),
        (
            event(
                ShortcutKey::Character('z'),
                KeyModifiers::command().with_shift(),
            ),
            event(
                ShortcutKey::Character('z'),
                KeyModifiers::control().with_shift(),
            ),
            EditorCommand::Redo,
        ),
        (
            event(ShortcutKey::ArrowUp, KeyModifiers::option()),
            event(ShortcutKey::ArrowUp, KeyModifiers::alt()),
            EditorCommand::IncreaseAdjustment,
        ),
        (
            event(ShortcutKey::ArrowDown, KeyModifiers::option()),
            event(ShortcutKey::ArrowDown, KeyModifiers::alt()),
            EditorCommand::DecreaseAdjustment,
        ),
    ];

    let macos = ShortcutResolver::new(built_in_keybinding_map(RuntimePlatform::MacOs));
    let linux = ShortcutResolver::new(built_in_keybinding_map(RuntimePlatform::Linux));
    for (macos_event, linux_event, expected) in cases {
        assert_eq!(macos.resolve(macos_event), Some(expected.clone()));
        assert_eq!(linux.resolve(linux_event), Some(expected));
    }
}

#[test]
fn effective_map_routes_edit_navigation_view_and_fullscreen_actions() {
    let resolver = ShortcutResolver::new(built_in_keybinding_map(RuntimePlatform::Linux));
    let plain = KeyModifiers::default();
    let shifted = plain.with_shift();
    let cases = [
        (
            event(ShortcutKey::ArrowUp, plain),
            EditorCommand::Navigate {
                direction: NavigationDirection::Left,
            },
        ),
        (
            event(ShortcutKey::PageDown, plain),
            EditorCommand::Navigate {
                direction: NavigationDirection::Right,
            },
        ),
        (
            event(ShortcutKey::Home, plain),
            EditorCommand::Navigate {
                direction: NavigationDirection::Home,
            },
        ),
        (
            event(ShortcutKey::End, plain),
            EditorCommand::Navigate {
                direction: NavigationDirection::End,
            },
        ),
        (
            event(ShortcutKey::Character('0'), plain),
            EditorCommand::SetFitToWindow,
        ),
        (
            event(ShortcutKey::Character('2'), plain),
            EditorCommand::SetManualZoom { percent: 200 },
        ),
        (
            event(ShortcutKey::Character('+'), plain),
            EditorCommand::ZoomByStep {
                direction: ZoomDirection::In,
            },
        ),
        (
            event(ShortcutKey::Character('h'), plain),
            EditorCommand::PanCanvas {
                direction: PanDirection::Left,
            },
        ),
        (
            event(ShortcutKey::F11, plain),
            EditorCommand::ToggleFullscreen,
        ),
        (
            event(ShortcutKey::Character('f'), plain),
            EditorCommand::FlipHorizontal,
        ),
        (
            event(ShortcutKey::Character('F'), shifted),
            EditorCommand::FlipVertical,
        ),
        (
            event(ShortcutKey::Character('r'), plain),
            EditorCommand::RotateClockwise90,
        ),
        (
            event(ShortcutKey::Character('R'), shifted),
            EditorCommand::RotateCounterclockwise90,
        ),
        (
            event(ShortcutKey::Character('c'), plain),
            EditorCommand::EnterCrop,
        ),
        (
            event(ShortcutKey::Character('b'), plain),
            EditorCommand::FocusAdjustment(AdjustmentKind::Brightness),
        ),
        (
            event(ShortcutKey::Character('d'), plain),
            EditorCommand::FocusAdjustment(AdjustmentKind::Contrast),
        ),
        (
            event(ShortcutKey::Enter, plain),
            EditorCommand::CommitAdjustment,
        ),
    ];

    for (input, expected) in cases {
        assert_eq!(resolver.resolve(input), Some(expected));
    }
}

#[test]
fn resolver_uses_the_supplied_effective_map_not_a_fixed_shortcut_table() {
    let plain = KeyModifiers::default();
    let mut bindings = BTreeMap::new();
    bindings.insert(
        KeybindingAction::ZoomIn,
        vec![KeybindingGesture::new(ShortcutKey::Character('f'), plain)],
    );
    let map = EffectiveKeybindingMap::try_from_bindings(bindings).expect("unique gesture");
    let resolver = ShortcutResolver::new(map);

    assert_eq!(
        resolver.resolve(event(ShortcutKey::Character('f'), plain)),
        Some(EditorCommand::ZoomByStep {
            direction: ZoomDirection::In,
        })
    );
    assert_eq!(
        resolver.resolve(event(ShortcutKey::Character('r'), plain)),
        None,
        "the fixed rotate binding must not survive outside the effective map"
    );
}

#[test]
fn release_repeat_and_text_consumed_events_are_ignored() {
    let resolver = ShortcutResolver::new(built_in_keybinding_map(RuntimePlatform::Linux));
    let accepted = event(ShortcutKey::Character('f'), KeyModifiers::default());
    assert_eq!(
        resolver.resolve(accepted),
        Some(EditorCommand::FlipHorizontal)
    );

    let mut released = accepted;
    released.pressed = false;
    assert_eq!(resolver.resolve(released), None);

    let mut repeated = accepted;
    repeated.repeat = true;
    assert_eq!(resolver.resolve(repeated), None);

    let mut text_consumed = accepted;
    text_consumed.consumed_by_text_control = true;
    assert_eq!(resolver.resolve(text_consumed), None);
}

#[test]
fn shortcut_labels_are_derived_from_configured_gestures() {
    let macos = built_in_keybinding_map(RuntimePlatform::MacOs);
    let linux = built_in_keybinding_map(RuntimePlatform::Linux);
    assert_eq!(
        shortcut_label(RuntimePlatform::MacOs, &macos, KeybindingAction::Undo),
        Some("Command+Z".to_owned())
    );
    assert_eq!(
        shortcut_label(RuntimePlatform::Linux, &linux, KeybindingAction::Redo),
        Some("Control+Shift+Z".to_owned())
    );
    assert_eq!(
        shortcut_label(
            RuntimePlatform::MacOs,
            &macos,
            KeybindingAction::IncreaseAdjustment,
        ),
        Some("Option+Up".to_owned())
    );
    assert_eq!(
        shortcut_label(RuntimePlatform::Linux, &linux, KeybindingAction::NextImage,),
        Some("Right / Down / PageDown / Space".to_owned())
    );
    assert_eq!(
        keybinding_action_for_command(&EditorCommand::CancelCrop),
        None,
        "unconfigured commands must not present a shortcut label"
    );
}
