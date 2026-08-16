//! Hosted integration coverage for platform adapters.
//!
//! Native picker presentation itself is manually gated: portal and macOS
//! dialogs cannot be selected deterministically by a headless test process.

use std::{
    fs,
    path::PathBuf,
    sync::{Arc, Mutex},
};

use image_editor_core::{
    AbsolutePath, ApplicationError, CapabilityName, ExportTargetResolution, ImageFormat,
};
use image_editor_platform::{
    DialogFailure, FolderDialogRequest, LocalFileIdentityResolver, PlatformDialogBackend,
    PlatformDialogs, SaveDialogRequest,
};

#[derive(Clone, Debug, Eq, PartialEq)]
enum Call {
    ProbeFolder,
    ProbeSave,
    PickFolder(FolderDialogRequest),
    PickSave(SaveDialogRequest),
}

struct RecordingBackend {
    calls: Arc<Mutex<Vec<Call>>>,
    folder_result: std::result::Result<Option<PathBuf>, DialogFailure>,
    save_result: std::result::Result<Option<PathBuf>, DialogFailure>,
}

impl RecordingBackend {
    fn available(
        calls: Arc<Mutex<Vec<Call>>>,
        folder_result: Option<PathBuf>,
        save_result: Option<PathBuf>,
    ) -> Self {
        Self {
            calls,
            folder_result: Ok(folder_result),
            save_result: Ok(save_result),
        }
    }

    fn failing_folder(calls: Arc<Mutex<Vec<Call>>>) -> Self {
        Self {
            calls,
            folder_result: Err(DialogFailure::new("portal service disappeared")),
            save_result: Ok(None),
        }
    }

    fn record(&self, call: Call) {
        self.calls.lock().unwrap().push(call);
    }
}

impl PlatformDialogBackend for RecordingBackend {
    fn probe_folder_picker(&self) -> std::result::Result<String, DialogFailure> {
        self.record(Call::ProbeFolder);
        Ok("hosted-test-backend".to_owned())
    }

    fn probe_save_picker(&self) -> std::result::Result<String, DialogFailure> {
        self.record(Call::ProbeSave);
        Ok("hosted-test-backend".to_owned())
    }

    fn pick_folder(
        &self,
        request: FolderDialogRequest,
    ) -> std::result::Result<Option<PathBuf>, DialogFailure> {
        self.record(Call::PickFolder(request));
        self.folder_result.clone()
    }

    fn pick_export_target(
        &self,
        request: SaveDialogRequest,
    ) -> std::result::Result<Option<PathBuf>, DialogFailure> {
        self.record(Call::PickSave(request));
        self.save_result.clone()
    }
}

fn calls() -> Arc<Mutex<Vec<Call>>> {
    Arc::new(Mutex::new(Vec::new()))
}

fn absolute_temporary_path(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "image-editor-platform-integration-{name}-{}",
        std::process::id()
    ))
}

fn absolute_path(path: &std::path::Path) -> AbsolutePath {
    AbsolutePath::new(path.to_string_lossy().into_owned()).expect("test path must be absolute")
}

#[test]
fn detection_probes_folder_before_save_and_completes_before_requests() {
    let calls = calls();
    let dialogs =
        PlatformDialogs::detect(RecordingBackend::available(Arc::clone(&calls), None, None));

    assert_eq!(
        *calls.lock().unwrap(),
        vec![Call::ProbeFolder, Call::ProbeSave],
        "startup must complete both probes before any dialog request"
    );
    assert!(dialogs.folder_picker_available().is_available());
    assert!(dialogs.save_picker_available().is_available());
}

#[test]
fn dialog_requests_are_limited_to_one_folder_and_one_path_for_one_format() {
    let calls = calls();
    let folder = absolute_temporary_path("folder");
    let target = absolute_temporary_path("target.png");
    let dialogs = PlatformDialogs::detect(RecordingBackend::available(
        Arc::clone(&calls),
        Some(folder.clone()),
        Some(target.clone()),
    ));

    assert_eq!(
        dialogs.pick_folder().unwrap(),
        Some(absolute_path(&folder)),
        "the adapter must forward one selected folder"
    );
    assert_eq!(
        dialogs.pick_export_target(ImageFormat::Png).unwrap(),
        Some(absolute_path(&target)),
        "the adapter must forward one selected export path"
    );
    assert_eq!(
        *calls.lock().unwrap(),
        vec![
            Call::ProbeFolder,
            Call::ProbeSave,
            Call::PickFolder(FolderDialogRequest::single_folder()),
            Call::PickSave(SaveDialogRequest::single_path(ImageFormat::Png)),
        ]
    );
}

