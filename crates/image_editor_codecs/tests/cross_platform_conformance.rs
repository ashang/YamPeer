#![cfg(feature = "portable-codecs")]

//! Cross-platform conformance coverage using a fixed, lossless 16-bit PNG.

use std::{
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

use image_editor_codecs::{CodecRegistry, DecodeLimits, StartupPlatformCapabilities};
use image_editor_core::{
    AbsolutePath, ConformanceResult, CropDraft, DirectoryEntry, DirectoryEntryKind,
    DirectoryEntryLocation, EditorCommand, EditorState, Effect, FolderEnumerationInput,
    ImageFormat, KeyModifiers, PlatformCapability, RawKeyEvent, RuntimePlatform, ShortcutKey,
    ShortcutResolver, Utf8FileName, built_in_keybinding_map, plan_folder_enumeration, reduce,
};

static NEXT_FIXTURE_ID: AtomicU64 = AtomicU64::new(0);

/// Fixed 3×2 straight-alpha RGBA16 lossless PNG. The bytes, pixels, color
/// interpretation, and top-left orientation are intentionally source-controlled.
const CONFORMANCE_PNG: &[u8] = &[
    137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 3, 0, 0, 0, 2, 16, 6, 0,
    0, 0, 205, 228, 186, 89, 0, 0, 0, 61, 73, 68, 65, 84, 120, 218, 1, 50, 0, 205, 255, 0, 10, 10,
    20, 20, 30, 30, 255, 255, 50, 50, 60, 60, 70, 70, 128, 0, 90, 90, 100, 100, 110, 110, 0, 0, 0,
    130, 130, 140, 140, 150, 150, 255, 255, 170, 170, 180, 180, 190, 190, 191, 255, 210, 210, 220,
    220, 230, 230, 64, 0, 199, 158, 23, 91, 233, 41, 219, 58, 0, 0, 0, 0, 73, 69, 78, 68, 174, 66,
    96, 130,
];

struct FixtureFile {
    path: AbsolutePath,
}

fn canonical_image_artifact(image: &image_editor_core::CanonicalImage) -> String {
    let samples = image
        .pixels()
        .iter()
        .map(|pixel| {
            format!(
                "{},{},{},{}",
                pixel.red, pixel.green, pixel.blue, pixel.alpha
            )
        })
        .collect::<Vec<_>>()
        .join(";");
    format!(
        "dimensions={}x{}\nrgba16={samples}\n",
        image.width(),
        image.height()
    )
}

/// Persists a deterministic runner artifact only when CI requests one. The
/// test itself remains self-contained for local runs.
fn write_ci_artifact(artifact: &str) {
    let Ok(path) = std::env::var("IMAGE_EDITOR_CONFORMANCE_ARTIFACT") else {
        return;
    };
    let path = PathBuf::from(path);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("create CI artifact directory");
    }
    std::fs::write(path, artifact).expect("write deterministic conformance artifact");
}

impl Drop for FixtureFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(self.path.as_str());
    }
}

fn available_platform() -> StartupPlatformCapabilities {
    StartupPlatformCapabilities::new(
        PlatformCapability::available("conformance-folder-picker"),
        PlatformCapability::available("conformance-save-picker"),
    )
}

fn fixed_png_fixture() -> FixtureFile {
    let id = NEXT_FIXTURE_ID.fetch_add(1, Ordering::Relaxed);
    let path: PathBuf = std::env::temp_dir().join(format!(
        "image-editor-conformance-{}-{id}.png",
        std::process::id()
    ));
    std::fs::write(&path, CONFORMANCE_PNG).expect("write fixed lossless PNG fixture");
    FixtureFile {
        path: AbsolutePath::new(path.to_string_lossy().into_owned())
            .expect("temporary fixture path is absolute UTF-8"),
    }
}

fn raw_event(platform: RuntimePlatform, key: ShortcutKey) -> RawKeyEvent {
    let modifiers = match (platform, key) {
        (RuntimePlatform::MacOs, ShortcutKey::ArrowUp | ShortcutKey::ArrowDown) => {
            KeyModifiers::option()
        }
        (RuntimePlatform::Linux, ShortcutKey::ArrowUp | ShortcutKey::ArrowDown) => {
            KeyModifiers::alt()
        }
        _ => KeyModifiers::default(),
    };
    RawKeyEvent::press(key, modifiers)
}

