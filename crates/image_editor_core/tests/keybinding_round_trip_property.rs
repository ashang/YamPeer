use std::collections::BTreeMap;

use image_editor_core::{
    KeyModifiers, KeybindingAction, KeybindingGesture, KeybindingSource, ShortcutKey,
    ValidatedKeybindingConfiguration, format_keybinding_configuration,
    parse_keybinding_configuration,
};
use proptest::prelude::*;

fn character_gesture(character: char, modifiers: KeyModifiers) -> KeybindingGesture {
    KeybindingGesture::new(ShortcutKey::Character(character), modifiers)
}

fn command_modifiers(shift: bool) -> KeyModifiers {
    let mut modifiers = KeyModifiers::command();
    modifiers.shift = shift;
    modifiers
}

fn control_modifiers(shift: bool) -> KeyModifiers {
    let mut modifiers = KeyModifiers::control();
    modifiers.shift = shift;
    modifiers
}

fn option_modifiers(shift: bool) -> KeyModifiers {
    let mut modifiers = KeyModifiers::option();
    modifiers.shift = shift;
    modifiers
}

fn alt_modifiers(shift: bool) -> KeyModifiers {
    let mut modifiers = KeyModifiers::alt();
    modifiers.shift = shift;
    modifiers
}

fn configuration_from_flags(flags: [bool; 8]) -> ValidatedKeybindingConfiguration {
    let mut bindings = BTreeMap::new();
    bindings.insert(
        KeybindingAction::FitToWindow,
        vec![
            character_gesture('0', KeyModifiers::default()),
            character_gesture(
                'a',
                KeyModifiers {
                    shift: flags[0],
                    ..KeyModifiers::default()
                },
            ),
        ],
    );
    bindings.insert(
        KeybindingAction::NextImage,
        vec![
            KeybindingGesture::new(ShortcutKey::ArrowRight, KeyModifiers::default()),
            KeybindingGesture::new(
                ShortcutKey::PageDown,
                KeyModifiers {
                    shift: flags[1],
                    ..KeyModifiers::default()
                },
            ),
        ],
    );

    let mut macos_bindings = BTreeMap::new();
    macos_bindings.insert(
        KeybindingAction::Undo,
        vec![
            character_gesture('z', command_modifiers(flags[2])),
            character_gesture('y', command_modifiers(flags[3])),
        ],
    );
    macos_bindings.insert(
        KeybindingAction::IncreaseAdjustment,
        vec![
            KeybindingGesture::new(ShortcutKey::ArrowUp, option_modifiers(flags[4])),
            KeybindingGesture::new(ShortcutKey::ArrowDown, option_modifiers(flags[5])),
        ],
    );

    let mut linux_bindings = BTreeMap::new();
    linux_bindings.insert(
        KeybindingAction::Redo,
        vec![
            character_gesture('z', control_modifiers(flags[6])),
            character_gesture('y', control_modifiers(flags[7])),
        ],
    );
    linux_bindings.insert(
        KeybindingAction::DecreaseAdjustment,
        vec![
            KeybindingGesture::new(ShortcutKey::ArrowUp, alt_modifiers(false)),
            KeybindingGesture::new(ShortcutKey::ArrowDown, alt_modifiers(true)),
        ],
    );

    ValidatedKeybindingConfiguration::new(bindings, macos_bindings, linux_bindings)
}

fn canonical_gesture(gesture: KeybindingGesture) -> String {
    let mut parts = Vec::with_capacity(6);
    if gesture.modifiers.control {
        parts.push("Control");
    }
    if gesture.modifiers.command {
        parts.push("Command");
    }
    if gesture.modifiers.option {
        parts.push("Option");
    }
    if gesture.modifiers.alt {
        parts.push("Alt");
    }
    if gesture.modifiers.shift {
        parts.push("Shift");
    }
    parts.push(match gesture.key {
        ShortcutKey::Character(character) => {
            return format!(
                "{}{}",
                parts.join("+"),
                if parts.is_empty() {
                    character.to_ascii_uppercase().to_string()
                } else {
                    format!("+{}", character.to_ascii_uppercase())
                }
            );
        }
        ShortcutKey::ArrowUp => "Up",
        ShortcutKey::ArrowDown => "Down",
        ShortcutKey::ArrowLeft => "Left",
        ShortcutKey::ArrowRight => "Right",
        ShortcutKey::PageUp => "PageUp",
        ShortcutKey::PageDown => "PageDown",
        ShortcutKey::Home => "Home",
        ShortcutKey::End => "End",
        ShortcutKey::Enter => "Enter",
        ShortcutKey::Space => "Space",
        ShortcutKey::F11 => "F11",
    });
    parts.join("+")
}

fn assert_canonical_table(
    document: &toml::Value,
    table_path: &[&str],
    expected: &BTreeMap<KeybindingAction, Vec<KeybindingGesture>>,
) {
    let mut table = document;
    for path_component in table_path {
        table = table
            .get(*path_component)
            .expect("formatted TOML contains every populated binding table");
    }
    let table = table
        .as_table()
        .expect("formatted binding declarations are TOML tables");

    assert_eq!(table.len(), expected.len());
    for (action, gestures) in expected {
        let actual = table
            .get(action.stable_name())
            .and_then(toml::Value::as_array)
            .expect("formatted TOML contains every declared action")
            .iter()
            .map(|value| {
                value
                    .as_str()
                    .expect("formatted gestures are TOML strings")
                    .to_owned()
            })
            .collect::<Vec<_>>();
        let canonical = gestures
            .iter()
            .copied()
            .map(canonical_gesture)
            .collect::<Vec<_>>();
        assert_eq!(actual, canonical);
    }
}

// Feature: macos-image-editor, Property 12: TOML bindings round-trip and preserve aliases
// Validates: Requirements 12.2
proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    #[test]
    fn formatted_validated_bindings_round_trip_with_canonical_gesture_order(
        flags in prop::array::uniform8(any::<bool>()),
    ) {
        let configuration = configuration_from_flags(flags);
        let formatted = format_keybinding_configuration(&configuration);
        let document = formatted
            .parse::<toml::Value>()
            .expect("formatter emits valid TOML");

        assert_canonical_table(&document, &["bindings"], configuration.bindings());
        assert_canonical_table(
            &document,
            &["macos", "bindings"],
            configuration.macos_bindings(),
        );
        assert_canonical_table(
            &document,
            &["linux", "bindings"],
            configuration.linux_bindings(),
        );

        let reparsed = parse_keybinding_configuration(&formatted, KeybindingSource::BuiltIn);
        prop_assert!(reparsed.diagnostics.is_empty());
        prop_assert_eq!(reparsed.configuration.as_ref(), Some(&configuration));
        prop_assert_eq!(
            format_keybinding_configuration(
                reparsed.configuration.as_ref().expect("valid formatter output reparses")
            ),
            formatted,
        );
    }
}
