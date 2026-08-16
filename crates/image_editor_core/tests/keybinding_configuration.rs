use image_editor_core::{
    KeybindingAction, KeybindingDiagnosticKind, KeybindingSource, RuntimePlatform, ShortcutKey,
    format_keybinding_configuration, parse_keybinding_configuration,
};

#[test]
fn parser_normalizes_multi_binding_global_and_platform_declarations() {
    let parsed = parse_keybinding_configuration(
        r#"
            [bindings]
            next_image = ["right", "Page_Down", "spacebar"]
            flip_vertical = ["shift+f"]

            [macos.bindings]
            undo = ["cmd+z"]
            toggle_fullscreen = ["ctrl+cmd+f", "f11"]

            [linux.bindings]
            undo = ["ctrl+z"]
            increase_adjustment = ["alt+arrowup"]
        "#,
        KeybindingSource::BuiltIn,
    );

    assert!(parsed.diagnostics.is_empty());
    let configuration = parsed.configuration.expect("valid configuration");
    assert_eq!(
        configuration
            .bindings()
            .get(&KeybindingAction::NextImage)
            .expect("next image declaration")
            .len(),
        3
    );
    assert_eq!(
        configuration
            .macos_bindings()
            .get(&KeybindingAction::ToggleFullscreen)
            .expect("macOS fullscreen declaration")[0]
            .key,
        ShortcutKey::Character('f')
    );

    let canonical_toml = format_keybinding_configuration(&configuration);
    assert!(canonical_toml.contains("Control+Command+F"));
    assert!(canonical_toml.contains("PageDown"));
    let reparsed = parse_keybinding_configuration(&canonical_toml, KeybindingSource::BuiltIn);
    assert!(reparsed.diagnostics.is_empty());
    assert_eq!(reparsed.configuration, Some(configuration));
}

#[test]
fn parser_retains_valid_declarations_and_reports_source_aware_validation_errors() {
    let parsed = parse_keybinding_configuration(
        r#"
            [bindings]
            next_image = ["Right"]
            unknown_action = ["F"]
            zoom_in = ["NotAKey"]
            undo = ["Control+Z"]

            [linux.bindings]
            redo = ["Command+Z"]
        "#,
        KeybindingSource::BuiltIn,
    );

    let configuration = parsed.configuration.expect("syntactically valid TOML");
    assert!(
        configuration
            .bindings()
            .contains_key(&KeybindingAction::NextImage)
    );
    assert!(
        !configuration
            .bindings()
            .contains_key(&KeybindingAction::ZoomIn)
    );
    assert!(
        !configuration
            .bindings()
            .contains_key(&KeybindingAction::Undo)
    );
    assert!(
        !configuration
            .linux_bindings()
            .contains_key(&KeybindingAction::Redo)
    );

    let categories = parsed
        .diagnostics
        .iter()
        .map(|diagnostic| diagnostic.category)
        .collect::<Vec<_>>();
    assert!(categories.contains(&KeybindingDiagnosticKind::UnknownAction));
    assert!(categories.contains(&KeybindingDiagnosticKind::UnknownKey));
    assert_eq!(
        categories
            .iter()
            .filter(|category| **category == KeybindingDiagnosticKind::IllegalModifier)
            .count(),
        2
    );
    assert!(
        parsed
            .diagnostics
            .iter()
            .all(|diagnostic| diagnostic.source == KeybindingSource::BuiltIn)
    );
}

#[test]
fn parser_rejects_invalid_toml_without_exposing_document_contents() {
    let parsed = parse_keybinding_configuration(
        "[bindings\nnext_image = [\"Right\"]",
        KeybindingSource::BuiltIn,
    );

    assert!(parsed.configuration.is_none());
    assert_eq!(parsed.diagnostics.len(), 1);
    assert_eq!(
        parsed.diagnostics[0].category,
        KeybindingDiagnosticKind::InvalidToml
    );
    assert_eq!(
        parsed.diagnostics[0].safe_message,
        "keybinding configuration is not valid TOML"
    );
}