fn apply_effects(
    mut state: EditorState,
    effects: Vec<Effect>,
    decoded_fixture: &image_editor_core::CanonicalImage,
) -> EditorState {
    for effect in effects {
        let command = match effect {
            Effect::DecodeImage { token, .. } => EditorCommand::ImageDecoded {
                token,
                image: decoded_fixture.clone(),
            },
            Effect::RenderPreview { token, request } => EditorCommand::PreviewRendered {
                token,
                image: image_editor_core::render_current_editing_result(
                    &request.source,
                    &request.history,
                    &request.draft,
                )
                .expect("reducer-generated preview request is renderable"),
            },
            Effect::EnumerateFolder { .. } | Effect::WriteExport { .. } => {
                panic!("test harness must complete enumeration before applying editing effects")
            }
        };
        let reduction = reduce(&state, command);
        state = apply_effects(reduction.state, reduction.effects, decoded_fixture);
    }
    state
}

fn dispatch_key(
    state: EditorState,
    platform: RuntimePlatform,
    key: ShortcutKey,
    decoded_fixture: &image_editor_core::CanonicalImage,
) -> EditorState {
    let command = ShortcutResolver::new(built_in_keybinding_map(platform))
        .resolve(raw_event(platform, key))
        .expect("defined conformance key resolves through the runtime table");
    let reduction = reduce(&state, command);
    apply_effects(reduction.state, reduction.effects, decoded_fixture)
}

fn run_conformance_sequence(
    platform: RuntimePlatform,
    registry: &CodecRegistry,
    fixture: &FixtureFile,
) -> (ConformanceResult, image_editor_core::CanonicalImage) {
    let decoded_fixture = registry
        .decode(ImageFormat::Png, &fixture.path, DecodeLimits::DEFAULT)
        .expect("decode fixed lossless PNG through the real registry");
    let folder = AbsolutePath::new("/conformance").unwrap();
    let entry = DirectoryEntry::new(
        fixture.path.clone(),
        Utf8FileName::new("conformance.png").unwrap(),
        DirectoryEntryLocation::Direct,
        DirectoryEntryKind::RegularFile,
        None,
    );
    let mut state = EditorState::new(registry.snapshot().clone());
    let enumeration = reduce(
        &state,
        EditorCommand::BeginFolderEnumeration {
            folder: folder.clone(),
        },
    );
    let token = match enumeration.effects.as_slice() {
        [Effect::EnumerateFolder { token, .. }] => *token,
        _ => panic!("folder enumeration emits exactly one effect"),
    };
    state = reduce(
        &enumeration.state,
        EditorCommand::FolderEnumerated {
            token,
            result: plan_folder_enumeration(
                registry.snapshot(),
                FolderEnumerationInput::Succeeded {
                    folder,
                    entries: vec![entry],
                },
            ),
        },
    )
    .state;
    let image_id = state.browsing().collection().entries()[0].id.clone();
    let activation = reduce(&state, EditorCommand::SelectImage { image_id });
    state = apply_effects(activation.state, activation.effects, &decoded_fixture);

    state = dispatch_key(
        state,
        platform,
        ShortcutKey::Character('f'),
        &decoded_fixture,
    );
    state = dispatch_key(
        state,
        platform,
        ShortcutKey::Character('r'),
        &decoded_fixture,
    );
    state = dispatch_key(
        state,
        platform,
        ShortcutKey::Character('c'),
        &decoded_fixture,
    );
    let crop = reduce(
        &state,
        EditorCommand::UpdateCropDraft {
            draft: CropDraft::new(0, 1, 2, 3),
        },
    );
    state = apply_effects(crop.state, crop.effects, &decoded_fixture);
    let crop = reduce(&state, EditorCommand::ConfirmCrop);
    state = apply_effects(crop.state, crop.effects, &decoded_fixture);

    state = dispatch_key(
        state,
        platform,
        ShortcutKey::Character('b'),
        &decoded_fixture,
    );
    for _ in 0..3 {
        state = dispatch_key(state, platform, ShortcutKey::ArrowUp, &decoded_fixture);
    }
    state = dispatch_key(state, platform, ShortcutKey::Enter, &decoded_fixture);
    state = dispatch_key(
        state,
        platform,
        ShortcutKey::Character('d'),
        &decoded_fixture,
    );
    for _ in 0..2 {
        state = dispatch_key(state, platform, ShortcutKey::ArrowDown, &decoded_fixture);
    }

    let active = state
        .browsing()
        .active()
        .expect("fixture remains active after the shared sequence");
    let document = state
        .browsing()
        .document(active)
        .expect("active fixture has one document");
    let rendered = image_editor_core::render_current_editing_result(
        document.source(),
        document.history(),
        document.draft(),
    )
    .expect("shared conformance sequence is renderable");
    (
        ConformanceResult::from_rendered(&rendered, document.history()),
        rendered,
    )
}

