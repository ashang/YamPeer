//! Desktop startup composition for layered keybinding configuration.
//!
//! The platform crate discovers and reads paths; this adapter converts readable
//! TOML into pure-core layer inputs. It deliberately does not route key events:
//! the effective-map router is introduced by the subsequent shortcut task.

use std::{ffi::OsString, path::Path};

use image_editor_core::{
    AbsolutePath, KeybindingLayerInput, KeybindingResolution, RuntimePlatform,
    parse_keybinding_configuration, resolve_keybindings,
};
use image_editor_platform::{
    KeybindingPathEnvironment, KeybindingSourceRead, KeybindingSourceReader,
    LocalKeybindingSourceReader, discover_current_keybinding_sources, discover_keybinding_sources,
};

/// Parses the process arguments that select the highest-priority keybinding layer.
///
/// Relative paths are resolved against the process working directory before
/// entering the pure-core path model. Unsupported or repeated arguments fail
/// startup instead of being silently ignored.
pub fn parse_explicit_keybindings_argument(
    arguments: impl IntoIterator<Item = OsString>,
    current_directory: &Path,
) -> Result<Option<AbsolutePath>, &'static str> {
    let mut arguments = arguments.into_iter();
    let mut explicit = None;

    while let Some(argument) = arguments.next() {
        if argument != "--keybindings" {
            return Err("unsupported command-line argument");
        }
        if explicit.is_some() {
            return Err("--keybindings may be supplied only once");
        }
        let value = arguments.next().ok_or("--keybindings requires a path")?;
        let path = Path::new(&value);
        let path = if path.is_absolute() {
            path.to_owned()
        } else {
            current_directory.join(path)
        };
        let path = path
            .to_str()
            .ok_or("--keybindings path must be valid UTF-8")?;
        explicit = Some(
            AbsolutePath::new(path.to_owned())
                .map_err(|_| "--keybindings path must resolve to an absolute path")?,
        );
    }

    Ok(explicit)
}

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
    use std::ffi::OsString;

    use image_editor_core::{
        AbsolutePath, KeyModifiers, KeybindingAction, KeybindingDiagnosticKind, KeybindingGesture,
        KeybindingSource, RuntimePlatform, ShortcutKey,
    };
    use image_editor_platform::{
        KeybindingPathEnvironment, KeybindingSourceRead, KeybindingSourceReader,
    };

    use super::{parse_explicit_keybindings_argument, resolve_startup_keybindings};

    #[test]
    fn explicit_keybindings_argument_accepts_absolute_and_working_directory_relative_paths() {
        let current_directory = std::path::Path::new("/workspace/project");

        assert_eq!(
            parse_explicit_keybindings_argument(Vec::<OsString>::new(), current_directory),
            Ok(None)
        );
        assert_eq!(
            parse_explicit_keybindings_argument(
                [
                    OsString::from("--keybindings"),
                    OsString::from("/config/keys.toml")
                ],
                current_directory,
            ),
            Ok(Some(path("/config/keys.toml")))
        );
        assert_eq!(
            parse_explicit_keybindings_argument(
                [
                    OsString::from("--keybindings"),
                    OsString::from(".yampixr/custom.toml")
                ],
                current_directory,
            ),
            Ok(Some(path("/workspace/project/.yampixr/custom.toml")))
        );
    }

    #[test]
    fn explicit_keybindings_argument_rejects_missing_repeated_and_unknown_arguments() {
        let current_directory = std::path::Path::new("/workspace/project");

        assert!(
            parse_explicit_keybindings_argument(
                [OsString::from("--keybindings")],
                current_directory,
            )
            .is_err()
        );
        assert!(
            parse_explicit_keybindings_argument(
                [
                    OsString::from("--keybindings"),
                    OsString::from("first.toml"),
                    OsString::from("--keybindings"),
                    OsString::from("second.toml"),
                ],
                current_directory,
            )
            .is_err()
        );
        assert!(
            parse_explicit_keybindings_argument([OsString::from("--unknown")], current_directory,)
                .is_err()
        );
    }

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
            &KeybindingPathEnvironment::new(some_path(), Some(path("/xdg"))),
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

    #[derive(Clone, Copy)]
    struct AllSourcesReader;

    impl KeybindingSourceReader for AllSourcesReader {
        fn read(&self, source: &KeybindingSource) -> KeybindingSourceRead {
            match source {
                KeybindingSource::ExplicitCli(_) => {
                    KeybindingSourceRead::Contents("[bindings]\nzoom_in = [\"Q\"]\n".to_owned())
                }
                KeybindingSource::Project(_) => KeybindingSourceRead::Contents(
                    "[bindings]\nzoom_in = [\"W\"]\nzoom_out = [\"E\"]\n".to_owned(),
                ),
                KeybindingSource::User(_) => KeybindingSourceRead::Contents(
                    "[bindings]\nzoom_in = [\"R\"]\npan_left = [\"Y\"]\n".to_owned(),
                ),
                KeybindingSource::BuiltIn => KeybindingSourceRead::Absent,
            }
        }
    }

    #[derive(Clone, Copy)]
    struct UnreadableCliReader;

    impl KeybindingSourceReader for UnreadableCliReader {
        fn read(&self, source: &KeybindingSource) -> KeybindingSourceRead {
            match source {
                KeybindingSource::ExplicitCli(_) => KeybindingSourceRead::Unreadable,
                KeybindingSource::Project(_)
                | KeybindingSource::User(_)
                | KeybindingSource::BuiltIn => KeybindingSourceRead::Absent,
            }
        }
    }

    #[test]
    fn startup_resolution_applies_every_present_source_in_descending_priority_order() {
        for platform in [RuntimePlatform::MacOs, RuntimePlatform::Linux] {
            let resolution = resolve_startup_keybindings(
                platform,
                Some(path("/arguments/keybindings.toml")),
                path("/project"),
                &KeybindingPathEnvironment::new(path("/home/tester"), Some(path("/xdg"))),
                &AllSourcesReader,
            );
            let plain = KeyModifiers::default();

            assert_eq!(
                resolution
                    .effective_map
                    .action_for(KeybindingGesture::new(ShortcutKey::Character('q'), plain)),
                Some(KeybindingAction::ZoomIn),
                "{platform:?}: explicit CLI declarations win over project and user declarations"
            );
            assert_eq!(
                resolution
                    .effective_map
                    .action_for(KeybindingGesture::new(ShortcutKey::Character('e'), plain)),
                Some(KeybindingAction::ZoomOut),
                "{platform:?}: project declarations win over user and built-in declarations"
            );
            assert_eq!(
                resolution
                    .effective_map
                    .action_for(KeybindingGesture::new(ShortcutKey::Character('y'), plain)),
                Some(KeybindingAction::PanLeft),
                "{platform:?}: user declarations win over built-in declarations when no higher source declares the action"
            );
            assert_eq!(
                resolution
                    .effective_map
                    .action_for(KeybindingGesture::new(ShortcutKey::Character('0'), plain)),
                Some(KeybindingAction::FitToWindow),
                "{platform:?}: undeclared actions retain their built-in defaults"
            );
        }
    }

    #[test]
    fn unreadable_explicit_cli_configuration_reports_a_diagnostic_and_keeps_built_ins() {
        let cli = path("/arguments/keybindings.toml");
        let resolution = resolve_startup_keybindings(
            RuntimePlatform::Linux,
            Some(cli.clone()),
            path("/project"),
            &KeybindingPathEnvironment::new(path("/home/tester"), None),
            &UnreadableCliReader,
        );

        assert!(resolution.diagnostics.iter().any(|diagnostic| {
            diagnostic.category == KeybindingDiagnosticKind::ReadFailed
                && diagnostic.source == KeybindingSource::ExplicitCli(cli.clone())
        }));
        assert_eq!(
            resolution.effective_map.action_for(KeybindingGesture::new(
                ShortcutKey::Character('0'),
                KeyModifiers::default(),
            )),
            Some(KeybindingAction::FitToWindow),
        );
    }

    fn some_path() -> AbsolutePath {
        path("/home/tester")
    }
}