#[test]
fn layered_resolution_rejects_same_layer_duplicates_and_retains_non_conflicting_fallbacks() {
    use std::collections::BTreeMap;

    use image_editor_core::{
        KeyModifiers, KeybindingGesture, KeybindingLayerInput, ShortcutKey,
        ValidatedKeybindingConfiguration, resolve_keybindings,
    };

    let duplicate = KeybindingGesture::new(ShortcutKey::Character('h'), KeyModifiers::default());
    let next = KeybindingGesture::new(ShortcutKey::Character('n'), KeyModifiers::default());
    let zoom = KeybindingGesture::new(ShortcutKey::Character('z'), KeyModifiers::default());
    let mut cli_bindings = BTreeMap::new();
    cli_bindings.insert(KeybindingAction::PanLeft, vec![duplicate]);
    cli_bindings.insert(KeybindingAction::PanRight, vec![duplicate]);
    cli_bindings.insert(KeybindingAction::NextImage, vec![next]);
    let cli = KeybindingLayerInput::from_parse_result(
        KeybindingSource::ExplicitCli(
            image_editor_core::AbsolutePath::new("/config/cli.toml").unwrap(),
        ),
        image_editor_core::KeybindingParseResult {
            configuration: Some(ValidatedKeybindingConfiguration::new(
                cli_bindings,
                BTreeMap::new(),
                BTreeMap::new(),
            )),
            diagnostics: Vec::new(),
        },
    );

    let mut project_bindings = BTreeMap::new();
    project_bindings.insert(KeybindingAction::FitToWindow, vec![next, zoom]);
    let project = KeybindingLayerInput::from_parse_result(
        KeybindingSource::Project(
            image_editor_core::AbsolutePath::new("/project/.yampixr/keybindings.toml").unwrap(),
        ),
        image_editor_core::KeybindingParseResult {
            configuration: Some(ValidatedKeybindingConfiguration::new(
                project_bindings,
                BTreeMap::new(),
                BTreeMap::new(),
            )),
            diagnostics: Vec::new(),
        },
    );

    let resolution = resolve_keybindings(RuntimePlatform::Linux, &[cli, project]);
    assert_eq!(
        resolution.effective_map.action_for(next),
        Some(KeybindingAction::NextImage)
    );
    assert_eq!(
        resolution.effective_map.action_for(zoom),
        Some(KeybindingAction::FitToWindow),
        "a lower layer retains its non-conflicting gesture"
    );
    assert!(
        resolution
            .effective_map
            .gestures_for(KeybindingAction::PanLeft)
            .is_empty()
    );
    assert!(
        resolution
            .effective_map
            .gestures_for(KeybindingAction::PanRight)
            .is_empty()
    );
    assert!(resolution.diagnostics.iter().any(|diagnostic| {
        diagnostic.category == KeybindingDiagnosticKind::DuplicateGesture
            && diagnostic.action == Some(KeybindingAction::PanLeft)
    }));
    assert!(resolution.diagnostics.iter().any(|diagnostic| {
        diagnostic.category == KeybindingDiagnosticKind::BlockedByHigherPriority
            && diagnostic.action == Some(KeybindingAction::FitToWindow)
            && diagnostic.gesture.as_deref() == Some("N")
    }));
}

#[test]
fn platform_specific_tables_can_supply_their_platform_modifier_sets() {
    let parsed = parse_keybinding_configuration(
        r#"
            [macos.bindings]
            increase_adjustment = ["Option+Up"]

            [linux.bindings]
            increase_adjustment = ["Alt+Up"]
        "#,
        KeybindingSource::BuiltIn,
    );

    let configuration = parsed.configuration.expect("valid configuration");
    assert!(parsed.diagnostics.is_empty());
    assert!(
        image_editor_core::keybindings::declarations_for_platform(
            &configuration,
            RuntimePlatform::MacOs
        )
        .get(&KeybindingAction::IncreaseAdjustment)
        .expect("macOS declaration")[0]
            .modifiers
            .option
    );
    assert!(
        image_editor_core::keybindings::declarations_for_platform(
            &configuration,
            RuntimePlatform::Linux
        )
        .get(&KeybindingAction::IncreaseAdjustment)
        .expect("Linux declaration")[0]
            .modifiers
            .alt
    );
}
