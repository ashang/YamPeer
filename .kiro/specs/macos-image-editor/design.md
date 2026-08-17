# Technical Design: macOS Image Editor

## Overview

Image Editor will be a native Rust desktop application for macOS and Linux. It will use a single shared Rust domain layer for collection discovery, commands, edit-history semantics, image transformation, capability projection, and export planning. Platform-specific code is restricted to the window/event host, native folder/save dialogs, runtime capability probes, file identities, and distributable package assembly. This preserves equivalent editing results across supported platforms while allowing each platform to report and disable unavailable integrations instead of failing to start.

**Selected approach.** The application will use `eframe` + `egui` with its native `wgpu` renderer for the one focused desktop workspace. `eframe` is selected because it supplies a Rust-native application host for macOS and Linux, while `egui` enables a compact keyboard-first workspace without a JavaScript runtime. The UI is deliberately an adapter over a pure core state machine; it does not own editing decisions. Native dialogs will be accessed through an internal `PlatformDialogs` adapter backed by `rfd`, which supports macOS and Linux native dialog backends, including XDG Desktop Portal and GTK choices on Linux. Portable JPEG, PNG, and TIFF processing will use `image-rs`; HEIC will be an optional `libheif-rs` adapter. `libheif-rs` and its libheif codec dependencies are optional so their absence is observable capability loss, not an application-start failure.

This approach satisfies the Rust/shared-behavior constraint and avoids selecting an OS-specific image framework. It also keeps the initial UI efficient: pixel processing runs off the UI thread; GPU upload happens only for a newly rendered preview revision; the immediate-mode UI redraws controls while rendering work is pending.

**Typography reliability.** Each macOS and Linux installable package will ship a vetted `Noto Sans CJK SC` font resource under the SIL Open Font License 1.1 (or an equivalently licensed replacement with the same required coverage), rather than relying on a user-installed system font. The packaged resource covers simplified Chinese, common Latin text, and UI punctuation/symbols needed by `Required_Text`. The desktop host treats this resource as a startup prerequisite for textual rendering: it reads and validates the packaged bytes, prepares `egui::FontDefinitions`, and registers the font as both the first proportional-family choice and a fallback before the first workspace frame. A missing, unreadable, malformed, or unregistrable resource follows a visible safe startup-error path and never silently falls through to tofu/missing-glyph rendering.

### Research findings

