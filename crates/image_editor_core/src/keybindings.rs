//! TOML keybinding configuration parsing, validation, and canonical formatting.
//!
//! This module is deliberately independent of filesystem and UI adapters. Callers
//! supply configuration text and a source, then use the validated declarations
//! and diagnostics to perform later platform-specific layer resolution.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::{
    KeyModifiers, KeybindingAction, KeybindingDiagnostic, KeybindingDiagnosticKind,
    KeybindingGesture, KeybindingSource, RuntimePlatform, ShortcutKey,
};

type ActionBindings = BTreeMap<KeybindingAction, Vec<KeybindingGesture>>;

/// A validated partial keybinding configuration.
///
/// Each table contains only declarations that have one or more valid gestures.
/// The platform tables selectively override the corresponding global actions
/// during the later layer-resolution stage.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ValidatedKeybindingConfiguration {
    bindings: ActionBindings,
    macos_bindings: ActionBindings,
    linux_bindings: ActionBindings,
}

impl ValidatedKeybindingConfiguration {
    /// Creates a normalized declaration from already-validated binding tables.
    pub fn new(
        mut bindings: ActionBindings,
        mut macos_bindings: ActionBindings,
        mut linux_bindings: ActionBindings,
    ) -> Self {
        normalize_bindings(&mut bindings);
        normalize_bindings(&mut macos_bindings);
        normalize_bindings(&mut linux_bindings);
        Self {
            bindings,
            macos_bindings,
            linux_bindings,
        }
    }

    pub fn bindings(&self) -> &ActionBindings {
        &self.bindings
    }

    pub fn macos_bindings(&self) -> &ActionBindings {
        &self.macos_bindings
    }

    pub fn linux_bindings(&self) -> &ActionBindings {
        &self.linux_bindings
    }

    pub fn is_empty(&self) -> bool {
        self.bindings.is_empty() && self.macos_bindings.is_empty() && self.linux_bindings.is_empty()
    }
}

/// A partial declaration is validated at parse time; it may omit any action.
pub type PartialKeybindingConfiguration = ValidatedKeybindingConfiguration;

/// The result of parsing one configuration source.
///
/// Invalid action declarations are omitted while valid, unrelated declarations
/// remain available for future partial-layer resolution. A syntactically invalid
/// TOML document has no configuration value.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct KeybindingParseResult {
    pub configuration: Option<PartialKeybindingConfiguration>,
    pub diagnostics: Vec<KeybindingDiagnostic>,
}

