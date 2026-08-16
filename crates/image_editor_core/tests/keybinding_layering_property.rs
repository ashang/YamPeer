use std::collections::{BTreeMap, BTreeSet};

use image_editor_core::{
    AbsolutePath, KeyModifiers, KeybindingAction, KeybindingDiagnosticKind, KeybindingGesture,
    KeybindingLayerInput, KeybindingSource, RuntimePlatform, ShortcutKey,
    ValidatedKeybindingConfiguration, parse_keybinding_configuration, resolve_keybindings,
};
use proptest::prelude::*;

type Bindings = BTreeMap<KeybindingAction, Vec<KeybindingGesture>>;
type DiagnosticView = (
    KeybindingSource,
    KeybindingDiagnosticKind,
    Option<KeybindingAction>,
    Option<String>,
);

#[derive(Clone, Debug)]
enum GeneratedLayer {
    Valid(Vec<(u8, Vec<u8>)>),
    Unreadable,
    InvalidToml,
}

fn action_from_id(id: u8) -> KeybindingAction {
    match id {
        0 => KeybindingAction::FitToWindow,
        1 => KeybindingAction::ZoomActual,
        2 => KeybindingAction::Zoom200,
        3 => KeybindingAction::ZoomIn,
        4 => KeybindingAction::PanLeft,
        5 => KeybindingAction::NextImage,
        _ => unreachable!("the generator restricts action identifiers"),
    }
}

fn gesture_from_id(id: u8) -> KeybindingGesture {
    KeybindingGesture::new(
        ShortcutKey::Character(char::from(b'a' + id)),
        KeyModifiers::default(),
    )
}

fn gesture_label(gesture: KeybindingGesture) -> String {
    match gesture.key {
        ShortcutKey::Character(character) => character.to_ascii_uppercase().to_string(),
        _ => unreachable!("the generator only creates character gestures"),
    }
}

fn layer_source(index: usize) -> KeybindingSource {
    let path = AbsolutePath::new(format!("/test/keybindings/layer-{index}.toml"))
        .expect("generated test path is absolute");
    match index % 3 {
        0 => KeybindingSource::ExplicitCli(path),
        1 => KeybindingSource::Project(path),
        _ => KeybindingSource::User(path),
    }
}

fn valid_layer() -> impl Strategy<Value = GeneratedLayer> {
    prop::collection::btree_set(0_u8..6, 0..=6).prop_flat_map(|actions| {
        let actions = actions.into_iter().collect::<Vec<_>>();
        prop::collection::vec(prop::collection::vec(0_u8..8, 1..3), actions.len()).prop_map(
            move |gesture_sets| {
                GeneratedLayer::Valid(
                    actions
                        .iter()
                        .copied()
                        .zip(gesture_sets)
                        .collect::<Vec<_>>(),
                )
            },
        )
    })
}

fn generated_layer() -> impl Strategy<Value = GeneratedLayer> {
    prop_oneof![
        6 => valid_layer(),
        1 => Just(GeneratedLayer::Unreadable),
        1 => Just(GeneratedLayer::InvalidToml),
    ]
}

fn bindings_from_declarations(declarations: &[(u8, Vec<u8>)], reverse_insertion: bool) -> Bindings {
    let mut bindings = BTreeMap::new();
    let declaration_iter: Box<dyn Iterator<Item = &(u8, Vec<u8>)>> = if reverse_insertion {
        Box::new(declarations.iter().rev())
    } else {
        Box::new(declarations.iter())
    };

    for (action_id, gesture_ids) in declaration_iter {
        let mut gestures = gesture_ids
            .iter()
            .copied()
            .map(gesture_from_id)
            .collect::<Vec<_>>();
        gestures.sort_unstable();
        gestures.dedup();
        bindings.insert(action_from_id(*action_id), gestures);
    }
    bindings
}

fn layer_input(
    layer: &GeneratedLayer,
    index: usize,
    reverse_insertion: bool,
) -> KeybindingLayerInput {
    let source = layer_source(index);
    match layer {
        GeneratedLayer::Valid(declarations) => KeybindingLayerInput::from_parse_result(
            source,
            image_editor_core::KeybindingParseResult {
                configuration: Some(ValidatedKeybindingConfiguration::new(
                    bindings_from_declarations(declarations, reverse_insertion),
                    BTreeMap::new(),
                    BTreeMap::new(),
                )),
                diagnostics: Vec::new(),
            },
        ),
        GeneratedLayer::Unreadable => KeybindingLayerInput::unreadable(source),
        GeneratedLayer::InvalidToml => KeybindingLayerInput::from_parse_result(
            source.clone(),
            parse_keybinding_configuration("[bindings", source),
        ),
    }
}