#[test]
fn dialog_runtime_failure_downgrades_only_the_failed_capability() {
    let calls = calls();
    let dialogs = PlatformDialogs::detect(RecordingBackend::failing_folder(Arc::clone(&calls)));

    let error = dialogs
        .pick_folder()
        .expect_err("a runtime dialog failure must be reported");
    assert!(matches!(
        error,
        ApplicationError::PlatformOperation {
            capability: CapabilityName::FolderPicker,
            ..
        }
    ));
    assert!(
        !dialogs.folder_picker_available().is_available(),
        "a failed folder dialog must be disabled for the rest of the session"
    );
    assert!(
        dialogs.save_picker_available().is_available(),
        "save dialog availability must not be changed by a folder dialog failure"
    );
    assert_eq!(
        *calls.lock().unwrap(),
        vec![
            Call::ProbeFolder,
            Call::ProbeSave,
            Call::PickFolder(FolderDialogRequest::single_folder())
        ]
    );
}

#[test]
fn filesystem_identity_distinguishes_missing_targets_and_detects_source_aliases() {
    let source_path = absolute_temporary_path("source");
    let alias_path = absolute_temporary_path("source-alias");
    let missing_path = absolute_temporary_path("missing-target");
    fs::write(&source_path, b"source bytes").unwrap();
    fs::hard_link(&source_path, &alias_path).unwrap();

    let resolver = LocalFileIdentityResolver;
    let source = resolver
        .resolve_source_identity(absolute_path(&source_path))
        .expect("regular source file must have an identity");
    let alias = resolver
        .resolve_export_target(&absolute_path(&alias_path))
        .expect("hard-link target lookup must succeed");
    let missing = resolver
        .resolve_export_target(&absolute_path(&missing_path))
        .expect("missing target lookup must succeed");

    assert!(matches!(
        alias,
        ExportTargetResolution::ExistingRegular { identity: Some(_) }
    ));
    assert_eq!(
        source.file_identity(),
        match &alias {
            ExportTargetResolution::ExistingRegular { identity } => identity.as_ref(),
            _ => unreachable!("hard link must resolve as an existing regular file"),
        },
        "hard links must share the source identity even though their paths differ"
    );
    assert_eq!(missing, ExportTargetResolution::Missing);

    fs::remove_file(source_path).unwrap();
    fs::remove_file(alias_path).unwrap();
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
#[test]
#[ignore = "requires a hosted desktop runner and human or portal interaction"]
fn native_dialog_interaction_is_explicitly_gated() {
    assert_eq!(
        std::env::var("IMAGE_EDITOR_RUN_NATIVE_DIALOG_TESTS").as_deref(),
        Ok("1"),
        "set IMAGE_EDITOR_RUN_NATIVE_DIALOG_TESTS=1 only on an interactive hosted runner"
    );
}

#[test]
fn keybinding_source_discovery_uses_priority_and_platform_user_paths() {
    use image_editor_core::{KeybindingSource, RuntimePlatform};
    use image_editor_platform::{KeybindingPathEnvironment, discover_keybinding_sources};

    let environment = KeybindingPathEnvironment::new(
        absolute_path(std::path::Path::new("/home/editor")),
        Some(absolute_path(std::path::Path::new("/xdg-config"))),
    );
    let cli = absolute_path(std::path::Path::new("/explicit/keybindings.toml"));
    let project = absolute_path(std::path::Path::new("/workspace"));

    let linux = discover_keybinding_sources(
        RuntimePlatform::Linux,
        Some(cli.clone()),
        project.clone(),
        &environment,
    );
    assert_eq!(
        linux,
        vec![
            KeybindingSource::ExplicitCli(cli),
            KeybindingSource::Project(absolute_path(std::path::Path::new(
                "/workspace/.yampixr/keybindings.toml",
            ))),
            KeybindingSource::User(absolute_path(std::path::Path::new(
                "/xdg-config/yampixr/keybindings.toml",
            ))),
            KeybindingSource::BuiltIn,
        ]
    );

    let macos = discover_keybinding_sources(RuntimePlatform::MacOs, None, project, &environment);
    assert!(matches!(macos.first(), Some(KeybindingSource::Project(_))));
    assert_eq!(
        macos[1],
        KeybindingSource::User(absolute_path(std::path::Path::new(
            "/home/editor/Library/Application Support/yampixr/keybindings.toml",
        )))
    );
    assert!(matches!(macos.last(), Some(KeybindingSource::BuiltIn)));
}

#[test]
fn local_keybinding_reader_distinguishes_optional_absence_from_unreadable_sources() {
    use image_editor_core::KeybindingSource;
    use image_editor_platform::{
        KeybindingSourceRead, KeybindingSourceReader, LocalKeybindingSourceReader,
    };

    let root = absolute_temporary_path("keybinding-reader");
    let missing = root.join("missing.toml");
    let directory = root.join("unreadable.toml");
    fs::create_dir_all(&directory).unwrap();
    let reader = LocalKeybindingSourceReader;

    assert_eq!(
        reader.read(&KeybindingSource::Project(absolute_path(&missing))),
        KeybindingSourceRead::Absent
    );
    assert_eq!(
        reader.read(&KeybindingSource::ExplicitCli(absolute_path(&missing))),
        KeybindingSourceRead::Unreadable,
        "an explicit CLI source is not optional when it is missing"
    );
    assert_eq!(
        reader.read(&KeybindingSource::User(absolute_path(&directory))),
        KeybindingSourceRead::Unreadable
    );

    fs::remove_dir_all(root).unwrap();
}
