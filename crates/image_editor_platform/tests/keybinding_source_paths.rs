use image_editor_core::{AbsolutePath, KeybindingSource, RuntimePlatform};
use image_editor_platform::{KeybindingPathEnvironment, discover_keybinding_sources};

fn path(value: &str) -> AbsolutePath {
    AbsolutePath::new(value).expect("fixture paths are absolute")
}

#[test]
fn macos_and_linux_source_discovery_uses_the_specified_platform_config_locations() {
    let project = path("/workspace");
    let environment = KeybindingPathEnvironment::new(path("/Users/editor"), None);

    let macos =
        discover_keybinding_sources(RuntimePlatform::MacOs, None, project.clone(), &environment);
    assert_eq!(
        macos,
        vec![
            KeybindingSource::Project(path("/workspace/.yampixr/keybindings.toml")),
            KeybindingSource::User(path(
                "/Users/editor/Library/Application Support/yampixr/keybindings.toml",
            )),
            KeybindingSource::BuiltIn,
        ]
    );

    let linux = discover_keybinding_sources(RuntimePlatform::Linux, None, project, &environment);
    assert_eq!(
        linux,
        vec![
            KeybindingSource::Project(path("/workspace/.yampixr/keybindings.toml")),
            KeybindingSource::User(path("/Users/editor/.config/yampixr/keybindings.toml")),
            KeybindingSource::BuiltIn,
        ]
    );
}

#[test]
fn linux_prefers_xdg_config_home_and_keeps_the_explicit_cli_layer_first() {
    let cli = path("/arguments/keybindings.toml");
    let sources = discover_keybinding_sources(
        RuntimePlatform::Linux,
        Some(cli.clone()),
        path("/workspace"),
        &KeybindingPathEnvironment::new(path("/home/editor"), Some(path("/xdg"))),
    );

    assert_eq!(sources[0], KeybindingSource::ExplicitCli(cli));
    assert_eq!(
        sources[2],
        KeybindingSource::User(path("/xdg/yampixr/keybindings.toml"))
    );
}