#[test]
fn fixed_png_fixture_decodes_to_the_documented_rgba16_samples() {
    let registry = CodecRegistry::detect(available_platform());
    let fixture = fixed_png_fixture();

    let decoded = registry
        .decode(ImageFormat::Png, &fixture.path, DecodeLimits::DEFAULT)
        .expect("decode fixed PNG fixture");
    assert_eq!((decoded.width(), decoded.height()), (3, 2));
    assert_eq!(
        decoded.pixels(),
        &[
            image_editor_core::Rgba16::new(2_570, 5_140, 7_710, u16::MAX),
            image_editor_core::Rgba16::new(12_850, 15_420, 17_990, 32_768),
            image_editor_core::Rgba16::new(23_130, 25_700, 28_270, 0),
            image_editor_core::Rgba16::new(33_410, 35_980, 38_550, u16::MAX),
            image_editor_core::Rgba16::new(43_690, 46_260, 48_830, 49_151),
            image_editor_core::Rgba16::new(53_970, 56_540, 59_110, 16_384),
        ]
    );
}

#[test]
fn runtime_command_tables_produce_identical_deterministic_pipeline_artifacts() {
    let registry = CodecRegistry::detect(available_platform());
    let fixture = fixed_png_fixture();

    let (macos, macos_rendered) =
        run_conformance_sequence(RuntimePlatform::MacOs, &registry, &fixture);
    let (linux, linux_rendered) =
        run_conformance_sequence(RuntimePlatform::Linux, &registry, &fixture);

    assert_eq!(
        macos, linux,
        "both runtime command tables must drive the same shared result"
    );
    assert_eq!(
        macos_rendered, linux_rendered,
        "both platform command tables must render identical RGBA16 samples"
    );
    assert_eq!(
        macos.crop_history().len(),
        1,
        "crop state is retained in the artifact"
    );
    assert_eq!(
        macos.orientation(),
        image_editor_core::NormalizedOrientation::TopLeft
    );
    assert_eq!(macos.serialize(), linux.serialize());
    assert_eq!(
        macos.serialize(),
        "image-editor-conformance-v1\n\
         dimensions=2x2\n\
         orientation=top-left\n\
         crop-history=0,1,2,3\n\
         rgba16=45398,47917,50435,49151;15175,17694,20212,32768;35324,37842,40361,65535;5101,7619,10138,65535\n",
        "the artifact must use stable field/sample ordering for cross-runner comparison"
    );

    let mut artifact = format!("conformance:\n{}", macos.serialize());
    for format in [ImageFormat::Png, ImageFormat::Tiff] {
        let macos_reopened = encode_and_reopen(&registry, &macos_rendered, format);
        let linux_reopened = encode_and_reopen(&registry, &linux_rendered, format);
        assert_eq!(
            macos_reopened, macos_rendered,
            "{format:?} export-reopen must preserve the macOS-equivalent RGBA16 result"
        );
        assert_eq!(
            linux_reopened, linux_rendered,
            "{format:?} export-reopen must preserve the Linux-equivalent RGBA16 result"
        );
        assert_eq!(
            macos_reopened, linux_reopened,
            "{format:?} export-reopen samples must match across platform runners"
        );
        artifact.push_str(&format!(
            "{format:?}:\n{}",
            canonical_image_artifact(&macos_reopened)
        ));
    }
    write_ci_artifact(&artifact);
}

use proptest::prelude::*;

