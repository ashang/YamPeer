use image_editor_core::{
    AdjustmentKind, EditorCommand, KeyModifiers, NavigationDirection, RawKeyEvent, RuntimePlatform,
    ShortcutKey, ShortcutResolver, shortcut_label,
};

fn event(key: ShortcutKey, modifiers: KeyModifiers) -> RawKeyEvent {
    RawKeyEvent::press(key, modifiers)
}

#[test]
fn platform_tables_resolve_the_same_semantic_history_and_adjustment_commands() {
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

    for (macos_event, linux_event, expected) in cases {
        assert_eq!(
            ShortcutResolver::new(RuntimePlatform::MacOs).resolve(macos_event),
            Some(expected.clone())
        );
        assert_eq!(
            ShortcutResolver::new(RuntimePlatform::Linux).resolve(linux_event),
            Some(expected)
        );
    }
}

#[test]
fn navigation_edit_and_adjustment_focus_inputs_resolve_once_without_modifiers() {
    let resolver = ShortcutResolver::new(RuntimePlatform::MacOs);
    let plain = KeyModifiers::default();
    let shifted = plain.with_shift();
    let cases = [
        (
            event(ShortcutKey::ArrowLeft, plain),
            EditorCommand::Navigate {
                direction: NavigationDirection::Left,
            },
        ),
        (
            event(ShortcutKey::ArrowRight, plain),
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
fn release_repeat_and_text_consumed_events_are_ignored() {
    let resolver = ShortcutResolver::new(RuntimePlatform::Linux);
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

    assert_eq!(
        resolver.resolve(event(ShortcutKey::Character('z'), KeyModifiers::command())),
        None,
        "Linux must not treat macOS Command as Control"
    );
}

#[test]
fn shortcut_labels_use_runtime_correct_modifier_names() {
    assert_eq!(
        shortcut_label(RuntimePlatform::MacOs, &EditorCommand::Undo),
        Some("Command+Z".to_owned())
    );
    assert_eq!(
        shortcut_label(RuntimePlatform::Linux, &EditorCommand::Redo),
        Some("Control+Shift+Z".to_owned())
    );
    assert_eq!(
        shortcut_label(RuntimePlatform::MacOs, &EditorCommand::IncreaseAdjustment),
        Some("Option+Up".to_owned())
    );
    assert_eq!(
        shortcut_label(RuntimePlatform::Linux, &EditorCommand::DecreaseAdjustment),
        Some("Alt+Down".to_owned())
    );
    assert_eq!(
        shortcut_label(RuntimePlatform::Linux, &EditorCommand::FlipVertical),
        Some("Shift+F".to_owned())
    );
    assert_eq!(
        shortcut_label(RuntimePlatform::MacOs, &EditorCommand::CancelCrop),
        None
    );
}
