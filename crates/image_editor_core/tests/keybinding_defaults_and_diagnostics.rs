use image_editor_core::{
    AbsolutePath, KeyModifiers, KeybindingAction, KeybindingDiagnosticKind, KeybindingGesture,
    KeybindingSource, RuntimePlatform, ShortcutKey, built_in_keybinding_map,
    parse_keybinding_configuration,
};

fn gesture(key: ShortcutKey, modifiers: KeyModifiers) -> KeybindingGesture {
    KeybindingGesture::new(key, modifiers)
}

#[test]
fn built_in_defaults_match_the_complete_configurable_view_and_editor_contract() {
    let plain = KeyModifiers::default();
    let shift = plain.with_shift();

    for (platform, primary, adjustment, fullscreen) in [
        (
            RuntimePlatform::MacOs,
            KeyModifiers::command(),
            KeyModifiers::option(),
            vec![
                gesture(
                    ShortcutKey::Character('f'),
                    KeyModifiers {
                        command: true,
                        control: true,
                        option: false,
                        alt: false,
                        shift: false,
                    },
                ),
                gesture(ShortcutKey::F11, plain),
            ],
        ),
        (
            RuntimePlatform::Linux,
            KeyModifiers::control(),
            KeyModifiers::alt(),
            vec![gesture(ShortcutKey::F11, plain)],
        ),
    ] {
        let bindings = built_in_keybinding_map(platform);
        let expected = [
            (
                KeybindingAction::FitToWindow,
                vec![gesture(ShortcutKey::Character('0'), plain)],
            ),
            (
                KeybindingAction::ZoomActual,
                vec![gesture(ShortcutKey::Character('1'), plain)],
            ),
            (
                KeybindingAction::Zoom200,
                vec![gesture(ShortcutKey::Character('2'), plain)],
            ),
            (
                KeybindingAction::ZoomIn,
                vec![
                    gesture(ShortcutKey::Character('+'), plain),
                    gesture(ShortcutKey::Character('='), plain),
                ],
            ),
            (
                KeybindingAction::ZoomOut,
                vec![gesture(ShortcutKey::Character('-'), plain)],
            ),
            (
                KeybindingAction::PanLeft,
                vec![gesture(ShortcutKey::Character('h'), plain)],
            ),
            (
                KeybindingAction::PanDown,
                vec![gesture(ShortcutKey::Character('j'), plain)],
            ),
            (
                KeybindingAction::PanUp,
                vec![gesture(ShortcutKey::Character('k'), plain)],
            ),
            (
                KeybindingAction::PanRight,
                vec![gesture(ShortcutKey::Character('l'), plain)],
            ),
            (
                KeybindingAction::PreviousImage,
                vec![
                    gesture(ShortcutKey::ArrowLeft, plain),
                    gesture(ShortcutKey::ArrowUp, plain),
                    gesture(ShortcutKey::PageUp, plain),
                ],
            ),
            (
                KeybindingAction::NextImage,
                vec![
                    gesture(ShortcutKey::ArrowRight, plain),
                    gesture(ShortcutKey::ArrowDown, plain),
                    gesture(ShortcutKey::PageDown, plain),
                    gesture(ShortcutKey::Space, plain),
                ],
            ),
            (
                KeybindingAction::FirstImage,
                vec![gesture(ShortcutKey::Home, plain)],
            ),
            (
                KeybindingAction::LastImage,
                vec![gesture(ShortcutKey::End, plain)],
            ),
            (
                KeybindingAction::FlipHorizontal,
                vec![gesture(ShortcutKey::Character('f'), plain)],
            ),
            (
                KeybindingAction::FlipVertical,
                vec![gesture(ShortcutKey::Character('f'), shift)],
            ),
            (
                KeybindingAction::RotateClockwise90,
                vec![gesture(ShortcutKey::Character('r'), plain)],
            ),
            (
                KeybindingAction::RotateCounterclockwise90,
                vec![gesture(ShortcutKey::Character('r'), shift)],
            ),
            (
                KeybindingAction::EnterCrop,
                vec![gesture(ShortcutKey::Character('c'), plain)],
            ),
            (
                KeybindingAction::FocusBrightness,
                vec![gesture(ShortcutKey::Character('b'), plain)],
            ),
            (
                KeybindingAction::FocusContrast,
                vec![gesture(ShortcutKey::Character('d'), plain)],
            ),
            (
                KeybindingAction::CommitAdjustment,
                vec![gesture(ShortcutKey::Enter, plain)],
            ),
            (
                KeybindingAction::Undo,
                vec![gesture(ShortcutKey::Character('z'), primary)],
            ),
            (
                KeybindingAction::Redo,
                vec![gesture(ShortcutKey::Character('z'), primary.with_shift())],
            ),
            (
                KeybindingAction::IncreaseAdjustment,
                vec![gesture(ShortcutKey::ArrowUp, adjustment)],
            ),
            (
                KeybindingAction::DecreaseAdjustment,
                vec![gesture(ShortcutKey::ArrowDown, adjustment)],
            ),
        ];

        assert_eq!(
            bindings.by_action().len(),
            expected.len() + 1,
            "{platform:?}"
        );
        assert_eq!(
            bindings.gestures_for(KeybindingAction::ToggleFullscreen),
            fullscreen,
            "{platform:?}"
        );
        for (action, mut gestures) in expected {
            gestures.sort_unstable();
            assert_eq!(
                bindings.gestures_for(action),
                gestures,
                "{platform:?}: {action:?}"
            );
        }
    }
}

#[test]
fn malformed_unknown_and_illegal_toml_declarations_report_safe_source_aware_diagnostics() {
    let source = KeybindingSource::ExplicitCli(
        AbsolutePath::new("/config/keybindings.toml").expect("fixture path is absolute"),
    );
    let malformed = parse_keybinding_configuration("[bindings", source.clone());
    assert!(malformed.configuration.is_none());
    assert_eq!(malformed.diagnostics[0].source, source);
    assert_eq!(
        malformed.diagnostics[0].category,
        KeybindingDiagnosticKind::InvalidToml
    );

    let parsed = parse_keybinding_configuration(
        r#"
            [bindings]
            next_image = ["Right"]
            unexpected_action = ["F"]
            zoom_in = ["NotAKey"]

            [linux.bindings]
            undo = ["Command+Z"]
        "#,
        source.clone(),
    );
    let configuration = parsed.configuration.expect("the surrounding TOML is valid");
    assert!(
        configuration
            .bindings()
            .contains_key(&KeybindingAction::NextImage)
    );
    assert!(
        parsed
            .diagnostics
            .iter()
            .all(|diagnostic| diagnostic.source == source)
    );
    for category in [
        KeybindingDiagnosticKind::UnknownAction,
        KeybindingDiagnosticKind::UnknownKey,
        KeybindingDiagnosticKind::IllegalModifier,
    ] {
        assert!(
            parsed
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.category == category)
        );
    }
}