impl KeybindingParseResult {
    pub fn is_valid(&self) -> bool {
        self.configuration.is_some() && self.diagnostics.is_empty()
    }
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawKeybindingConfiguration {
    #[serde(default)]
    bindings: BTreeMap<String, Vec<String>>,
    #[serde(default)]
    macos: RawPlatformBindings,
    #[serde(default)]
    linux: RawPlatformBindings,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawPlatformBindings {
    #[serde(default)]
    bindings: BTreeMap<String, Vec<String>>,
}

#[derive(Serialize)]
struct FormattedKeybindingConfiguration {
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    bindings: BTreeMap<String, Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    macos: Option<FormattedPlatformBindings>,
    #[serde(skip_serializing_if = "Option::is_none")]
    linux: Option<FormattedPlatformBindings>,
}

#[derive(Serialize)]
struct FormattedPlatformBindings {
    bindings: BTreeMap<String, Vec<String>>,
}

#[derive(Clone, Copy)]
enum BindingScope {
    Global,
    MacOs,
    Linux,
}

/// Parses `[bindings]`, `[macos.bindings]`, and `[linux.bindings]` from TOML.
///
/// The parser accepts case-insensitive supported key and modifier aliases,
/// canonicalizes them into domain gestures, and rejects each invalid action
/// declaration with a diagnostic that retains the supplied source. Global
/// bindings must be platform-neutral; platform modifier keys belong in their
/// corresponding platform table.
pub fn parse_keybinding_configuration(
    text: &str,
    source: KeybindingSource,
) -> KeybindingParseResult {
    let raw = match toml::from_str::<RawKeybindingConfiguration>(text) {
        Ok(raw) => raw,
        Err(_) => {
            return KeybindingParseResult {
                configuration: None,
                diagnostics: vec![KeybindingDiagnostic::new(
                    source,
                    None,
                    None,
                    KeybindingDiagnosticKind::InvalidToml,
                    "keybinding configuration is not valid TOML",
                )],
            };
        }
    };

    let mut diagnostics = Vec::new();
    let bindings = parse_table(
        raw.bindings,
        &source,
        BindingScope::Global,
        &mut diagnostics,
    );
    let macos_bindings = parse_table(
        raw.macos.bindings,
        &source,
        BindingScope::MacOs,
        &mut diagnostics,
    );
    let linux_bindings = parse_table(
        raw.linux.bindings,
        &source,
        BindingScope::Linux,
        &mut diagnostics,
    );

    KeybindingParseResult {
        configuration: Some(ValidatedKeybindingConfiguration::new(
            bindings,
            macos_bindings,
            linux_bindings,
        )),
        diagnostics,
    }
}

/// Formats a validated declaration as deterministic, canonical TOML.
///
/// Action names and gestures are sorted, modifier order is normalized, and the
/// result is accepted by [`parse_keybinding_configuration`].
pub fn format_keybinding_configuration(configuration: &ValidatedKeybindingConfiguration) -> String {
    let macos = format_table(configuration.macos_bindings());
    let linux = format_table(configuration.linux_bindings());
    let output = FormattedKeybindingConfiguration {
        bindings: format_table(configuration.bindings()),
        macos: (!macos.is_empty()).then_some(FormattedPlatformBindings { bindings: macos }),
        linux: (!linux.is_empty()).then_some(FormattedPlatformBindings { bindings: linux }),
    };

    toml::to_string_pretty(&output)
        .expect("validated keybinding declarations always serialize to TOML")
}

fn parse_table(
    raw_bindings: BTreeMap<String, Vec<String>>,
    source: &KeybindingSource,
    scope: BindingScope,
    diagnostics: &mut Vec<KeybindingDiagnostic>,
) -> ActionBindings {
    let mut parsed_bindings = BTreeMap::new();

    for (action_name, raw_gestures) in raw_bindings {
        let Some(action) = KeybindingAction::from_stable_name(&action_name) else {
            diagnostics.push(KeybindingDiagnostic::new(
                source.clone(),
                None,
                None,
                KeybindingDiagnosticKind::UnknownAction,
                "keybinding declaration names an unknown action",
            ));
            continue;
        };

        if raw_gestures.is_empty() {
            diagnostics.push(KeybindingDiagnostic::new(
                source.clone(),
                Some(action),
                None,
                KeybindingDiagnosticKind::InvalidToml,
                "a keybinding action must declare one or more gestures",
            ));
            continue;
        }

        let mut gestures = Vec::with_capacity(raw_gestures.len());
        let mut declaration_is_valid = true;
        for raw_gesture in raw_gestures {
            match parse_gesture(&raw_gesture, scope) {
                Ok(gesture) => gestures.push(gesture),
                Err(error) => {
                    declaration_is_valid = false;
                    diagnostics.push(KeybindingDiagnostic::new(
                        source.clone(),
                        Some(action),
                        Some(raw_gesture),
                        error.category(),
                        error.message(),
                    ));
                }
            }
        }

        if declaration_is_valid {
            gestures.sort_unstable();
            gestures.dedup();
            parsed_bindings.insert(action, gestures);
        }
    }

    parsed_bindings
}

fn parse_gesture(
    raw_gesture: &str,
    scope: BindingScope,
) -> Result<KeybindingGesture, GestureParseError> {
    let raw_gesture = raw_gesture.trim();
    if raw_gesture.is_empty() {
        return Err(GestureParseError::UnknownKey);
    }

    let mut modifiers = KeyModifiers::default();
    let key_name = if raw_gesture == "+" {
        "+"
    } else {
        let mut parts = raw_gesture.split('+').map(str::trim).collect::<Vec<_>>();
        if parts.len() == 1 {
            parts[0]
        } else {
            let Some(key_name) = parts.pop() else {
                return Err(GestureParseError::UnknownKey);
            };
            if key_name.is_empty() || parts.iter().any(|part| part.is_empty()) {
                return Err(GestureParseError::UnknownKey);
            }
            for modifier_name in parts {
                apply_modifier(modifier_name, &mut modifiers)?;
            }
            key_name
        }
    };

    validate_modifiers(modifiers, scope)?;
    let key = parse_key(key_name)?;
    Ok(KeybindingGesture::new(key, modifiers))
}

fn apply_modifier(name: &str, modifiers: &mut KeyModifiers) -> Result<(), GestureParseError> {
    match name.to_ascii_lowercase().as_str() {
        "command" | "cmd" => modifiers.command = true,
        "control" | "ctrl" => modifiers.control = true,
        "option" | "opt" => modifiers.option = true,
        "alt" => modifiers.alt = true,
        "shift" => modifiers.shift = true,
        _ => return Err(GestureParseError::UnknownKey),
    }
    Ok(())
}

fn validate_modifiers(
    modifiers: KeyModifiers,
    scope: BindingScope,
) -> Result<(), GestureParseError> {
    let is_legal = match scope {
        BindingScope::Global => {
            !modifiers.command && !modifiers.control && !modifiers.option && !modifiers.alt
        }
        // Control+Command remains valid on macOS for its defined full-screen
        // binding, but bare Control is a Linux-only modifier in configuration.
        BindingScope::MacOs => !modifiers.alt && (!modifiers.control || modifiers.command),
        BindingScope::Linux => !modifiers.command && !modifiers.option,
    };

    is_legal
        .then_some(())
        .ok_or(GestureParseError::IllegalModifier)
}

fn parse_key(name: &str) -> Result<ShortcutKey, GestureParseError> {
    let normalized = name.trim().to_ascii_lowercase();
    let key = match normalized.as_str() {
        "up" | "arrowup" => ShortcutKey::ArrowUp,
        "down" | "arrowdown" => ShortcutKey::ArrowDown,
        "left" | "arrowleft" => ShortcutKey::ArrowLeft,
        "right" | "arrowright" => ShortcutKey::ArrowRight,
        "pageup" | "page_up" | "pgup" => ShortcutKey::PageUp,
        "pagedown" | "page_down" | "pgdown" => ShortcutKey::PageDown,
        "home" => ShortcutKey::Home,
        "end" => ShortcutKey::End,
        "enter" | "return" => ShortcutKey::Enter,
        "space" | "spacebar" => ShortcutKey::Space,
        "f11" => ShortcutKey::F11,
        _ if normalized.chars().count() == 1 && normalized.is_ascii() => {
            ShortcutKey::Character(normalized.chars().next().expect("one ASCII character"))
        }
        _ => return Err(GestureParseError::UnknownKey),
    };
    Ok(key)
}

#[derive(Clone, Copy)]
enum GestureParseError {
    UnknownKey,
    IllegalModifier,
}

impl GestureParseError {
    const fn category(self) -> KeybindingDiagnosticKind {
        match self {
            Self::UnknownKey => KeybindingDiagnosticKind::UnknownKey,
            Self::IllegalModifier => KeybindingDiagnosticKind::IllegalModifier,
        }
    }

    const fn message(self) -> &'static str {
        match self {
            Self::UnknownKey => "keybinding declaration names an unsupported key",
            Self::IllegalModifier => {
                "keybinding declaration uses a modifier that is illegal for this table"
            }
        }
    }
}

fn normalize_bindings(bindings: &mut ActionBindings) {
    bindings.retain(|_, gestures| {
        gestures.sort_unstable();
        gestures.dedup();
        !gestures.is_empty()
    });
}

fn format_table(bindings: &ActionBindings) -> BTreeMap<String, Vec<String>> {
    bindings
        .iter()
        .filter(|(_, gestures)| !gestures.is_empty())
        .map(|(action, gestures)| {
            (
                action.stable_name().to_owned(),
                gestures.iter().copied().map(format_gesture).collect(),
            )
        })
        .collect()
}

fn format_gesture(gesture: KeybindingGesture) -> String {
    let mut parts = Vec::with_capacity(6);
    if gesture.modifiers.control {
        parts.push("Control".to_owned());
    }
    if gesture.modifiers.command {
        parts.push("Command".to_owned());
    }
    if gesture.modifiers.option {
        parts.push("Option".to_owned());
    }
    if gesture.modifiers.alt {
        parts.push("Alt".to_owned());
    }
    if gesture.modifiers.shift {
        parts.push("Shift".to_owned());
    }
    parts.push(match gesture.key {
        ShortcutKey::Character(character) => character.to_ascii_uppercase().to_string(),
        ShortcutKey::ArrowUp => "Up".to_owned(),
        ShortcutKey::ArrowDown => "Down".to_owned(),
        ShortcutKey::ArrowLeft => "Left".to_owned(),
        ShortcutKey::ArrowRight => "Right".to_owned(),
        ShortcutKey::PageUp => "PageUp".to_owned(),
        ShortcutKey::PageDown => "PageDown".to_owned(),
        ShortcutKey::Home => "Home".to_owned(),
        ShortcutKey::End => "End".to_owned(),
        ShortcutKey::Enter => "Enter".to_owned(),
        ShortcutKey::Space => "Space".to_owned(),
        ShortcutKey::F11 => "F11".to_owned(),
    });
    parts.join("+")
}

impl KeybindingAction {
    /// Finds an action by its stable ASCII configuration identifier.
    pub fn from_stable_name(name: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|action| action.stable_name() == name)
    }