fn encoded_fixture(format: ImageFormat, bytes: &[u8]) -> FixtureFile {
    let id = NEXT_FIXTURE_ID.fetch_add(1, Ordering::Relaxed);
    let extension = match format {
        ImageFormat::Png => "png",
        ImageFormat::Tiff => "tiff",
        _ => unreachable!("the shared pipeline property only exercises lossless formats"),
    };
    let path = std::env::temp_dir().join(format!(
        "image-editor-conformance-output-{}-{id}.{extension}",
        std::process::id()
    ));
    std::fs::write(&path, bytes).expect("write encoded conformance output");
    FixtureFile {
        path: AbsolutePath::new(path.to_string_lossy().into_owned())
            .expect("temporary output path is absolute UTF-8"),
    }
}

fn raw_event_with_shift(platform: RuntimePlatform, key: ShortcutKey, shift: bool) -> RawKeyEvent {
    let mut event = raw_event(platform, key);
    if shift {
        event.modifiers = event.modifiers.with_shift();
    }
    event
}

fn dispatch_key_with_shift(
    state: EditorState,
    platform: RuntimePlatform,
    key: ShortcutKey,
    shift: bool,
    decoded_fixture: &image_editor_core::CanonicalImage,
) -> EditorState {
    let command = ShortcutResolver::new(built_in_keybinding_map(platform))
        .resolve(raw_event_with_shift(platform, key, shift))
        .expect("generated shared editing key resolves through the runtime table");
    let reduction = reduce(&state, command);
    apply_effects(reduction.state, reduction.effects, decoded_fixture)
}

fn initialize_conformance_state(
    registry: &CodecRegistry,
    fixture: &FixtureFile,
) -> (EditorState, image_editor_core::CanonicalImage) {
    let decoded_fixture = registry
        .decode(ImageFormat::Png, &fixture.path, DecodeLimits::DEFAULT)
        .expect("decode fixed lossless PNG through the real registry");
    let folder = AbsolutePath::new("/conformance-property").expect("test folder is absolute");
    let entry = DirectoryEntry::new(
        fixture.path.clone(),
        Utf8FileName::new("conformance.png").expect("fixture filename is valid"),
        DirectoryEntryLocation::Direct,
        DirectoryEntryKind::RegularFile,
        None,
    );
    let state = EditorState::new(registry.snapshot().clone());
    let enumeration = reduce(
        &state,
        EditorCommand::BeginFolderEnumeration {
            folder: folder.clone(),
        },
    );
    let token = match enumeration.effects.as_slice() {
        [Effect::EnumerateFolder { token, .. }] => *token,
        _ => panic!("folder enumeration emits exactly one effect"),
    };
    let state = reduce(
        &enumeration.state,
        EditorCommand::FolderEnumerated {
            token,
            result: plan_folder_enumeration(
                registry.snapshot(),
                FolderEnumerationInput::Succeeded {
                    folder,
                    entries: vec![entry],
                },
            ),
        },
    )
    .state;
    let image_id = state.browsing().collection().entries()[0].id.clone();
    let activation = reduce(&state, EditorCommand::SelectImage { image_id });
    (
        apply_effects(activation.state, activation.effects, &decoded_fixture),
        decoded_fixture,
    )
}

fn current_dimensions(state: &EditorState) -> (u32, u32) {
    let active = state
        .browsing()
        .active()
        .expect("conformance fixture stays active for the entire generated sequence");
    let document = state
        .browsing()
        .document(active)
        .expect("active conformance fixture retains its document");
    let rendered = image_editor_core::render_current_editing_result(
        document.source(),
        document.history(),
        document.draft(),
    )
    .expect("generated operations remain valid and renderable");
    (rendered.width(), rendered.height())
}