- [`eframe` documentation](https://docs.rs/eframe/latest/eframe/) describes native application hosting and selectable native renderers; the design uses its native host plus `wgpu`.
- [`rfd` documentation](https://docs.rs/rfd/latest/rfd/) documents macOS support and Linux GTK/XDG Portal backends. Its Linux runtime dependencies justify explicit availability detection and degraded mode.
- [`image` documentation](https://docs.rs/image/latest/image/) documents Rust-native image decode/encode APIs and typed image buffers, suitable for the portable codec adapter.
- [`libheif-rs` documentation](https://docs.rs/libheif-rs/latest/libheif_rs/) documents the libheif wrapper, optional image integration, and Linux libheif dependency. It supports the optional HEIC adapter strategy.

Content in this research summary was rephrased for compliance with licensing restrictions.

### Scope and non-goals

- The design supports JPEG, PNG, TIFF, and, when detected, HEIC; it does not silently claim HEIC support because an extension exists.
- Editing is non-destructive during the open session. Export creates a new file and never overwrites a source or an existing target.
- One process owns exactly one `Primary_Main_Window`. Separate multi-window editing, recursive folder scans, cloud storage, metadata editing, and persistence of edit sessions across application restarts are outside this feature.
- The initial renderer supports a scaled display texture and source-pixel crop overlay. It does not make display coordinates authoritative for crop geometry.

## Architecture

### Layered architecture

```mermaid
flowchart TB
  UI[egui Primary_Main_Window\nviews, shortcuts, messages] --> APP[Desktop application adapter]
  APP --> ROUTER[Keyboard and control command router]
  ROUTER --> CORE[image_editor_core\nPure reducer and state]
  CORE --> PIPE[Deterministic image pipeline]
  CORE --> PLAN[Collection, capability, and export planners]
  APP --> ASYNC[Worker executor]
  ASYNC --> FILES[Filesystem adapter]
  ASYNC --> CODECS[Codec registry\nportable + optional HEIC]
  APP --> DIALOGS[PlatformDialogs adapter\nrfd/macOS, XDG portal or GTK/Linux]
  APP --> GPU[Preview texture adapter]
  CORE --> UI
```

The workspace will be organized as these Rust crates/modules; each dependency version is pinned to an exact vetted version in `Cargo.toml` and locked in `Cargo.lock` at implementation time.

| Unit | Responsibility | Platform dependence |
|---|---|---|
| `image_editor_core` | Domain models, pure command reducer, edit history, collection filtering and configurable deterministic ordering, startup-folder/first-activation planning, settings validation, capability projection, deterministic transforms, export request planning | None |
| `image_editor_codecs` | `CodecRegistry`, safe image resource limits, portable `image-rs` codecs, optional HEIC adapter | HEIC runtime library/codec plugins only |
| `image_editor_platform` | `PlatformDialogs`, platform detection, settings-path/current-directory resolution, atomic settings storage, file identity, package/runtime probe implementations | macOS/Linux |
| `image_editor_desktop` | eframe startup, egui views, key-event intake, task orchestration, preview texture cache | Native window/graphics APIs through eframe |

`image_editor_core` must never import `egui`, `eframe`, `rfd`, OS APIs, or asynchronous runtime types. Its operations accept values and return a new state plus declarative effects, which makes the command and rendering behavior testable without a display server or installed codec.

### State ownership and effect flow

1. Startup resolves exactly one settings file location before any settings I/O: macOS `~/Library/Application Support/yampixr/settings.json`; Linux `$XDG_CONFIG_HOME/yampixr/settings.json` only when `XDG_CONFIG_HOME` is absolute, otherwise `~/.config/yampixr/settings.json`. The adapter then performs a 1 MiB-bounded read and the pure settings decoder validates the schema version, sort field/direction, and optional absolute last-folder path. Absence selects defaults without a notice; every other read/validation failure selects `full_file_name/ascending` and emits a safe settings diagnostic.
2. The process captures `StartupWorkingDirectory` once. The pure startup planner attempts an accessible, enumerable persisted `LastSuccessfulSourceFolder` first, then the captured startup working directory. Each failed candidate emits a safe diagnostic and advances exactly once; failure to obtain the working directory, or exhaustion of both candidates, commits an operable empty workspace.
3. Every startup enumeration carries a monotonically increasing `StartupRequestRevision`; a successful enumeration creates a new `CollectionRevision`, applies `EffectiveSortSettings`, and atomically installs that source folder and ordered collection only when both revisions match the current pending startup request. An empty collection emits no decode effect. A nonempty collection emits exactly one `StartupActivationPlan` for the first sorted entry; decode success atomically activates and displays that entry, while decode failure leaves no startup-selected active image and never advances automatically to a later entry.
4. The desktop adapter creates exactly one primary window and hands the immutable capability snapshot plus validated effective settings to the pure core.
5. A button or normalized keyboard event becomes one `EditorCommand`. The core validates it synchronously, changes only domain state that it owns, and optionally emits an effect such as `ChooseFolder`, `DecodeCandidate`, `PersistSettings`, or `WriteExport`.
6. The adapter executes effects on the correct boundary: dialogs on the UI/main thread; settings I/O, file enumeration, decoding, replay, encoding, and disk writes on a bounded worker executor.
7. Each effect includes a monotonically increasing request/revision token. Completion is applied only if its token still matches the relevant pending request and collection/document revision; stale startup enumeration or first-decode work cannot replace a user-opened folder, changed ordering, newer active image, or preview.
8. Success is committed atomically through a core completion command. Failure becomes a visible `ApplicationError` or safe startup/settings diagnostic and preserves the state specified by the requirements.

The core uses a reducer style rather than mutating UI callbacks directly. This is the mechanism that ensures a failed decode, unavailable feature, boundary navigation, or invalid crop cannot partially update `Browsing_State`.

### Startup and capability lifecycle

#### Font bootstrap before workspace creation

`FontBootstrapper` runs before the desktop host creates the normal `Primary_Main_Window`. It resolves the package-relative `resources/fonts/NotoSansCJKsc-Regular.otf` path through the platform package adapter, reads the complete resource, verifies that the byte stream contains the required font face, and constructs a `FontConfiguration` value. The host registers that value through `egui::FontDefinitions` before the primary-window creation call; where eframe provides the `egui` context only in `CreationContext`, the host performs the registration as the first creation callback operation and prevents the workspace from being created or drawn until registration succeeds. The bundled face is inserted at the front of both the proportional and monospace family lists and remains in the fallback lists. This makes Chinese UI labels, Chinese filenames, and Chinese notices take the bundled face without depending on an installed system font, while preserving the normal `egui` fallback chain for non-required code points.

The bootstrap result is explicit: `Ready(FontConfiguration)` or `Failed(FontBootstrapFailure)`. A failure to locate/read the resource, parse its required face, construct `FontDefinitions`, or register the definitions does not create a normal editor workspace and does not accept input commands. The platform launcher instead presents a minimal, safe `Startup_Availability_Error` using a native-safe error path that does not need the failed font, records a local diagnostic category only, and exits nonzero or remains in a non-editable startup-error state. The message identifies the unavailable font capability/resource category but excludes raw font bytes, stack traces, and untrusted file data.

`CapabilityDetector::detect()` returns a complete `CapabilitySnapshot` before the command router accepts `OpenFolder` or `Export`:

```text
CapabilitySnapshot {
  formats: BTreeMap<ImageFormat, FormatCapability>,
  folder_picker: PlatformCapability,
  save_picker: PlatformCapability,
  diagnostics: Vec<AvailabilityMessage>,
}
```

`FormatCapability` contains independent `decode` and `encode` availability, the provider (`portable-rust` or `libheif`), and a human-readable unavailable reason. `PlatformCapability` contains `available`, backend identity, and a reason where unavailable. This representation prevents the common error of treating an extension or a compiled feature as proof that both decode and encode work.

Portable JPEG, PNG, and TIFF availability is determined from the compiled codec registry and an in-process decode/encode self-check using fixed fixture bytes and a bounded sink. HEIC is available only when the optional HEIC adapter is linked **and** its runtime `libheif` initialization reports the required decoder/encoder for the operation. Decode and encode are recorded independently. The application must not load or require HEIC merely to open JPEG, PNG, or TIFF.

On macOS, the dialog capability probe verifies that the native adapter has been constructed. On Linux, it verifies the selected rfd backend can be used: first the XDG Portal D-Bus service and a portal implementation, or the intentionally packaged GTK backend and required libraries. A failed probe creates a disabled dependent operation and a non-blocking availability message; it does not prevent the primary window from opening. The actual dialog invocation still reports a normal application error if the service disappears after probing.

## Components and Interfaces

### Domain state and command reducer

```rust
pub struct EditorState {
    pub capabilities: CapabilitySnapshot,
    pub browsing: BrowsingState,
    pub mode: InteractionMode,
    pub pending: BTreeMap<RequestId, PendingRequest>,
    pub notices: Vec<VisibleNotice>,
}

pub struct BrowsingState {
    pub source_folder: Option<AbsolutePath>,
    pub collection: ImageCollection,
    pub active: Option<ImageId>,
    pub documents: BTreeMap<ImageId, ImageDocument>,
    pub preview: PreviewState,
}

pub struct ImageDocument {
    pub source: SourceImage,
    pub history: Vec<EditOperation>,
    pub redo: Vec<EditOperation>,
    pub draft: DraftAdjustments,
    pub revision: Revision,
}

pub enum EditOperation {
    FlipHorizontal,
    FlipVertical,
    RotateClockwise90,
    RotateCounterclockwise90,
    Crop(CropRect),
    Brightness(i16),
    Contrast(i16),
}
```

`ImageCollection` stores only supported, directly contained regular files and the metadata needed by the selected sort field. Collection planning accepts `EffectiveSortSettings`: `FullFileName`, `ModifiedTime`, or `FileSize`, together with `Ascending` or `Descending`. It compares the selected available field in the requested direction; entries missing selected metadata sort after entries with metadata. Every primary-field tie, including missing metadata, is broken by complete filename UTF-8 bytes ascending and then complete local-path UTF-8 bytes ascending, independent of requested direction. Discovery takes an injected directory listing and sort settings so membership and every ordering mode can be tested independently of the operating system. Directories, descendant files, and files whose extension is not one of the defined candidate extensions are never collection entries. HEIC candidates whose decoder capability is false remain visible as availability notices rather than selectable collection entries, as required.

`ImageId` is a stable source identity for this open session, composed of the absolute path plus platform file identity metadata where available. It keeps individual history and redo stacks separate even when two files have identical names. Every generated filename/path used for the requirement-defined ordering must be UTF-8; a filename that cannot be represented is reported as an availability/error notice and is not incorrectly sorted by a lossy replacement representation.

The reducer exposes the following interface:

```rust
pub fn reduce(state: &EditorState, command: EditorCommand) -> Reduction;

pub struct Reduction {
    pub state: EditorState,
    pub effects: Vec<Effect>,
}
```

`Reduction` either retains the relevant prior state or produces a complete validated new state. Effects have no authority to mutate state; their typed completions (`FolderEnumerated`, `ImageDecoded`, `PreviewRendered`, `ExportWritten`, `OperationFailed`) re-enter the reducer. This separation makes the required retain-prior-state behavior explicit.

### UI workspace

`DesktopApp` renders the only primary window as a three-region layout:

- **Collection pane:** open-folder control, supported-image file-name entries, selected-image indicator, unavailable-format availability messages, and folder enumeration errors.
- **Preview pane:** source-name title including extension, empty-collection/no-active messages, decoded preview, pending-progress indicator, crop overlay, and error banner. The crop overlay converts pointer positions to source pixel coordinates only through the current render transform; all persisted values are source-pixel integers.
- **Command pane:** visible controls and platform-correct shortcut labels for flip, rotation, crop entry/confirm/cancel, adjustment focus/increase/decrease/commit, undo, redo, and export. Availability projection disables any command whose required capability is absent and explains why.

The renderer receives an already-computed `ViewModel` from the core. Controls emit the same `EditorCommand` types as keys, so there is one command path. A control is enabled only when `CapabilitySnapshot`, active-image state, crop mode, and history state allow its command. When an active image exists, all required applicable controls are simultaneously present in the one window.

### Keyboard abstraction

The desktop adapter normalizes eframe/winit key events into `RawKeyEvent { key, modifiers, pressed, repeat }`. `ShortcutResolver` maps it to an `EditorCommand` using the runtime platform:

| Intent | macOS input/label | Linux input/label |
|---|---|---|
| Undo / redo | `Command+Z` / `Command+Shift+Z` | `Control+Z` / `Control+Shift+Z` |
| Adjustment increase / decrease | `Option+Up` / `Option+Down` | `Alt+Up` / `Alt+Down` |
| Navigation | Left, Right, Home, End | Left, Right, Home, End |
| Edit actions | F, Shift+F, R, Shift+R, C, B, D, Return | F, Shift+F, R, Shift+R, C, B, D, Return |

The resolver is a pure table indexed by `RuntimePlatform`, physical/logical key identity, and modifier set. It accepts exactly one command for one non-repeat press event and ignores release, auto-repeat, and events already consumed by an active text-capable control. No UI widget independently executes a shortcut, preventing duplicate processing. The platform-specific table changes modifier recognition and labels only; both tables emit the same semantic command.

### Image codec and processing interfaces

```rust
pub trait ImageCodec: Send + Sync {
    fn capability(&self, format: ImageFormat) -> FormatCapability;
    fn decode(&self, path: &AbsolutePath, limits: DecodeLimits)
        -> Result<DecodedSource, CodecError>;
    fn encode(&self, image: &CanonicalImage, format: ImageFormat,
              destination: &mut dyn Write) -> Result<(), CodecError>;
}

pub trait PlatformDialogs: Send + Sync {
    fn folder_picker_available(&self) -> PlatformCapability;
    fn save_picker_available(&self) -> PlatformCapability;
    fn pick_folder(&self) -> Result<Option<AbsolutePath>, PlatformError>;
    fn pick_export_target(&self, formats: &[ImageFormat])
        -> Result<Option<ExportTarget>, PlatformError>;
}

pub trait FileSystem: Send + Sync {
    fn enumerate_direct_files(&self, folder: &AbsolutePath)
        -> Result<Vec<DirectoryEntry>, FileError>;
    fn identity(&self, path: &AbsolutePath) -> Result<FileIdentity, FileError>;
    fn create_new(&self, path: &AbsolutePath) -> Result<NewFile, FileError>;
}
```

`image_editor_codecs` registers the portable codecs unconditionally when compiled with their pinned features. `OptionalHeicCodec` is registered only after runtime probe success. The registry selects by the requested available format, not by filename extension alone. File-content decode errors are distinct from unavailable capabilities, so a malformed supported PNG reports a decode `ApplicationError` rather than misleadingly disabling PNG.

### Shared image-processing pipeline

```mermaid
flowchart LR
  S[Source bytes] --> D[Decode and normalize once]
  D --> B[CanonicalImage: straight-alpha RGBA16, fixed sRGB interpretation]
  B --> H[Replay committed Edit_History]
  H --> A[Apply uncommitted brightness/contrast drafts]
  A --> P[Current_Editing_Result]
  P --> T[Preview scale/upload]
  P --> E[Format encoder]
```

`CanonicalImage` is an owned, deterministic, straight-alpha `RGBA16` raster with explicit width and height. The decoder normalizes every supported source to this representation and applies source-orientation normalization once before it becomes the base image. The pipeline uses integer arithmetic and specified rounding, never platform floating-point image APIs:

- Horizontal flip maps `(x, y)` to `(W - 1 - x, y)`.
- Vertical flip maps `(x, y)` to `(x, H - 1 - y)`.
- Clockwise rotation maps `(x, y)` to `(H - 1 - y, x)` with dimensions `(H, W)`.
- Counterclockwise rotation maps `(x, y)` to `(y, W - 1 - x)` with dimensions `(H, W)`.
- Crop validates a half-open `CropRect { left, top, right, bottom }` against the dimensions of the image produced by prior committed operations, then copies exactly that source-coordinate rectangle.
- Brightness and contrast accept only `-100..=100`. Brightness adds a signed fixed-point fraction of the full 16-bit range to each RGB sample and clamps; contrast scales the distance from the 16-bit midpoint by `(100 + value) / 100`, rounds with a documented ties-to-nearest rule, and clamps. Alpha is unchanged. A value of zero is an identity transformation.

The exact arithmetic functions live only in `image_editor_core`. `image-rs` is used for decode/encode and buffer conversion, never for a platform-dependent edit operation. This is the critical cross-platform equivalence boundary.

Rendering is lazy and revision-keyed. The worker replays the source plus committed history and drafts into a `CurrentEditingResult`. A cache keyed by `(ImageId, history_revision, draft_values)` may retain intermediate images, but cached and uncached execution must call the same operation implementations. A thumbnail/preview may be downscaled after the full-resolution result has been produced; export always encodes the full-resolution result.

### Crop and adjustment interaction

`InteractionMode` is `Browse`, `Crop(CropDraft)`, or `Adjust(FocusedAdjustment)`. Crop entry requires an active image and creates a draft constrained to integer source pixels. The UI may show the initial selection and pointer handles in screen space, but it converts through `PreviewTransform` and clamps to `[0, W] × [0, H]` before creating `CropDraft`. Confirming a valid non-empty draft appends exactly one `Crop` operation, clears only that document's redo history, exits crop mode, and requests a preview. Invalid confirmation leaves the crop draft, history, and preview unchanged and creates an error. Cancel discards the draft without touching history or preview.

Each `ImageDocument` retains brightness and contrast draft values plus its focused field. `B`/`D` changes focus without committing. Increase/decrease changes only the focused value by exactly one inside the closed range; at either endpoint it is an explicit no-op that still leaves the current preview visible. The current preview includes committed history followed by the two non-zero drafts in fixed order (brightness, then contrast). `Return` appends exactly one operation for the focused value, including zero, resets that focused value to zero, clears only that document's redo history, and rebuilds the preview from the committed result plus any remaining draft. This realizes the required visible, uncommitted adjustment behavior without destructively altering the source.

### Navigation and decoding

`NavigationPlanner` is a pure function that returns `NoTarget`, `Candidate(ImageId)`, or `NoActiveImage` for Left, Right, Home, and End. It uses the defined collection ordering and never wraps. The adapter must decode a candidate before sending `NavigationDecodeSucceeded`. Only that completion updates `active` and preview. Decode failure retains the old active image, histories, preview, and collection, and emits an error containing the candidate filename. Empty-collection navigation retains state and presents the required empty preview message; navigation with a nonempty collection but no active image also retains state.

Folder enumeration follows the same two-phase pattern. The previous `BrowsingState` remains live until enumeration succeeds. A successful result atomically installs the source folder and new ordered collection, clears stale active/document state, and shows each supported file name. Selecting an entry likewise decodes before activation.

### Non-destructive history and export

For each `ImageDocument`, `history` and `redo` are stacks ordered by application time. Applying a new operation appends it and clears **only** the active document's redo stack. Undo pops the history tail and pushes that operation to redo; redo pops redo and pushes to history. Empty stacks are no-ops. Documents associated with other collection entries are never modified by an active document's command.

Export is a planned, guarded transaction:

1. `Export` is enabled only with an active image, save-picker capability, and at least one encoder capability. The dialog receives only `Available_Export_Format` choices.
2. The user selects one format and path. The core validates that the format is currently encodable and constructs `ExportPlan` containing the active document's immutable source identity, history/draft revision, desired path, and format.
3. The filesystem adapter resolves file identities for source and target where target exists. If the target identifies the source or another existing local regular file, it rejects the plan before opening a writer and preserves all bytes/state.
4. The render worker computes the exact `Current_Editing_Result` for the plan revision. The writer opens the target with exclusive `create_new`, encodes to a sibling temporary/new destination according to the filesystem adapter, flushes, then atomically publishes only if publication cannot replace an existing file. A collision/race is a failure, never permission to overwrite.
5. On any failure, the source, history, redo, and displayed preview remain unchanged; a path-specific error is displayed. On success, exactly one new export file exists and the source byte sequence remains unchanged.

PNG and TIFF encode the canonical result without changing width, height, or RGBA samples. JPEG and HEIC use the selected encoder and retain orientation-normalized dimensions and crop result; their sample differences may be lossy as defined by the requirements. Reopening an export always uses the normal decoder and therefore verifies the appropriate output equivalence in integration tests.

## Data Models

```rust
pub enum ImageFormat { Jpeg, Png, Tiff, Heic }

pub struct FormatCapability {
    pub decode: Availability,
    pub encode: Availability,
    pub provider: Option<CodecProvider>,
}

pub enum Availability {
    Available,
    Unavailable { reason: AvailabilityReason },
}

pub struct ImageCollection {
    pub entries: Vec<CollectionEntry>, // ordered by EffectiveSortSettings
    pub unavailable: Vec<UnavailableImage>,
    pub revision: CollectionRevision,
}

pub struct CollectionEntry {
    pub id: ImageId,
    pub absolute_path: AbsolutePath,
    pub file_name: Utf8FileName,
    pub format: ImageFormat,
    pub modified_time: Option<PortableTimestamp>,
    pub file_size: Option<u64>,
}

pub struct AppSettings {
    pub version: u16,
    pub sort: SortSettings,
    pub last_successful_source_folder: Option<AbsolutePath>,
}

pub struct SortSettings {
    pub field: SortField,
    pub direction: SortDirection,
}

pub enum SortField { FullFileName, ModifiedTime, FileSize }
pub enum SortDirection { Ascending, Descending }

pub struct StartupPlan {
    pub candidates: Vec<StartupFolderCandidate>,
    pub effective_sort: SortSettings,
}

pub struct StartupActivationPlan {
    pub candidate: ImageId,
    pub enumeration_request: RequestId,
    pub collection_revision: CollectionRevision,
}

pub struct CropRect {
    pub left: u32,
    pub top: u32,
    pub right: u32,
    pub bottom: u32,
}

pub struct DraftAdjustments {
    pub brightness: i16, // invariant: -100..=100
    pub contrast: i16,   // invariant: -100..=100
    pub focused: Option<AdjustmentKind>,
}

pub struct CanonicalImage {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<Rgba16>, // exactly width * height, row-major
    pub color_space: CanonicalColorSpace, // sRGB
    pub orientation: NormalizedOrientation,
}
```

Core constructors keep the following invariants:

- A `CropRect` is valid iff `left < right ≤ width` and `top < bottom ≤ height`; its bounds use half-open source-pixel coordinates.
- A `CanonicalImage` has nonzero dimensions, an overflow-checked `width * height` count, and exactly one pixel per coordinate.
- `history`, `redo`, and drafts belong to exactly one `ImageId`; `ImageId` does not depend on its display filename.
- `CurrentEditingResult` is a pure value derived from immutable base pixels, ordered history, and current drafts. It never aliases writable source-file memory.
- Capability availability is explicit for every decode/encode and dialog operation. No missing capability is represented by a panic, absent enum variant, or implicit fallback.
- `VisibleNotice` has a severity (`Availability`, `Error`, `Info`), stable subject (filename/path/capability), and user-safe message. It never exposes stack traces or untrusted raw file content.

Resource limits are part of `DecodeLimits`: maximum input bytes, dimensions, total pixels, and intermediate allocation. Exceeding them is a decode failure with no browsing-state update. Every multiplication/addition used for buffer dimensions is checked before allocation.

### Settings storage, startup restoration, and deterministic sorting

`AppSettingsStore` is the only component allowed to read or replace the versioned settings file. Platform path resolution occurs before construction of the store and returns either one absolute location or a typed startup diagnostic. Reads open at most one file, reject a reported size above 1 MiB, stream at most 1 MiB plus one sentinel byte to detect metadata races, decode UTF-8 JSON only when no overflow sentinel is present, reject trailing/malformed data and unsupported versions, and validate every field before returning `Valid(AppSettings)`. A missing file returns `Absent`; unreadable, oversized, malformed, unsupported, or semantically invalid files return `Invalid(SettingsDiagnostic)`. Raw settings text, environment values, and temporary-file contents never enter notices.

```rust
pub trait AppSettingsStore {
    fn load(&self, location: &SettingsStorageLocation) -> SettingsLoadOutcome;
    fn replace(&self, location: &SettingsStorageLocation, value: &AppSettings)
        -> Result<(), SettingsWriteError>;
}

pub enum SettingsLoadOutcome {
    Absent,
    Valid(AppSettings),
    Invalid(SettingsDiagnostic),
}

pub struct StartupState {
    pub request_revision: StartupRequestRevision,
    pub pending_candidate: Option<StartupFolderCandidate>,
    pub attempted_persisted_folder: bool,
    pub attempted_working_directory: bool,
    pub activation: Option<StartupActivationPlan>,
}
```

Writes serialize the complete settings value to canonical JSON, create a uniquely named sibling temporary file with create-new semantics, write and flush all bytes, sync the file, then atomically rename the sibling over `settings.json` on the same filesystem. The adapter creates the parent application directory when absent and syncs the parent directory after replacement where the platform supports that operation. A failure before replacement removes only the attempt-created temporary file; a failure never truncates the last complete settings file. The reducer keeps a newly selected sort setting effective for the current session even if persistence fails. A successfully enumerated user-selected folder updates only `last_successful_source_folder`, preserving the effective sort setting in the complete replacement value.

`StartupFolderPlanner` is a pure state machine. It receives validated settings, the captured startup working-directory result, and typed metadata/enumeration completions. Candidate order is fixed: persisted folder, then startup working directory. Duplicate paths are attempted only once. Every enumeration request and completion carries `StartupRequestRevision`; every first-image decode additionally carries `CollectionRevision`. A completion with either stale token is discarded before any state field, notice, history, preview, or view state is changed. User-initiated folder selection supersedes and increments the pending startup revision.

`EffectiveImageOrder` uses one total comparator. For `full_file_name`, it compares complete filename UTF-8 bytes in the selected direction. For `modified_time` and `file_size`, entries with values precede missing values regardless of direction, present values compare in the selected direction, and missing values tie. Every primary tie is resolved by complete filename UTF-8 bytes ascending and then complete absolute-path UTF-8 bytes ascending, independent of sort direction. The same comparator is used for startup enumeration, user-opened folders, and in-session resorting. Resorting changes only collection order; `ImageId` retains the active document, histories, redo stacks, drafts, preview revision, and view state.

## Correctness Properties

A correctness invariant is a characteristic or behavior that should hold true across all valid executions of a system—essentially, a formal statement about what the system should do. Correctness invariants serve as the bridge between human-readable specifications and machine-verifiable correctness guarantees.

PBT applies to the shared pure core: ordered collections, command reduction, integer image transforms, crop validation, adjustment arithmetic, history transitions, keyboard translation, capability projection, and in-memory lossless codec contracts. It does **not** apply to native window layout, portal/GTK availability, OS dialogs, package metadata, or physical filesystem writes; those need focused example, smoke, and integration tests.

The following properties are the non-redundant result of property reflection. For example, geometric transform properties include dimension and pixel-placement checks together, and history properties include both LIFO movement and redo invalidation rather than splitting implied assertions into separate properties.

### Property 1: Capability-filtered collection is complete and ordered

For any directory listing with valid UTF-8 direct regular files and any capability snapshot, collection planning shall include every and only decodable JPEG, PNG, TIFF, or HEIC candidate, exclude descendants/directories and undecodable candidates, and order included entries by complete filename UTF-8 bytes then full-path UTF-8 bytes.

**Validates: Requirements 1.2, 1.4, 9.3, 9.4**

### Property 2: Candidate activation is atomic

For any prior browsing state and any selection or navigation candidate, a successful decode shall set exactly that candidate as active and render its preview, while any decode error shall leave the source folder, collection, active image, histories, redo stacks, and preview equal to the prior state and produce an error naming the candidate.

**Validates: Requirements 1.5, 1.6, 1.7, 2.1, 2.2, 2.3, 2.4, 2.5, 2.6, 2.7, 2.8, 2.9**

### Property 3: Navigation targets obey collection order and boundaries

For any ordered nonempty collection and active index, Left, Right, Home, and End shall select respectively the preceding, following, first, or last valid target when one exists; at a boundary they shall produce no target and retain state. For any empty collection or missing active image, those commands shall retain the prior state.

**Validates: Requirements 2.10, 2.11, 2.12, 2.13**

### Property 4: Geometric operations preserve their specified pixel mapping

For any nonempty canonical image and every valid source coordinate, horizontal flip, vertical flip, clockwise rotation, and counterclockwise rotation shall place that source pixel at the required destination coordinate and dimensions; applying clockwise rotation four times shall yield an image equal in dimensions and every pixel location to the original.

**Validates: Requirements 3.1, 3.2, 3.3, 3.4, 3.5**

### Property 5: Crop is bounded, exact, and transactional

For any canonical image and candidate crop bounds, valid integer nonempty in-bounds bounds shall append one crop operation whose rendered result consists exactly of the corresponding half-open source rectangle; all invalid bounds shall leave crop mode, crop draft, history, and preview unchanged; cancelling any crop draft shall leave history and preview unchanged while exiting crop mode.

**Validates: Requirements 4.3, 4.4, 4.5, 4.6, 4.7**

### Property 6: Adjustment commands are clamped and commit exactly their draft

For any active document, focused adjustment kind, and valid sequence of increase/decrease commands, each command shall change the value by one only while within `[-100, 100]`, clamp at the endpoints, and render identity contribution at zero. For any focused value in the range, commit shall append exactly one matching brightness/contrast operation, reset only that focused draft to zero, and preserve the resulting committed pixels.

**Validates: Requirements 5.1, 5.2, 5.3, 5.4, 5.5, 5.6, 5.7, 5.8, 5.9, 5.10, 5.11**

### Property 7: Per-image history is reversible and branch-safe

For any image document and sequence of edit operations, undo followed by redo shall restore the exact prior ordered history and rendered result; undo/redo on an empty respective stack shall be a no-op; and adding a new operation after undo shall clear only that document's redo history without modifying another document's histories.

**Validates: Requirements 6.1, 6.2, 6.3, 6.4, 6.5, 6.6**

### Property 8: Shortcut resolution has platform-invariant semantics

For any defined non-repeat keyboard intent, the macOS and Linux shortcut tables shall resolve their respective modifier forms to the same semantic browsing, edit, history, or adjustment command; each single accepted raw press shall produce exactly one command and release/repeat events shall produce none.

**Validates: Requirements 8.3, 8.5, 8.6, 10.3, 10.4**

### Property 9: Capability projection is conservative and format-specific

For any combination of independent decode, encode, folder-picker, and save-picker availability, the view/export projection shall expose a selectable image, export choice, or enabled dependent operation if and only if its specific required capability is available; unavailable capabilities shall have a notice and shall not prevent unrelated portable format operations.

**Validates: Requirements 7.1, 8.1, 9.1, 9.2, 9.3, 9.4, 9.5, 9.6, 9.7, 9.8, 11.2, 11.3**

### Property 10: Shared pipeline is platform-equivalent

For any conformance image and any valid ordered sequence of shared edit operations and draft adjustments, evaluating the platform-independent Rust pipeline through macOS and Linux command tables shall produce equal dimensions, normalized orientation, crop bounds, and RGBA samples. For any such result, lossless PNG and TIFF encode/decode through the same supported codec contract shall preserve those dimensions and samples.

**Validates: Requirements 7.7, 10.1, 10.2, 10.5**

### Property 11: Export planning never permits replacement

For any active source identity and requested destination identity, export planning shall reject a target that identifies the source or any pre-existing regular file and shall otherwise create a plan that preserves the immutable source identity and result revision.

**Validates: Requirements 7.2, 7.3, 7.4, 7.5, 7.6**

## Error Handling

### Error model

```rust
pub enum ApplicationError {
    FolderEnumeration { folder: AbsolutePath, cause: SafeError },
    Decode { file_name: Utf8FileName, cause: SafeError },
    MissingActiveImage { command: CommandName },
    InvalidCrop { reason: CropValidationError },
    ExportTargetConflict { path: AbsolutePath, kind: TargetConflict },
    ExportWrite { path: AbsolutePath, cause: SafeError },
    PlatformOperation { capability: CapabilityName, cause: SafeError },
    FontInitialization { resource: PackageResourcePath, cause: SafeError },
    SettingsRead { category: SettingsDiagnosticKind },
    SettingsWrite { location: SettingsStorageLocation, cause: SafeError },
    StartupDirectory { candidate: StartupCandidateKind, cause: SafeError },
    ResourceLimit { subject: Utf8FileName, limit: ResourceLimitKind },
}
```

Errors are values returned through reducer completions, not panics. `SafeError` carries a user-facing summary and structured diagnostic category; detailed OS/codec diagnostics are available only in local debug logs. Errors are shown non-modally in the relevant workspace area and do not open a second primary window.

| Condition | UI behavior | State preservation |
|---|---|---|
| Folder enumeration failure | Error names selected folder | Retain prior `BrowsingState` |
| Selection/navigation decode failure | Error names candidate filename | Retain prior active image, histories, redo, preview, and collection |
| No active image for edit/adjust/crop | Error names attempted command | Retain collection, all histories/redo, and preview; do not queue command |
| Invalid crop confirmation | Show validation reason near crop controls | Retain crop mode/draft/history/preview |
| Missing dialog, HEIC decoder/encoder, or other optional dependency | Availability message and disabled dependent controls | Window and unrelated controls stay usable |
| Bundled font missing, unreadable, malformed, or rejected during registration | Native-safe startup availability error identifying the font resource category | Do not create the normal editing workspace or accept its commands; do not render Required_Text as missing-glyph boxes |
| Existing/source export path or create-new race | Error names target path | Do not open/replace target; preserve source and editor state |
| Encode/write failure | Error names target path | Preserve source, histories, redo, preview; best-effort remove only a file newly created by this failed attempt |
| Worker panic/unexpected failure | Contain at worker boundary, show generic error, record diagnostic | Retain last committed state and remain responsive |

### Degraded mode

Capability detection produces degraded mode rather than a binary supported/unsupported application state. The primary window always starts on macOS/Linux even if every optional runtime dependency is absent. Examples:

- With HEIC unavailable, `.heic` direct files are identified in the image-list area with a decoding-unavailable message; JPEG/PNG/TIFF browsing/editing/export remains available.
- With a save dialog unavailable, export is disabled with a platform-capability message; open-folder and editing remain usable.
- With folder selection unavailable, open-folder is disabled; the workspace still displays a clear availability message and does not crash.
- With a specific encoder unavailable, it is absent from export choices. JPEG/PNG/TIFF export never depends on HEIC capability.

The app does not auto-install packages, prompt the user to install dependencies during a session, or fall back to destructive behavior. Availability state is immutable for a session except that a failed actual dialog invocation can downgrade the corresponding capability and disable its dependent operation for the remainder of the session.

## Testing Strategy

### Test layers

| Layer | Scope | Tools/approach |
|---|---|---|
| Core unit tests | Exact mappings, representative state transitions, error messages, resource limits | Rust `cargo test` |
| Property tests | All 20 numbered design properties against pure core/adapters | `proptest`, at least 100 cases/property |
| Codec integration tests | Real fixture decode/encode, malformed files, PNG/TIFF equivalence, JPEG/HEIC tolerance | Temp directories and fixed fixtures; optional HEIC suite gated by detected capability |
| Desktop integration tests | One-window startup, visible controls, keyboard routing once, crop overlay coordinate conversion | eframe test harness / accessibility tree where supported |
| Font configuration tests | Bundled font discovery, required glyph coverage, `egui::FontDefinitions` priority/fallback construction, and registration failures | Pure unit tests with packaged-fixture bytes and injected resource/registration failures |
| Platform integration tests | rfd folder/save dialogs, actual filesystem identity/no-overwrite behavior, exact settings paths, bounded settings reads, and atomic replacement | macOS and Linux CI runners; injected filesystem failures; use manual/guarded tests where portal interaction cannot be automated |
| Startup desktop integration tests | Persisted-folder/cwd priority, empty workspace, first-image activation/failure, stale completion rejection, and sort controls | Deterministic fake platform/worker adapters plus focused hosted-platform cases |
| Packaging smoke tests | Install/start package, capability message with missing optional runtime dependency | Per-target package installation image/VM |
| Visual regression tests | Workspace layout, disabled-control messaging, macOS/Linux shortcut labels | Deterministic screenshot snapshots per platform |

`proptest` is selected rather than a custom generator. Every property test runs a minimum of 100 cases and begins with a traceability comment in this exact form:

```rust
// Feature: macos-image-editor, Property 4: Geometric operations preserve their specified pixel mapping
```

Each numbered design property is implemented by one property-based test function; arbitrary generators may generate a complete operation sequence within that test. Failures retain the minimized seed/case as a regression fixture.

### Property generators and oracles

- Generate small, nonzero `CanonicalImage` values with distinct RGBA16 pixels, including asymmetric dimensions, to expose pixel-coordinate mistakes.
- Generate valid and invalid `CropRect` values independently; include zero-area, reversed, negative-at-construction, and out-of-bounds cases through an untrusted crop-input representation.
- Generate operation sequences constrained by current dimensions, plus histories and independent two-document scenarios.
- Generate filenames/paths as valid UTF-8 byte strings and directory entries with file kind/format/capability permutations. A simple reference sort is the oracle for collection order.
- Generate command streams and compare reducer history state against a compact reference stack model.
- Generate shortcut intents rather than native key events; run them through both platform key tables and compare semantic commands.
- Generate capability truth tables (decode/encode/dialog) and verify enabled controls/export formats against a small declarative requirement matrix.
- Generate conformance operation sequences once and execute the same core build/test binary in macOS and Linux CI, then compare serialized deterministic result hashes and PNG/TIFF sample data as artifacts.
- Generate valid AppSettings values plus absent, oversized, malformed, unsupported-version, and semantically invalid byte streams; use canonical JSON encode/decode equivalence and the default/diagnostic matrix as the oracle.
- Generate collection entries with duplicate primary values, missing metadata, and permuted input order; compare all six sort modes against a simple reference total comparator and assert ImageId-associated state survives resorting.
- Generate persisted-folder/working-directory availability and enumeration outcome trees plus matching/stale request and collection revisions; compare startup effects and committed state against a compact priority state-machine model.

### Example, edge, integration, and smoke coverage by requirement

The following testability classification covers every acceptance criterion. `P` means a property above; `E` an example/unit or UI test; `I` a real integration test; `S` a packaging/startup smoke test; `X` an explicit error/edge test. These classifications avoid attempting PBT against OS services or UI aesthetics.

| Requirement | Acceptance criteria classification and coverage |
|---|---|
| 1 | 1.1 I (native single-folder dialog configuration); 1.2 P1 + E (direct-file enumeration); 1.3 X (enumeration failure retains state); 1.4 P1 + E (entry names); 1.5–1.7 P2 + I (decode-before-activation and failure). |
| 2 | 2.1–2.9 P2/P3 + E (candidate selection/decode transaction); 2.10–2.13 P3 + E (boundary, empty, and no-active cases). |
| 3 | 3.1–3.5 P4; 3.6 X (no-active error and no deferred execution). |
| 4 | 4.1–4.2 E/X (mode entry/no-active); 4.3–4.7 P5 + E (source-coordinate overlay, valid/invalid/cancel behavior). |
| 5 | 5.1–5.11 P6 + E (focused control presentation and preview revision); 5.12 X (no-active command preservation). |
| 6 | 6.1–6.6 P7 + E (specific undo/redo examples and no-active behavior). |
| 7 | 7.1 I (single-path native save dialog and choices); 7.2–7.6 P11 + I (create-new, file identity, source byte preservation, write failures); 7.7 P10 + I (reopen actual export and compare format-appropriate output). |
| 8 | 8.1 E + visual regression (one-window control visibility); 8.2 E (complete filename); 8.3 P8 + desktop I (one command per key press); 8.4 S (exactly one window); 8.5–8.6 P8 + visual regression (platform label names); 8.7–8.8 E + headless/visual regression (Chinese UI, filename, and notice glyph rendering plus font priority/fallback registration); 8.9 X/S (safe startup error and no editable workspace after font bootstrap failure). |
| 9 | 9.1 S/I (startup probes before request); 9.2 P9 + I; 9.3–9.8 P1/P9 + E/I (notices, filtering, disabled controls, portable independence). |
| 10 | 10.1–10.2 P10 + cross-platform I; 10.3–10.4 P8; 10.5 S/I (shared capabilities available on both targets). |
| 11 | 11.1 S (package manifests/dependency documentation); 11.2–11.3 P9 + S/I (degraded/available operation projection); 11.4–11.5 I (actual platform choosers); 11.6–11.7 S/I (package font resource/license metadata and unreadable-resource startup failure). |
| 12 | 12.1 P13 + I (ordered source discovery and partial overrides); 12.2 P12 + E (multi-binding parser/formatter round trip); 12.3 E (default-table fixture); 12.4–12.5 P13/P14 + E (read, parse, validation, collision diagnostics, and fallback); 12.6–12.9 P15 + E (fit, zoom, pan, and navigation aliases); 12.10 I (full-screen request and failure retention); 12.11–12.12 P14 + E/visual regression (effective labels, help groups, and text focus); 12.13 E (no-active view/navigation no-op). |
| 13 | 13.1 I (exact macOS/Linux settings-path resolution); 13.2–13.4 P16 + E/X (valid round trip, absent/default, bounded malformed/oversized/version fallback); 13.5–13.7 P17 + I (complete-value update, atomic replacement, failure preservation, selected-folder persistence); 13.8–13.12 P19 + E/I (candidate priority, metadata/enumeration failure fallback, cwd failure empty workspace); 13.13 P18 + E (startup ordering); 13.14–13.16 P20 + E/I (single first-item activation, success/failure transaction, no skip); 13.17 P19 + E (empty collection); 13.18 P20 + E (stale completion discard); 13.19–13.21 P18 + E (state-preserving resort, deterministic ties, missing metadata last). |

### CI matrix and acceptance gates

CI will run a pinned Rust toolchain on `aarch64-apple-darwin` and `x86_64-unknown-linux-gnu` (plus an additional macOS architecture build when release policy requires it). Both targets run core unit/property tests. Headless UI tests use the renderer's supported headless/screenshot configuration; native dialog tests run only on hosted runners with their required desktop/portal capability and are reported separately from pure-core tests.

Required gates before release:

1. Formatting, clippy, core unit tests, and all 20 property tests pass with at least 100 generated cases each on both platforms.
2. Cross-platform conformance fixtures produce equal deterministic pipeline artifacts on both platforms.
3. PNG/TIFF export-reopen tests verify exact width, height, and RGBA samples. JPEG/HEIC tests verify dimensions, orientation/crop semantics, and successful decode rather than exact lossy samples.
4. macOS and Linux packaging smoke tests start exactly one window with optional HEIC and dialog dependencies both present and intentionally absent.
5. Integration tests demonstrate that source files and existing target files remain byte-identical after every rejected or failed export path.
6. Font configuration unit tests verify required glyph coverage and `egui::FontDefinitions` priority/fallback setup; headless or visual-regression tests render Chinese UI text, a Chinese filename, and a Chinese notice without missing-glyph boxes; macOS and Linux package smoke tests verify the packaged font resource is present, readable, license-recorded, and permits startup.
7. Platform integration tests verify exact macOS/Linux settings locations, the 1 MiB read boundary, complete-value sibling-temp flush and atomic replacement, and preservation of the last complete file under injected failures.
8. Core and desktop startup suites verify persisted-folder priority, working-directory fallback and acquisition failure, empty collections, first-image success/failure without skip, in-session state-preserving resort, and rejection of stale enumeration/decode completions.

## Packaging and Distribution Considerations

The application will ship separate, target-specific installable packages that declare their platform and runtime capability providers. Every such package must also carry the mandatory `resources/fonts/NotoSansCJKsc-Regular.otf` Bundled_Font_Resource (or an approved equivalent), its license notice, and a resource-path entry; no supported target relies on a user-installed font for Required_Text.

- **macOS:** notarized/signed `.app`/DMG or `.pkg` containing the native Rust binary, the Bundled_Font_Resource in the app resource directory, and bundled codec libraries where license-compatible. The package manifest records the font resource path/license and whether HEIC decode/encode is bundled. The native dialog adapter uses the macOS dialog backend. Bundle signing/notarization and universal-binary assembly are release-pipeline concerns, not shared-core behavior.
- **Linux:** a native package plus a portable package strategy (for example, AppImage/Flatpak) selected during release engineering. Each artifact includes the Bundled_Font_Resource at the application-relative resource path and its license record. Package metadata declares the rfd backend choice. XDG Portal packages declare a portal backend/runtime as optional; GTK packages declare the corresponding shared library dependency. `libheif` and its codec plugins are explicitly identified as optional runtime dependencies when not bundled.
- **Capability manifest:** each package includes a machine-readable `capabilities.json` and human-readable release note that state target triple/platform, compiled portable codecs, optional HEIC provider expectations, dialog backend expectations, bundled-font resource path, and bundled-font license. The application does not trust this manifest as runtime truth; it still reads and registers the font resource and probes its environment at startup.
- **Reproducibility:** build from a locked dependency graph, record Rust toolchain and target, record the bundled font name/version/license/checksum, separate compile-time features (`portable-codecs`, `heic`, `xdg-portal`, `gtk`) from runtime availability, and generate SBOM/license data for bundled native libraries and the font resource.

Packaging must never turn an optional dependency into an undisclosed startup prerequisite. If a package installs without a portal backend or HEIC codec plugin, the application still opens its one primary window and projects that fact as disabled capability-aware behavior. If its mandatory Bundled_Font_Resource cannot be read or registered, it must instead follow the visible Startup_Availability_Error path and must not open a normal workspace with missing-glyph text.


## Configurable Keybindings and Image Viewing Extension

### Overview and design decisions

The fixed `ShortcutResolver` table becomes the built-in `Keybinding_Configuration`, not a second competing shortcut path. A startup `KeybindingService` discovers four layers in descending priority: the optional explicit CLI `--keybindings <path>`, project `.yampixr/keybindings.toml` relative to the process project root, the platform user path (`~/Library/Application Support/yampixr/keybindings.toml` on macOS, `$XDG_CONFIG_HOME/yampixr/keybindings.toml` on Linux), and the compiled built-in default. The CLI layer is consulted only when the argument is supplied; absent project and user files are normal absence states without a diagnostic.

Every configuration layer is parsed independently and is a partial declaration: it changes only its declared actions. The merge is deterministic. Candidate declarations are considered by layer precedence. Within a layer, every action involved in a duplicate normalized gesture is rejected as a group and receives a diagnostic; no lexical or hash-map iteration chooses a winner. Across layers, an already accepted higher-layer gesture blocks a lower-layer candidate action, which then retains its lower candidate if non-conflicting or becomes unbound with a diagnostic. Thus each accepted gesture selects exactly one action, unrelated valid actions remain available, and a bad high-priority declaration does not erase a valid lower declaration.

The core has no filesystem, environment-variable, or eframe dependency. It accepts ordered source inputs and parsed text, returns an immutable `EffectiveKeybindingMap` plus diagnostics, and owns the view reducer. The desktop adapter obtains paths and CLI/environment values, reads files, forwards safe read errors, uses the map for event routing and labels, and delegates the actual window full-screen side effect to eframe only after the core emits `ToggleFullscreen`.

### Components and interfaces

```rust
pub enum KeybindingAction {
    FitToWindow, ZoomActual, Zoom200, ZoomIn, ZoomOut,
    PanLeft, PanDown, PanUp, PanRight,
    PreviousImage, NextImage, FirstImage, LastImage, ToggleFullscreen,
    FlipHorizontal, FlipVertical, RotateClockwise90, RotateCounterclockwise90,
    EnterCrop, FocusBrightness, FocusContrast, CommitAdjustment,
    Undo, Redo, IncreaseAdjustment, DecreaseAdjustment,
}

pub struct KeybindingGesture {
    pub key: ShortcutKey,
    pub modifiers: KeyModifiers,
}

pub struct EffectiveKeybindingMap {
    by_gesture: BTreeMap<KeybindingGesture, KeybindingAction>,
    by_action: BTreeMap<KeybindingAction, Vec<KeybindingGesture>>,
}

pub trait KeybindingSourceReader {
    fn read(&self, source: &KeybindingSource) -> Result<Option<String>, KeybindingReadError>;
}

pub fn resolve_keybindings(
    platform: RuntimePlatform,
    layers: &[KeybindingLayerInput],
) -> KeybindingResolution;

pub fn parse_keybinding_configuration(
    text: &str,
    source: KeybindingSource,
) -> Result<PartialKeybindingConfiguration, Vec<KeybindingDiagnostic>>;

pub fn format_keybinding_configuration(
    configuration: &ValidatedKeybindingConfiguration) -> String;
```

`Keybinding_Configuration` uses deterministic TOML tables. `[bindings]` is platform-neutral; `[macos.bindings]` and `[linux.bindings]` selectively replace the corresponding global action declaration on that platform. Each value is an array, so aliases such as `next_image = ["Right", "Down", "PageDown", "Space"]` are first-class. The parser canonicalizes key spelling and modifier order before collision detection, accepts only keys exposed by `ShortcutKey`, and accepts `Command`/`Option` only on macOS and `Control`/`Alt` only on Linux except where a platform-specific table makes the combination valid for that platform. The formatter sorts action names and gestures, writes canonical modifier/key spellings, and emits TOML that the parser accepts into an equivalent validated configuration.

The built-in configuration is explicit and test-fixture-visible:

| Group | Action | Built-in binding(s) |
|---|---|---|
| 缩放与视图 | `fit_to_window`, `zoom_actual`, `zoom_200`, `zoom_in`, `zoom_out` | `0`; `1`; `2`; `+`, `=`; `-` |
| 缩放与视图 | `pan_left`, `pan_down`, `pan_up`, `pan_right` | `H`; `J`; `K`; `L` |
| 浏览 | `previous_image`, `next_image`, `first_image`, `last_image` | Left, Up, PageUp; Right, Down, PageDown, Space; Home; End |
| 文件 | `toggle_fullscreen` | Linux: F11; macOS: F11, Control+Command+F |
| 编辑 | 翻转、旋转、裁剪、调整、提交、撤销/重做 | 既有 F/Shift+F、R/Shift+R、C、B、D、Enter，以及平台正确的撤销/重做和调整键 |

`ShortcutResolver` becomes a thin pure lookup over `EffectiveKeybindingMap`. It resolves only a pressed, non-repeat event that has not been consumed by `Text_Input_Focus`; the desktop adapter must mark every key event consumed by an editable text widget before invoking the resolver. One event results in either zero or one `EditorCommand`, never multiple commands. `shortcut_label` and the command palette derive labels from `by_action`, formatting macOS modifiers as `Command`/`Option` and Linux modifiers as `Control`/`Alt`.

```rust
pub struct ViewState {
    pub zoom: ZoomMode,
    pub manual_scale: RationalScale,
    pub canvas_offset: LogicalVector,
    pub preview_size: LogicalSize,
}

pub enum ZoomMode { FitToWindow, Manual }

pub enum EditorCommand {
    // existing variants
    SetFitToWindow,
    SetManualZoom { percent: u16 },
    ZoomByStep { direction: ZoomDirection },
    PanCanvas { direction: PanDirection },
    ToggleFullscreen,
}
```

`ViewReducer` keeps fit-to-window scale equal to the smaller axis ratio between the rendered image and the available Preview size. `1` and `2` set manual scale to exactly 100% and 200%. `+`/`=` multiply manual scale by `1.25`; `-` divides by `1.25`; each result clamps to 25%–800%. H/J/K/L modifies only the relevant canvas offset by `Pan_Step` when the scaled image exceeds the Preview on that axis. Every scale, viewport-size, image-change, and pan operation clamps offsets to the image bounds; a pan with no scrollable extent is a no-op. Selection/navigation aliases route to the existing decode-before-activation commands, so Left/Up/PageUp are identical `PreviousImage` intents and Right/Down/PageDown/Space are identical `NextImage` intents.

### Data models and diagnostics

```rust
pub enum KeybindingSource {
    ExplicitCli(AbsolutePath),
    Project(AbsolutePath),
    User(AbsolutePath),
    BuiltIn,
}

pub struct KeybindingDiagnostic {
    pub source: KeybindingSource,
    pub action: Option<KeybindingActionName>,
    pub gesture: Option<String>,
    pub category: KeybindingDiagnosticKind,
    pub safe_message: String,
}

pub enum KeybindingDiagnosticKind {
    ReadFailed, InvalidToml, UnknownAction, UnknownKey,
    IllegalModifier, DuplicateGesture, BlockedByHigherPriority,
}
```

Diagnostics contain a normalized source description, line/column where TOML supplies one, and an actionable reason; they never include arbitrary configuration contents, stack traces, or environment values. The source reader distinguishes a missing optional project/user file from read failure. An explicitly named CLI file that cannot be read is diagnosed, then lower layers continue. The desktop view exposes diagnostics in the existing non-modal notice area.

### Correctness properties

The feature contains pure parsing, merging, routing, and viewport transformation logic with large input domains, so PBT is appropriate for those core layers. Native environment path discovery, filesystem permission failures, eframe full-screen application, command-palette rendering, and actual focus delivery remain example/integration tests.

### Property 12: TOML bindings round-trip and preserve aliases

For any validated keybinding configuration containing supported action names and one or more non-conflicting gestures per action, formatting then parsing the configuration shall produce an equivalent platform-neutral and platform-specific declaration with the same ordered action-to-gesture sets.

**Validates: Requirements 12.2**

### Property 13: Layered partial overrides retain valid lower declarations

For any ordered set of keybinding layers and platform, resolving the layers shall use the highest-priority valid declaration for each action, retain lower-layer declarations for actions not validly declared above, and produce the same map and diagnostics regardless of input-map insertion order.

**Validates: Requirements 12.1, 12.3, 12.4, 12.5**

### Property 14: Effective bindings are exclusive and text-safe

For any parsed keybinding layers, every gesture in the resulting EffectiveKeybindingMap shall resolve to at most one action; any duplicate or unsupported gesture shall resolve to no command and a diagnostic; and every event marked as consumed by a text control shall resolve to no command.

**Validates: Requirements 12.4, 12.5, 12.12**

### Property 15: View transforms remain bounded and navigation aliases are semantically equivalent

For any nonzero image size, Preview size, initial ViewState, and sequence of zoom or pan commands, the resulting scale shall remain in the configured range and every canvas offset shall remain within the image bounds; for any navigation alias configured for previous or next image, resolving the alias shall emit the same corresponding navigation command.

**Validates: Requirements 12.6, 12.8, 12.9**

### Property 16: Settings decoding is bounded, round-trippable, and fail-safe

For any valid AppSettings value, canonical JSON encoding followed by bounded decoding shall produce an equivalent version, sort setting, and optional last-successful folder; for any absent input the effective sort shall be `full_file_name/ascending` without a diagnostic, and for any oversized, malformed, unsupported-version, or semantically invalid input the effective sort shall be `full_file_name/ascending` with exactly one safe diagnostic category.

**Validates: Requirements 13.2, 13.3, 13.4**

### Property 17: Settings updates preserve unrelated values

For any valid AppSettings value and any valid new SortSettings or successfully enumerated selected folder, producing the complete replacement settings value shall change only the requested field, preserve every unrelated field, and retain the selected in-session sort value when persistence reports failure.

**Validates: Requirements 13.5, 13.6, 13.7**

### Property 18: Effective image ordering is total, deterministic, and identity-preserving

For any finite collection of supported images and any valid SortSettings, sorting shall compare the selected available field in the requested direction, place missing modified-time or file-size values after present values, break every primary tie by filename UTF-8 bytes and then absolute-path UTF-8 bytes ascending, and produce the same order for every input permutation; applying that order to an existing collection shall preserve active image, per-image histories, redo stacks, drafts, preview, and view state by ImageId.

**Validates: Requirements 13.13, 13.19, 13.20, 13.21**

### Property 19: Startup candidate planning follows priority and terminates safely

For any optional last-successful folder, startup-working-directory result, and sequence of accessibility or enumeration outcomes, startup planning shall attempt each distinct candidate at most once in persisted-folder-then-working-directory order, select the first successfully enumerated candidate, and otherwise terminate in an operable empty workspace; any successful empty collection shall emit no startup activation plan.

**Validates: Requirements 13.8, 13.9, 13.10, 13.11, 13.12, 13.17**

### Property 20: Startup activation is first-item atomic and revision-safe

For any ordered nonempty startup collection, the planner shall emit exactly one activation plan for the first item; a matching successful decode shall atomically activate and preview only that item, a matching decode failure shall retain no startup-selected active image and emit no later automatic decode, and any enumeration or activation completion with a stale startup-request or collection revision shall leave all editor state unchanged.

**Validates: Requirements 13.14, 13.15, 13.16, 13.18**

### Error handling additions

| Settings/startup condition | UI behavior | Effective behavior |
|---|---|---|
| Settings file absent | No settings error notice | Use `full_file_name/ascending`; continue startup candidate planning |
| Settings unreadable, over 1 MiB, malformed, unsupported, or invalid | Safe non-modal settings diagnostic names only the failure category | Ignore the complete file; use defaults; continue startup |
| Atomic settings replacement fails | Safe non-modal persistence diagnostic | Keep selected sort/folder state for the session and preserve the last complete file |
| Persisted folder missing, inaccessible, not a directory, or not enumerable | Startup directory diagnostic identifies persisted-folder category | Attempt captured startup working directory exactly once |
| Startup working directory unavailable or cannot be enumerated | Startup directory diagnostic | Present operable empty workspace; retain capability-aware open-folder behavior |
| Startup directory is valid but contains no supported images | Empty-collection message | Emit no activation decode |
| First sorted startup image cannot decode | Error names the candidate filename | Keep collection, leave startup active image unset, and do not skip to another image |
| Startup completion has stale request/collection revision | No user-visible state regression | Discard completion without changing editor, preview, history, or view state |

| Keybinding condition | UI behavior | Effective behavior |
|---|---|---|
| Explicit CLI file cannot be read or TOML is invalid | Notice identifies layer/path and safe reason | Continue with project, user, and built-in candidates |
| Optional project/user file is absent | No error notice | Continue to the next layer |
| Unknown action/key or illegal platform modifier | Notice identifies declaration | Reject only that action declaration and retain fallback candidates |
| Duplicate gesture in a layer or collision with accepted higher mapping | Notice identifies gesture and conflicting action(s) | Never dispatch more than one command; preserve non-conflicting actions and lower fallback where available |
| Full-screen adapter rejects a toggle | Non-modal platform error | Preserve key map and editor/view state; retain the current window mode |

### Testing strategy additions

Use pinned `toml` and `serde` crates for parsing/formatting rather than a custom TOML grammar. Add one `proptest` function per Properties 12–15 with at least 100 cases and the existing traceability comment format. Unit tests cover known default tables, macOS/Linux label spellings, malformed TOML line/column diagnostics, invalid modifiers, explicit CLI read failures, and exact 100%/200%/fit calculations. Desktop tests verify that toolbar buttons and the command palette display the effective bindings in the four groups 浏览、缩放与视图、编辑、文件; focus an editable text widget and assert configured printable shortcuts do not execute; mock the full-screen adapter on both platforms; and verify no-active, collection-boundary, and unzoomed-pan cases are no-ops. Hosted macOS/Linux integration tests validate user-path selection, CLI/project/user/built-in precedence, and F11 versus Control+Command+F full-screen variants.
### Startup restoration testing additions

Add one `proptest` function for each of Properties 16–20, with at least 100 cases and the existing traceability-comment format. Property 16 uses valid settings plus absent, malformed, oversized, unsupported-version, and invalid-semantic byte streams; Property 17 exercises complete-value update reducers and injected persistence outcomes; Property 18 compares every sort mode and input permutation to a simple reference comparator; Properties 19–20 compare planner effects and committed state to a compact startup state-machine model with matching and stale revisions.

Example and integration tests remain necessary around side effects. Core examples cover exact defaults, empty collections, first-image decode failure without skip, and state preservation after resorting. Platform tests cover exact macOS/Linux locations, XDG fallback, the 1 MiB-plus-sentinel read bound, sibling temporary creation, flush/sync/atomic replacement, and injected failure preservation. Desktop tests use deterministic fake workers for persisted-folder priority, working-directory fallback/acquisition failure, superseding user actions, and stale enumeration/decode completion rejection; hosted macOS/Linux cases verify the real path and filesystem adapters.