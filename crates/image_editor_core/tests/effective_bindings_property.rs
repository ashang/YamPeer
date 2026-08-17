use image_editor_core::{
    EditorCommand, KeyModifiers, KeybindingDiagnosticKind, KeybindingGesture, KeybindingLayerInput,
    KeybindingSource, RawKeyEvent, RuntimePlatform, ShortcutKey, ShortcutResolver,
    parse_keybinding_configuration, resolve_keybindings,
};
use proptest::prelude::*;

#[derive(Clone, Copy, Debug)]
enum InvalidDeclaration {
    UnknownAction,
    UnknownKey,
    IllegalModifier,
}

fn invalid_declaration_from_id(id: u8) -> InvalidDeclaration {
    match id {
        0 => InvalidDeclaration::UnknownAction,
        1 => InvalidDeclaration::UnknownKey,
        2 => InvalidDeclaration::IllegalModifier,
        _ => unreachable!("the generator restricts invalid declaration identifiers"),
    }
}

fn custom_gesture(key_id: u8, uppercase: bool) -> String {
    let character = ['v', 'w', 'x', 'y'][usize::from(key_id)];
    if uppercase {
        character.to_ascii_uppercase().to_string()
    } else {
        character.to_string()
    }
}

fn event_for_gesture(key_id: u8, uppercase: bool) -> RawKeyEvent {
    RawKeyEvent::press(
        ShortcutKey::Character(
            custom_gesture(key_id, uppercase)
                .chars()
                .next()
                .expect("generated gestures contain one character"),
        ),
        KeyModifiers::default(),
    )
}

fn event_variant(mut event: RawKeyEvent, variant: u8) -> RawKeyEvent {
    match variant {
        0 => event,
        1 => {
            event.pressed = false;
            event
        }
        2 => {
            event.repeat = true;
            event
        }
        3 => {
            event.consumed_by_text_control = true;
            event
        }
        _ => unreachable!("the generator restricts raw event variants"),
    }
}

fn invalid_declaration_toml(
    declaration: InvalidDeclaration,
) -> (&'static str, KeybindingDiagnosticKind) {
    match declaration {
        InvalidDeclaration::UnknownAction => (
            "unknown_action = [\"Z\"]",
            KeybindingDiagnosticKind::UnknownAction,
        ),
        InvalidDeclaration::UnknownKey => (
            "zoom_200 = [\"NotAKey\"]",
            KeybindingDiagnosticKind::UnknownKey,
        ),
        InvalidDeclaration::IllegalModifier => (
            "zoom_200 = [\"Control+Y\"]",
            KeybindingDiagnosticKind::IllegalModifier,
        ),
    }
}

fn rejected_event(declaration: InvalidDeclaration) -> RawKeyEvent {
    match declaration {
        InvalidDeclaration::UnknownAction => {
            RawKeyEvent::press(ShortcutKey::Character('z'), KeyModifiers::default())
        }
        InvalidDeclaration::UnknownKey => {
            RawKeyEvent::press(ShortcutKey::Character('?'), KeyModifiers::default())
        }
        InvalidDeclaration::IllegalModifier => {
            RawKeyEvent::press(ShortcutKey::Character('y'), KeyModifiers::control())
        }
    }
}

// Feature: macos-image-editor, Property 14: Effective bindings are exclusive and text-safe
// Validates: Requirements 12.4, 12.5, 12.12
proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    #[test]
    fn effective_bindings_are_exclusive_and_ignore_rejected_or_text_consumed_events(
        platform in prop_oneof![Just(RuntimePlatform::MacOs), Just(RuntimePlatform::Linux)],
        key_id in 0_u8..4,
        duplicate_normalized_gesture in any::<bool>(),
        invalid_declaration_id in 0_u8..3,
        raw_event_variant in 0_u8..4,
        uppercase_primary_key in any::<bool>(),
        uppercase_duplicate_key in any::<bool>(),
    ) {
        let primary = custom_gesture(key_id, uppercase_primary_key);
        let duplicate = custom_gesture(key_id, uppercase_duplicate_key);
        let duplicate_declaration = duplicate_normalized_gesture
            .then(|| format!("zoom_actual = [\"{duplicate}\"]"))
            .unwrap_or_default();
        let invalid = invalid_declaration_from_id(invalid_declaration_id);
        let (invalid_toml, expected_diagnostic) = invalid_declaration_toml(invalid);
        let source = KeybindingSource::BuiltIn;
        let document = format!(
            "[bindings]\nfit_to_window = [\"{primary}\"]\n{duplicate_declaration}\n{invalid_toml}\n"
        );
        let parsed = parse_keybinding_configuration(&document, source.clone());
        let layer = KeybindingLayerInput::from_parse_result(source, parsed);
        let resolution = resolve_keybindings(
            platform,
            &[layer, KeybindingLayerInput::built_in(platform)],
        );
        let resolver = ShortcutResolver::new(resolution.effective_map.clone());
        let primary_gesture = KeybindingGesture::new(
            event_for_gesture(key_id, false).key,
            KeyModifiers::default(),
        );
        let primary_event = event_variant(
            event_for_gesture(key_id, uppercase_primary_key),
            raw_event_variant,
        );
        let routed = resolver.resolve(primary_event);

        prop_assert!(
            resolution
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.category == expected_diagnostic),
            "the generated invalid declaration must be diagnosed"
        );
        prop_assert!(
            resolution
                .effective_map
                .by_gesture()
                .iter()
                .all(|(gesture, action)| resolution.effective_map.action_for(*gesture) == Some(*action)),
            "every normalized gesture has exactly its single indexed action"
        );
        prop_assert!(usize::from(routed.is_some()) <= 1, "one raw event routes to at most one command");
        prop_assert_eq!(
            resolver.resolve(rejected_event(invalid)),
            None,
            "a rejected declaration never produces a command"
        );

        if duplicate_normalized_gesture {
            prop_assert_eq!(
                resolution.effective_map.action_for(primary_gesture),
                None,
                "all actions sharing a normalized gesture in one layer are rejected"
            );
            prop_assert!(
                resolution.diagnostics.iter().any(|diagnostic| {
                    diagnostic.category == KeybindingDiagnosticKind::DuplicateGesture
                        && diagnostic.gesture.as_deref() == Some(primary.to_ascii_uppercase().as_str())
                }),
                "the normalized duplicate gesture is diagnosed"
            );
            prop_assert_eq!(routed, None);
        } else {
            let expected = (raw_event_variant == 0).then_some(EditorCommand::SetFitToWindow);
            prop_assert_eq!(routed, expected);
        }

        let text_consumed = RawKeyEvent {
            consumed_by_text_control: true,
            ..event_for_gesture(key_id, uppercase_primary_key)
        };
        prop_assert_eq!(
            resolver.resolve(text_consumed),
            None,
            "a text-consumed event never invokes its configured binding"
        );
    }
}