fn run_generated_conformance_sequence(
    platform: RuntimePlatform,
    registry: &CodecRegistry,
    fixture: &FixtureFile,
    actions: &[(u8, u8)],
) -> (ConformanceResult, image_editor_core::CanonicalImage) {
    let (mut state, decoded_fixture) = initialize_conformance_state(registry, fixture);

    for &(action, crop_seed) in actions {
        state = match action {
            0 => dispatch_key_with_shift(
                state,
                platform,
                ShortcutKey::Character('f'),
                false,
                &decoded_fixture,
            ),
            1 => dispatch_key_with_shift(
                state,
                platform,
                ShortcutKey::Character('f'),
                true,
                &decoded_fixture,
            ),
            2 => dispatch_key_with_shift(
                state,
                platform,
                ShortcutKey::Character('r'),
                false,
                &decoded_fixture,
            ),
            3 => dispatch_key_with_shift(
                state,
                platform,
                ShortcutKey::Character('r'),
                true,
                &decoded_fixture,
            ),
            4 => {
                let entering = dispatch_key_with_shift(
                    state,
                    platform,
                    ShortcutKey::Character('c'),
                    false,
                    &decoded_fixture,
                );
                let (width, height) = current_dimensions(&entering);
                let left = u32::from(crop_seed) % width;
                let top = u32::from(crop_seed.rotate_left(4)) % height;
                let updated = reduce(
                    &entering,
                    EditorCommand::UpdateCropDraft {
                        draft: CropDraft::new(left, top, width, height),
                    },
                );
                let confirmed = reduce(&updated.state, EditorCommand::ConfirmCrop);
                apply_effects(confirmed.state, confirmed.effects, &decoded_fixture)
            }
            5 => dispatch_key_with_shift(
                state,
                platform,
                ShortcutKey::Character('b'),
                false,
                &decoded_fixture,
            ),
            6 => dispatch_key_with_shift(
                state,
                platform,
                ShortcutKey::Character('d'),
                false,
                &decoded_fixture,
            ),
            7 => dispatch_key_with_shift(
                state,
                platform,
                ShortcutKey::ArrowUp,
                false,
                &decoded_fixture,
            ),
            8 => dispatch_key_with_shift(
                state,
                platform,
                ShortcutKey::ArrowDown,
                false,
                &decoded_fixture,
            ),
            9 => dispatch_key_with_shift(
                state,
                platform,
                ShortcutKey::Enter,
                false,
                &decoded_fixture,
            ),
            _ => unreachable!("the generator only produces defined shared editing actions"),
        };
    }

    let active = state
        .browsing()
        .active()
        .expect("conformance fixture remains active after generated commands");
    let document = state
        .browsing()
        .document(active)
        .expect("active conformance fixture has a document");
    let rendered = image_editor_core::render_current_editing_result(
        document.source(),
        document.history(),
        document.draft(),
    )
    .expect("generated document history and drafts are renderable");
    (
        ConformanceResult::from_rendered(&rendered, document.history()),
        rendered,
    )
}

fn encode_and_reopen(
    registry: &CodecRegistry,
    image: &image_editor_core::CanonicalImage,
    format: ImageFormat,
) -> image_editor_core::CanonicalImage {
    let mut encoded = Vec::new();
    registry
        .encode(image, format, &mut encoded)
        .expect("PNG/TIFF capability is detected before the lossless conformance check");
    let output = encoded_fixture(format, &encoded);
    registry
        .decode(format, &output.path, DecodeLimits::DEFAULT)
        .expect("reopen real lossless conformance output")
}

// Feature: macos-image-editor, Property 10: Shared pipeline is platform-equivalent
// Validates: Requirements 7.7, 10.1, 10.2, 10.5
proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    #[test]
    fn generated_shared_edit_and_draft_sequences_are_platform_equivalent_and_lossless(
        actions in prop::collection::vec((0_u8..10, any::<u8>()), 0..40),
    ) {
        let registry = CodecRegistry::detect(available_platform());
        let fixture = fixed_png_fixture();
        let (macos_result, macos_rendered) = run_generated_conformance_sequence(
            RuntimePlatform::MacOs,
            &registry,
            &fixture,
            &actions,
        );
        let (linux_result, linux_rendered) = run_generated_conformance_sequence(
            RuntimePlatform::Linux,
            &registry,
            &fixture,
            &actions,
        );

        prop_assert_eq!(&macos_result, &linux_result);
        prop_assert_eq!(&macos_rendered, &linux_rendered);

        for format in [ImageFormat::Png, ImageFormat::Tiff] {
            let macos_reopened = encode_and_reopen(&registry, &macos_rendered, format);
            let linux_reopened = encode_and_reopen(&registry, &linux_rendered, format);
            prop_assert_eq!(&macos_reopened, &macos_rendered);
            prop_assert_eq!(&linux_reopened, &linux_rendered);
            prop_assert_eq!(&macos_reopened, &linux_reopened);
        }
    }
}