    pub const ALL: [Self; 26] = [
        Self::FitToWindow,
        Self::ZoomActual,
        Self::Zoom200,
        Self::ZoomIn,
        Self::ZoomOut,
        Self::PanLeft,
        Self::PanDown,
        Self::PanUp,
        Self::PanRight,
        Self::PreviousImage,
        Self::NextImage,
        Self::FirstImage,
        Self::LastImage,
        Self::ToggleFullscreen,
        Self::FlipHorizontal,
        Self::FlipVertical,
        Self::RotateClockwise90,
        Self::RotateCounterclockwise90,
        Self::EnterCrop,
        Self::FocusBrightness,
        Self::FocusContrast,
        Self::CommitAdjustment,
        Self::Undo,
        Self::Redo,
        Self::IncreaseAdjustment,
        Self::DecreaseAdjustment,
    ];
}

/// Returns the declarations applicable to one platform before layer merging.
pub fn declarations_for_platform(
    configuration: &ValidatedKeybindingConfiguration,
    platform: RuntimePlatform,
) -> ActionBindings {
    let mut declarations = configuration.bindings.clone();
    let platform_bindings = match platform {
        RuntimePlatform::MacOs => configuration.macos_bindings(),
        RuntimePlatform::Linux => configuration.linux_bindings(),
    };
    declarations.extend(platform_bindings.clone());
    declarations
}