fn expected_resolution(layers: &[GeneratedLayer]) -> (Bindings, Vec<DiagnosticView>) {
    let mut accepted_bindings = BTreeMap::new();
    let mut accepted_gestures = BTreeMap::new();
    let mut replaced_actions = BTreeSet::new();
    let mut diagnostics = Vec::new();

    for (index, layer) in layers.iter().enumerate() {
        let source = layer_source(index);
        let declarations = match layer {
            GeneratedLayer::Unreadable => {
                diagnostics.push((source, KeybindingDiagnosticKind::ReadFailed, None, None));
                continue;
            }
            GeneratedLayer::InvalidToml => {
                diagnostics.push((source, KeybindingDiagnosticKind::InvalidToml, None, None));
                continue;
            }
            GeneratedLayer::Valid(declarations) => bindings_from_declarations(declarations, false),
        };

        let mut owners = BTreeMap::<KeybindingGesture, Vec<KeybindingAction>>::new();
        for (action, gestures) in &declarations {
            for gesture in gestures {
                owners.entry(*gesture).or_default().push(*action);
            }
        }

        let mut rejected_actions = BTreeSet::new();
        for (gesture, actions) in owners {
            if actions.len() < 2 {
                continue;
            }
            for action in actions {
                rejected_actions.insert(action);
                diagnostics.push((
                    source.clone(),
                    KeybindingDiagnosticKind::DuplicateGesture,
                    Some(action),
                    Some(gesture_label(gesture)),
                ));
            }
        }

        for (action, gestures) in declarations {
            if rejected_actions.contains(&action) || replaced_actions.contains(&action) {
                continue;
            }

            let retained = gestures
                .into_iter()
                .filter(|gesture| {
                    if accepted_gestures.contains_key(gesture) {
                        diagnostics.push((
                            source.clone(),
                            KeybindingDiagnosticKind::BlockedByHigherPriority,
                            Some(action),
                            Some(gesture_label(*gesture)),
                        ));
                        false
                    } else {
                        true
                    }
                })
                .collect::<Vec<_>>();

            if retained.is_empty() {
                continue;
            }

            for gesture in &retained {
                accepted_gestures.insert(*gesture, action);
            }
            accepted_bindings.insert(action, retained);
            replaced_actions.insert(action);
        }
    }

    (accepted_bindings, diagnostics)
}

fn diagnostic_view(diagnostics: &[image_editor_core::KeybindingDiagnostic]) -> Vec<DiagnosticView> {
    diagnostics
        .iter()
        .map(|diagnostic| {
            (
                diagnostic.source.clone(),
                diagnostic.category,
                diagnostic.action,
                diagnostic.gesture.clone(),
            )
        })
        .collect()
}

// Feature: macos-image-editor, Property 13: Layered partial overrides retain valid lower declarations
// Validates: Requirements 12.1, 12.3, 12.4, 12.5
proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    #[test]
    fn layered_partial_overrides_match_precedence_fallback_and_diagnostics(
        layers in prop::collection::vec(generated_layer(), 1..5),
    ) {
        let expected = expected_resolution(&layers);
        let forward = resolve_keybindings(
            RuntimePlatform::Linux,
            &layers
                .iter()
                .enumerate()
                .map(|(index, layer)| layer_input(layer, index, false))
                .collect::<Vec<_>>(),
        );
        let reverse_inserted = resolve_keybindings(
            RuntimePlatform::Linux,
            &layers
                .iter()
                .enumerate()
                .map(|(index, layer)| layer_input(layer, index, true))
                .collect::<Vec<_>>(),
        );

        prop_assert_eq!(forward.effective_map.by_action(), &expected.0);
        prop_assert_eq!(diagnostic_view(&forward.diagnostics), expected.1);
        prop_assert_eq!(reverse_inserted.effective_map, forward.effective_map);
        prop_assert_eq!(
            diagnostic_view(&reverse_inserted.diagnostics),
            diagnostic_view(&forward.diagnostics),
        );
    }
}
