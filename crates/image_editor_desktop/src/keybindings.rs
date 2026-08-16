//! Desktop startup composition for layered keybinding configuration.
//!
//! The platform crate discovers and reads paths; this adapter converts readable
//! TOML into pure-core layer inputs. It deliberately does not route key events:
//! the effective-map router is introduced by the subsequent shortcut task.

use image_editor_core::{
    AbsolutePath, KeybindingLayerInput, KeybindingResolution, RuntimePlatform,
    parse_keybinding_configuration, resolve_keybindings,
};
use image_editor_platform::{
    KeybindingPathEnvironment, KeybindingSourceRead, KeybindingSourceReader,
    LocalKeybindingSourceReader, discover_current_keybinding_sources, discover_keybinding_sources,
};

/// Resolves every supported source into one immutable effective keybinding map.
///
/// Sources are considered in explicit CLI, project, user, and built-in order.
/// Missing project/user files contribute no layer; unreadable sources contribute
/// only a safe `ReadFailed` diagnostic, then lower-priority layers continue.
pub fn resolve_startup_keybindings<R: KeybindingSourceReader>(
    platform: RuntimePlatform,
    explicit_cli: Option<AbsolutePath>,
    project_root: AbsolutePath,
    environment: &KeybindingPathEnvironment,
    reader: &R,
) -> KeybindingResolution {
    resolve_discovered_sources(
        platform,
        discover_keybinding_sources(platform, explicit_cli, project_root, environment),
        reader,
    )
}

/// Resolves the process-local CLI/project/user sources used by the desktop host.
///
/// If process path discovery itself is unavailable, the editor keeps the
/// deterministic built-in layer. File-specific failures remain source-aware
/// diagnostics through the shared reader and parser path.
pub fn resolve_current_startup_keybindings(
    platform: RuntimePlatform,
    explicit_cli: Option<AbsolutePath>,
) -> KeybindingResolution {
    let reader = LocalKeybindingSourceReader;
    match discover_current_keybinding_sources(platform, explicit_cli) {
        Ok(sources) => resolve_discovered_sources(platform, sources, &reader),
        Err(_) => resolve_keybindings(platform, &[KeybindingLayerInput::built_in(platform)]),
    }
}

fn resolve_discovered_sources<R: KeybindingSourceReader>(
    platform: RuntimePlatform,
    sources: Vec<image_editor_core::KeybindingSource>,
    reader: &R,
) -> KeybindingResolution {
    let layers = sources
        .into_iter()
        .filter_map(|source| match source {
            image_editor_core::KeybindingSource::BuiltIn => {
                Some(KeybindingLayerInput::built_in(platform))
            }
            source => match reader.read(&source) {
                KeybindingSourceRead::Absent => None,
                KeybindingSourceRead::Contents(contents) => {
                    Some(KeybindingLayerInput::from_parse_result(
                        source.clone(),
                        parse_keybinding_configuration(&contents, source),
                    ))
                }
                KeybindingSourceRead::Unreadable => Some(KeybindingLayerInput::unreadable(source)),
            },
        })
        .collect::<Vec<_>>();

    resolve_keybindings(platform, &layers)
}

#[cfg(test)]
mod tests {
    use image_editor_core::{
        AbsolutePath, KeyModifiers, KeybindingAction, KeybindingDiagnosticKind, KeybindingGesture,
        KeybindingSource, RuntimePlatform, ShortcutKey,
    };
    use image_editor_platform::{
        KeybindingPathEnvironment, KeybindingSourceRead, KeybindingSourceReader,
    };

    use super::resolve_startup_keybindings;

    #[derive(Clone, Copy)]
    struct Reader;

    impl KeybindingSourceReader for Reader {
        fn read(&self, source: &KeybindingSource) -> KeybindingSourceRead {
            match source {
                KeybindingSource::ExplicitCli(_) => {
                    KeybindingSourceRead::Contents("[bindings]\nnext_image = [\"H\"]\n".to_owned())
                }
                KeybindingSource::Project(_) => KeybindingSourceRead::Unreadable,
                KeybindingSource::User(_) => KeybindingSourceRead::Absent,
                KeybindingSource::BuiltIn => KeybindingSourceRead::Absent,
            }
        }
    }

    fn path(value: &str) -> AbsolutePath {
        AbsolutePath::new(value).expect("test path is absolute")
    }

    #[test]
    fn startup_resolution_keeps_valid_layers_when_optional_sources_are_absent_or_unreadable() {
        let resolution = resolve_startup_keybindings(
            RuntimePlatform::Linux,
            Some(path("/config/cli.toml")),
            path("/project"),
            &KeybindingPathEnvironment::new(Some_path(), Some(path("/xdg"))),
            &Reader,
        );
        let plain = KeyModifiers::default();

        assert_eq!(
            resolution
                .effective_map
                .action_for(KeybindingGesture::new(ShortcutKey::Character('h'), plain,)),
            Some(KeybindingAction::NextImage),
            "the explicit action replaces only its lower-priority counterpart"
        );
        assert_eq!(
            resolution
                .effective_map
                .action_for(KeybindingGesture::new(ShortcutKey::Character('0'), plain,)),
            Some(KeybindingAction::FitToWindow),
            "unrelated built-in declarations remain available"
        );
        assert!(resolution.diagnostics.iter().any(|diagnostic| {
            diagnostic.category == KeybindingDiagnosticKind::ReadFailed
                && matches!(diagnostic.source, KeybindingSource::Project(_))
        }));
        assert!(resolution.diagnostics.iter().any(|diagnostic| {
            diagnostic.category == KeybindingDiagnosticKind::BlockedByHigherPriority
                && diagnostic.action == Some(KeybindingAction::PanLeft)
        }));
    }

    fn Some_path() -> AbsolutePath {
        path("/home/tester")
    }
}
