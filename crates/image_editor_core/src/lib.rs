//! Platform-independent domain contracts for the Image Editor.
//!
//! This crate deliberately has no UI, OS, codec, or asynchronous-runtime
//! dependencies. Its constructors make the invariant-bearing values used at
//! every adapter boundary impossible to construct accidentally.

pub mod keybindings;

pub use keybindings::{
    KeybindingLayerInput, KeybindingParseResult, KeybindingResolution,
    PartialKeybindingConfiguration, ValidatedKeybindingConfiguration,
    format_keybinding_configuration, parse_keybinding_configuration, resolve_keybindings,
};

use std::{collections::BTreeMap, fmt, path::Path};

/// The result type shared by Image Editor crates.
pub type Result<T> = std::result::Result<T, ApplicationError>;

/// A stable category for diagnostics without exposing platform internals.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ErrorCategory {
    Invariant,
    OptionalDependency,
    PlatformIntegration,
    PortableCodec,
    FileSystem,
    ResourceLimit,
}

/// A user-safe error summary suitable for an in-window notice.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SafeError {
    category: ErrorCategory,
    summary: String,
}

impl SafeError {
    /// Creates an error from a vetted, user-safe summary.
    ///
    /// Control characters are replaced so a platform or codec diagnostic cannot
    /// inject additional UI lines or terminal control sequences into a notice.
    pub fn new(category: ErrorCategory, summary: impl Into<String>) -> Self {
        let summary = summary
            .into()
            .chars()
            .map(|character| {
                if character.is_control() {
                    ' '
                } else {
                    character
                }
            })
            .collect();
        Self { category, summary }
    }

    pub fn category(&self) -> ErrorCategory {
        self.category
    }

    pub fn summary(&self) -> &str {
        &self.summary
    }
}

impl fmt::Display for SafeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.summary)
    }
}

impl std::error::Error for SafeError {}

/// Image formats recognized by the shared domain.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub enum ImageFormat {
    Jpeg,
    Png,
    Tiff,
    Heic,
}

impl ImageFormat {
    /// Returns the candidate format associated with a supported extension.
    pub fn from_extension(extension: &str) -> Option<Self> {
        if extension.eq_ignore_ascii_case("jpg") || extension.eq_ignore_ascii_case("jpeg") {
            Some(Self::Jpeg)
        } else if extension.eq_ignore_ascii_case("png") {
            Some(Self::Png)
        } else if extension.eq_ignore_ascii_case("tif") || extension.eq_ignore_ascii_case("tiff") {
            Some(Self::Tiff)
        } else if extension.eq_ignore_ascii_case("heic") {
            Some(Self::Heic)
        } else {
            None
        }
    }

    pub const fn display_name(self) -> &'static str {
        match self {
            Self::Jpeg => "JPEG",
            Self::Png => "PNG",
            Self::Tiff => "TIFF",
            Self::Heic => "HEIC",
        }
    }
}

/// A diagnosed capability state. Missing capability is always explicit.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Availability {
    Available,
    Unavailable { reason: AvailabilityReason },
}

impl Availability {
    pub const fn is_available(&self) -> bool {
        matches!(self, Self::Available)
    }
}

/// A safe explanation for an unavailable capability.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AvailabilityReason(SafeError);

impl AvailabilityReason {
    pub fn new(summary: impl Into<String>) -> Self {
        Self(SafeError::new(ErrorCategory::OptionalDependency, summary))
    }

    pub fn safe_error(&self) -> &SafeError {
        &self.0
    }

    pub fn summary(&self) -> &str {
        self.0.summary()
    }
}

/// The component supplying a codec capability.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CodecProvider {
    PortableRust,
    Libheif,
}

/// Independently probed decode and encode capability for one image format.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FormatCapability {
    pub decode: Availability,
    pub encode: Availability,
    pub provider: Option<CodecProvider>,
}

impl FormatCapability {
    pub fn new(
        decode: Availability,
        encode: Availability,
        provider: Option<CodecProvider>,
    ) -> Self {
        Self {
            decode,
            encode,
            provider,
        }
    }

    pub fn can_decode(&self) -> bool {
        self.decode.is_available()
    }

    pub fn can_encode(&self) -> bool {
        self.encode.is_available()
    }
}

/// All formats whose decode and encode availability must be recorded at startup.
pub const SUPPORTED_IMAGE_FORMATS: [ImageFormat; 4] = [
    ImageFormat::Jpeg,
    ImageFormat::Png,
    ImageFormat::Tiff,
    ImageFormat::Heic,
];

/// The runtime state of a platform operation such as a native file picker.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PlatformCapability {
    availability: Availability,
    backend: Option<String>,
}

impl PlatformCapability {
    pub fn available(backend: impl Into<String>) -> Self {
        Self {
            availability: Availability::Available,
            backend: Some(backend.into()),
        }
    }

    pub fn unavailable(reason: impl Into<String>) -> Self {
        Self {
            availability: Availability::Unavailable {
                reason: AvailabilityReason::new(reason),
            },
            backend: None,
        }
    }

    pub const fn availability(&self) -> &Availability {
        &self.availability
    }

    pub const fn is_available(&self) -> bool {
        self.availability.is_available()
    }

    pub fn backend(&self) -> Option<&str> {
        self.backend.as_deref()
    }
}

/// Immutable, complete startup capability state used to gate dependent commands.
///
/// Construction fills every defined format even when a probe omits one, making an
/// absent probe result an explicit unavailable state instead of an implicit
/// fallback. The snapshot has no mutation API and is intended to be shared with
/// the command router for the lifetime of an application session.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CapabilitySnapshot {
    formats: BTreeMap<ImageFormat, FormatCapability>,
    folder_picker: PlatformCapability,
    save_picker: PlatformCapability,
    diagnostics: Vec<VisibleNotice>,
}

impl CapabilitySnapshot {
    pub fn new(
        mut formats: BTreeMap<ImageFormat, FormatCapability>,
        folder_picker: PlatformCapability,
        save_picker: PlatformCapability,
    ) -> Self {
        for format in SUPPORTED_IMAGE_FORMATS {
            formats.entry(format).or_insert_with(|| {
                let reason = AvailabilityReason::new(format!(
                    "{} capability probe did not report a result",
                    format.display_name()
                ));
                FormatCapability::new(
                    Availability::Unavailable {
                        reason: reason.clone(),
                    },
                    Availability::Unavailable { reason },
                    None,
                )
            });
        }

        let diagnostics = Self::diagnostics_for(&formats, &folder_picker, &save_picker);
        Self {
            formats,
            folder_picker,
            save_picker,
            diagnostics,
        }
    }

    pub fn format(&self, format: ImageFormat) -> &FormatCapability {
        // `new` inserts every supported format, and ImageFormat is closed.
        &self.formats[&format]
    }

    pub fn formats(&self) -> &BTreeMap<ImageFormat, FormatCapability> {
        &self.formats
    }

    pub const fn folder_picker(&self) -> &PlatformCapability {
        &self.folder_picker
    }

    pub const fn save_picker(&self) -> &PlatformCapability {
        &self.save_picker
    }

    pub fn diagnostics(&self) -> &[VisibleNotice] {
        &self.diagnostics
    }

    fn diagnostics_for(
        formats: &BTreeMap<ImageFormat, FormatCapability>,
        folder_picker: &PlatformCapability,
        save_picker: &PlatformCapability,
    ) -> Vec<VisibleNotice> {
        let mut diagnostics = Vec::new();
        for format in SUPPORTED_IMAGE_FORMATS {
            let capability = &formats[&format];
            if let Availability::Unavailable { reason } = &capability.decode {
                diagnostics.push(VisibleNotice::new(
                    NoticeSeverity::Availability,
                    NoticeSubject::Capability(CapabilityName::FormatDecode(format)),
                    reason.safe_error().clone(),
                ));
            }
            if let Availability::Unavailable { reason } = &capability.encode {
                diagnostics.push(VisibleNotice::new(
                    NoticeSeverity::Availability,
                    NoticeSubject::Capability(CapabilityName::FormatEncode(format)),
                    reason.safe_error().clone(),
                ));
            }
        }
        for (name, capability) in [
            (CapabilityName::FolderPicker, folder_picker),
            (CapabilityName::SavePicker, save_picker),
        ] {
            if let Availability::Unavailable { reason } = capability.availability() {
                diagnostics.push(VisibleNotice::new(
                    NoticeSeverity::Availability,
                    NoticeSubject::Capability(name),
                    reason.safe_error().clone(),
                ));
            }
        }
        diagnostics
    }
}

/// A capability name suitable for a user-visible subject.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CapabilityName {
    FormatDecode(ImageFormat),
    FormatEncode(ImageFormat),
    FolderPicker,
    SavePicker,
}

impl fmt::Display for CapabilityName {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::FormatDecode(format) => write!(formatter, "{} decoding", format.display_name()),
            Self::FormatEncode(format) => write!(formatter, "{} encoding", format.display_name()),
            Self::FolderPicker => formatter.write_str("folder picker"),
            Self::SavePicker => formatter.write_str("save picker"),
        }
    }
}

/// A UTF-8 absolute local path. It deliberately stores no lossy path form.
#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct AbsolutePath(String);

impl AbsolutePath {
    pub fn new(path: impl Into<String>) -> std::result::Result<Self, PathValidationError> {
        let path = path.into();
        if path.is_empty() {
            return Err(PathValidationError::Empty);
        }
        if path.contains('\0') {
            return Err(PathValidationError::ContainsNul);
        }
        if !Path::new(&path).is_absolute() {
            return Err(PathValidationError::NotAbsolute);
        }
        Ok(Self(path))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for AbsolutePath {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// Why a UTF-8 path could not become a domain path.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PathValidationError {
    Empty,
    ContainsNul,
    NotAbsolute,
}

impl fmt::Display for PathValidationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Empty => "path must not be empty",
            Self::ContainsNul => "path must not contain a NUL byte",
            Self::NotAbsolute => "path must be absolute",
        })
    }
}

/// A single UTF-8 filename, with no path separators or special directory names.
#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct Utf8FileName(String);

impl Utf8FileName {
    pub fn new(name: impl Into<String>) -> std::result::Result<Self, FileNameValidationError> {
        let name = name.into();
        if name.is_empty() {
            return Err(FileNameValidationError::Empty);
        }
        if name == "." || name == ".." {
            return Err(FileNameValidationError::SpecialDirectory);
        }
        if name.contains(['/', '\\', '\0']) {
            return Err(FileNameValidationError::ContainsSeparatorOrNul);
        }
        Ok(Self(name))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn format(&self) -> Option<ImageFormat> {
        self.0
            .rsplit_once('.')
            .and_then(|(_, extension)| ImageFormat::from_extension(extension))
    }
}

impl fmt::Display for Utf8FileName {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// Why a filename cannot be represented as a collection filename.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FileNameValidationError {
    Empty,
    SpecialDirectory,
    ContainsSeparatorOrNul,
}

impl fmt::Display for FileNameValidationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Empty => "file name must not be empty",
            Self::SpecialDirectory => "file name must not be a directory marker",
            Self::ContainsSeparatorOrNul => "file name must not contain a separator or NUL byte",
        })
    }
}

/// Platform-provided stable metadata for an existing file.
#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct FileIdentity(String);

impl FileIdentity {
    pub fn new(value: impl Into<String>) -> std::result::Result<Self, IdentityValidationError> {
        let value = value.into();
        if value.is_empty() {
            return Err(IdentityValidationError::Empty);
        }
        if value.contains('\0') {
            return Err(IdentityValidationError::ContainsNul);
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A source's immutable session identity. Display names are intentionally absent.
#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct SourceIdentity {
    absolute_path: AbsolutePath,
    file_identity: Option<FileIdentity>,
}

impl SourceIdentity {
    pub fn new(absolute_path: AbsolutePath, file_identity: Option<FileIdentity>) -> Self {
        Self {
            absolute_path,
            file_identity,
        }
    }

    pub fn absolute_path(&self) -> &AbsolutePath {
        &self.absolute_path
    }

    pub fn file_identity(&self) -> Option<&FileIdentity> {
        self.file_identity.as_ref()
    }
}

/// A filesystem resolution of a requested export target before a writer opens.
///
/// A missing path is eligible for an exclusive create. Existing regular files
/// are never eligible, even when the platform cannot provide file metadata.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ExportTargetResolution {
    Missing,
    ExistingRegular { identity: Option<FileIdentity> },
    ExistingOther,
}

impl ExportTargetResolution {
    pub const fn missing() -> Self {
        Self::Missing
    }

    pub const fn existing_regular(identity: Option<FileIdentity>) -> Self {
        Self::ExistingRegular { identity }
    }

    pub const fn existing_other() -> Self {
        Self::ExistingOther
    }
}

/// An immutable, validated snapshot that authorizes one export attempt.
///
/// The filesystem adapter must resolve the target before constructing this
/// value. Writer code consumes this plan rather than the unvalidated picker
/// path, so conflicts cannot reach a writer-open operation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExportPlan {
    source_identity: SourceIdentity,
    document_revision: Revision,
    target: AbsolutePath,
    format: ImageFormat,
}

impl ExportPlan {
    pub fn validate(
        source_identity: SourceIdentity,
        document_revision: Revision,
        target: AbsolutePath,
        format: ImageFormat,
        target_resolution: ExportTargetResolution,
    ) -> Result<Self> {
        let identifies_source = target == *source_identity.absolute_path()
            || matches!(
                &target_resolution,
                ExportTargetResolution::ExistingRegular {
                    identity: Some(target_identity),
                } if source_identity.file_identity() == Some(target_identity)
            );

        if identifies_source {
            return Err(ApplicationError::ExportTargetConflict {
                path: target,
                kind: TargetConflict::SourceImage,
            });
        }

        if matches!(
            target_resolution,
            ExportTargetResolution::ExistingRegular { .. }
        ) {
            return Err(ApplicationError::ExportTargetConflict {
                path: target,
                kind: TargetConflict::ExistingLocalFile,
            });
        }

        Ok(Self {
            source_identity,
            document_revision,
            target,
            format,
        })
    }

    pub fn source_identity(&self) -> &SourceIdentity {
        &self.source_identity
    }

    pub const fn document_revision(&self) -> Revision {
        self.document_revision
    }

    pub fn target(&self) -> &AbsolutePath {
        &self.target
    }

    pub const fn format(&self) -> ImageFormat {
        self.format
    }
}

/// The stable identifier used to keep document histories distinct.
pub type ImageId = SourceIdentity;

/// Why an opaque source identity is invalid.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IdentityValidationError {
    Empty,
    ContainsNul,
}

/// A selectable supported image in a direct folder collection.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CollectionEntry {
    pub id: ImageId,
    pub absolute_path: AbsolutePath,
    pub file_name: Utf8FileName,
    pub format: ImageFormat,
}

impl CollectionEntry {
    pub fn new(
        id: ImageId,
        absolute_path: AbsolutePath,
        file_name: Utf8FileName,
        format: ImageFormat,
    ) -> std::result::Result<Self, CollectionEntryError> {
        if id.absolute_path() != &absolute_path {
            return Err(CollectionEntryError::IdentityPathMismatch);
        }
        if file_name.format() != Some(format) {
            return Err(CollectionEntryError::FormatMismatch);
        }
        Ok(Self {
            id,
            absolute_path,
            file_name,
            format,
        })
    }
}

/// Collection entry validation failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CollectionEntryError {
    IdentityPathMismatch,
    FormatMismatch,
}

/// An immutable supported-image collection, ordered by filename then full path.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ImageCollection {
    entries: Vec<CollectionEntry>,
}

impl ImageCollection {
    pub fn new(mut entries: Vec<CollectionEntry>) -> Self {
        entries.sort_by(|left, right| {
            left.file_name
                .as_str()
                .as_bytes()
                .cmp(right.file_name.as_str().as_bytes())
                .then_with(|| {
                    left.absolute_path
                        .as_str()
                        .as_bytes()
                        .cmp(right.absolute_path.as_str().as_bytes())
                })
        });
        Self { entries }
    }

    pub fn entries(&self) -> &[CollectionEntry] {
        &self.entries
    }
}

/// The location of an item supplied by a directory-listing adapter.
///
/// The collection planner accepts this explicit distinction instead of walking
/// the filesystem itself, keeping recursive traversal from becoming an
/// accidental part of folder browsing.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DirectoryEntryLocation {
    Direct,
    Descendant,
}

/// The filesystem kind of an item supplied by a directory-listing adapter.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DirectoryEntryKind {
    RegularFile,
    Directory,
    Other,
}

/// One item reported by an injected folder listing.
///
/// Paths and names are already validated UTF-8 domain values, so collection
/// ordering can use their exact byte sequences without a lossy conversion.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DirectoryEntry {
    absolute_path: AbsolutePath,
    file_name: Utf8FileName,
    location: DirectoryEntryLocation,
    kind: DirectoryEntryKind,
    file_identity: Option<FileIdentity>,
}

impl DirectoryEntry {
    pub fn new(
        absolute_path: AbsolutePath,
        file_name: Utf8FileName,
        location: DirectoryEntryLocation,
        kind: DirectoryEntryKind,
        file_identity: Option<FileIdentity>,
    ) -> Self {
        Self {
            absolute_path,
            file_name,
            location,
            kind,
            file_identity,
        }
    }

    pub fn absolute_path(&self) -> &AbsolutePath {
        &self.absolute_path
    }

    pub fn file_name(&self) -> &Utf8FileName {
        &self.file_name
    }

    pub const fn location(&self) -> DirectoryEntryLocation {
        self.location
    }

    pub const fn kind(&self) -> DirectoryEntryKind {
        self.kind
    }

    pub fn file_identity(&self) -> Option<&FileIdentity> {
        self.file_identity.as_ref()
    }
}

/// A candidate image whose decoder is unavailable in the current session.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UnavailableImage {
    absolute_path: AbsolutePath,
    file_name: Utf8FileName,
    format: ImageFormat,
    reason: AvailabilityReason,
}

impl UnavailableImage {
    pub fn absolute_path(&self) -> &AbsolutePath {
        &self.absolute_path
    }

    pub fn file_name(&self) -> &Utf8FileName {
        &self.file_name
    }

    pub const fn format(&self) -> ImageFormat {
        self.format
    }

    pub fn reason(&self) -> &AvailabilityReason {
        &self.reason
    }

    /// Builds the non-blocking image-list notice required for this candidate.
    pub fn availability_notice(&self) -> VisibleNotice {
        VisibleNotice::new(
            NoticeSeverity::Availability,
            NoticeSubject::FileName(self.file_name.clone()),
            SafeError::new(
                ErrorCategory::OptionalDependency,
                format!(
                    "{} decoding is unavailable on the current platform: {}",
                    self.format.display_name(),
                    self.reason.summary()
                ),
            ),
        )
    }
}

/// The successful, not-yet-installed result of a folder enumeration.
///
/// This is deliberately separate from `BrowsingState` (added by the reducer
/// task), allowing an adapter to plan a selected folder without replacing the
/// currently browsed folder until its completion is accepted.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FolderCollectionPlan {
    source_folder: AbsolutePath,
    collection: ImageCollection,
    unavailable: Vec<UnavailableImage>,
}

impl FolderCollectionPlan {
    pub fn source_folder(&self) -> &AbsolutePath {
        &self.source_folder
    }

    pub fn collection(&self) -> &ImageCollection {
        &self.collection
    }

    pub fn unavailable(&self) -> &[UnavailableImage] {
        &self.unavailable
    }

    pub fn availability_notices(&self) -> Vec<VisibleNotice> {
        self.unavailable
            .iter()
            .map(UnavailableImage::availability_notice)
            .collect()
    }
}

/// The pure outcome supplied by an injected filesystem enumeration.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FolderEnumerationInput {
    Succeeded {
        folder: AbsolutePath,
        entries: Vec<DirectoryEntry>,
    },
    Failed {
        folder: AbsolutePath,
        cause: SafeError,
    },
}

/// A pure plan that either prepares a replacement collection or reports why
/// the prior browsing state must remain installed.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FolderEnumerationPlan {
    Ready(FolderCollectionPlan),
    Failed(ApplicationError),
}

/// Filters and prepares a direct-folder image collection without mutating any
/// browsing state.
///
/// Only direct regular files with a defined image extension are candidates.
/// Decodable candidates become collection entries; candidates whose format is
/// currently undecodable are retained as non-blocking availability notices.
pub fn plan_folder_enumeration(
    capabilities: &CapabilitySnapshot,
    input: FolderEnumerationInput,
) -> FolderEnumerationPlan {
    let FolderEnumerationInput::Succeeded { folder, entries } = input else {
        let FolderEnumerationInput::Failed { folder, cause } = input else {
            unreachable!("input was matched by the preceding pattern")
        };
        return FolderEnumerationPlan::Failed(ApplicationError::FolderEnumeration {
            folder,
            cause,
        });
    };

    let mut supported = Vec::new();
    let mut unavailable = Vec::new();

    for entry in entries {
        if entry.location != DirectoryEntryLocation::Direct
            || entry.kind != DirectoryEntryKind::RegularFile
        {
            continue;
        }

        let Some(format) = entry.file_name.format() else {
            continue;
        };

        if capabilities.format(format).can_decode() {
            let identity = SourceIdentity::new(entry.absolute_path.clone(), entry.file_identity);
            // A format was derived from the same validated filename, so this
            // constructor cannot fail unless its invariant is changed.
            let collection_entry = CollectionEntry::new(
                identity,
                entry.absolute_path,
                entry.file_name,
                format,
            )
            .expect("candidate format and source identity were derived from one directory entry");
            supported.push(collection_entry);
        } else {
            let Availability::Unavailable { reason } = &capabilities.format(format).decode else {
                unreachable!("decode availability was checked above")
            };
            unavailable.push(UnavailableImage {
                absolute_path: entry.absolute_path,
                file_name: entry.file_name,
                format,
                reason: reason.clone(),
            });
        }
    }

    FolderEnumerationPlan::Ready(FolderCollectionPlan {
        source_folder: folder,
        collection: ImageCollection::new(supported),
        unavailable,
    })
}

/// A user action whose availability depends on a platform integration or an
/// image encoder. Image selection is represented by `selectable_images`, where
/// an entry exists only while its own decoder is available.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub enum DependentOperation {
    OpenFolder,
    Export,
}

/// An operation that the workspace must render disabled, together with the
/// exact unavailable capabilities that caused the disabled state.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DisabledOperation {
    pub operation: DependentOperation,
    pub unavailable_capabilities: Vec<CapabilityName>,
}

/// Pure, session-stable data for rendering capability-aware workspace controls.
///
/// The projection preserves independent decode, encode, and dialog facts: a
/// format's decoder controls selectable entries, its encoder controls export
/// choices, and a missing dialog disables only the operation requiring it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CapabilityProjection {
    selectable_images: Vec<CollectionEntry>,
    export_formats: Vec<ImageFormat>,
    enabled_operations: Vec<DependentOperation>,
    disabled_operations: Vec<DisabledOperation>,
    availability_messages: Vec<VisibleNotice>,
}

impl CapabilityProjection {
    pub fn selectable_images(&self) -> &[CollectionEntry] {
        &self.selectable_images
    }

    pub fn export_formats(&self) -> &[ImageFormat] {
        &self.export_formats
    }

    pub fn enabled_operations(&self) -> &[DependentOperation] {
        &self.enabled_operations
    }

    pub fn disabled_operations(&self) -> &[DisabledOperation] {
        &self.disabled_operations
    }

    pub fn availability_messages(&self) -> &[VisibleNotice] {
        &self.availability_messages
    }

    pub fn is_operation_enabled(&self, operation: DependentOperation) -> bool {
        self.enabled_operations.contains(&operation)
    }
}

/// Projects independent startup capabilities and an optional folder result into
/// the selectable image entries, available export formats, operation state, and
/// non-blocking messages required by the workspace.
///
/// `folder_plan` is not installed or mutated. It is filtered again against the
/// immutable snapshot so the projection remains conservative even if callers
/// provide a plan produced before a capability downgrade.
pub fn project_capabilities(
    capabilities: &CapabilitySnapshot,
    folder_plan: Option<&FolderCollectionPlan>,
) -> CapabilityProjection {
    let selectable_images = folder_plan
        .map(|plan| {
            plan.collection()
                .entries()
                .iter()
                .filter(|entry| capabilities.format(entry.format).can_decode())
                .cloned()
                .collect()
        })
        .unwrap_or_default();

    let export_formats = SUPPORTED_IMAGE_FORMATS
        .into_iter()
        .filter(|format| capabilities.format(*format).can_encode())
        .collect::<Vec<_>>();

    let mut enabled_operations = Vec::new();
    let mut disabled_operations = Vec::new();

    if capabilities.folder_picker().is_available() {
        enabled_operations.push(DependentOperation::OpenFolder);
    } else {
        disabled_operations.push(DisabledOperation {
            operation: DependentOperation::OpenFolder,
            unavailable_capabilities: vec![CapabilityName::FolderPicker],
        });
    }

    let mut export_requirements = Vec::new();
    if !capabilities.save_picker().is_available() {
        export_requirements.push(CapabilityName::SavePicker);
    }
    if export_formats.is_empty() {
        export_requirements.extend(
            SUPPORTED_IMAGE_FORMATS
                .into_iter()
                .map(CapabilityName::FormatEncode),
        );
    }
    if export_requirements.is_empty() {
        enabled_operations.push(DependentOperation::Export);
    } else {
        disabled_operations.push(DisabledOperation {
            operation: DependentOperation::Export,
            unavailable_capabilities: export_requirements,
        });
    }

    let mut availability_messages = capabilities.diagnostics().to_vec();
    if let Some(plan) = folder_plan {
        availability_messages.extend(plan.availability_notices());
    }

    CapabilityProjection {
        selectable_images,
        export_formats,
        enabled_operations,
        disabled_operations,
        availability_messages,
    }
}

/// A straight-alpha 16-bit RGBA pixel.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Hash)]
pub struct Rgba16 {
    pub red: u16,
    pub green: u16,
    pub blue: u16,
    pub alpha: u16,
}

impl Rgba16 {
    pub const fn new(red: u16, green: u16, blue: u16, alpha: u16) -> Self {
        Self {
            red,
            green,
            blue,
            alpha,
        }
    }
}

/// The only color-space interpretation accepted in the canonical pipeline.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum CanonicalColorSpace {
    #[default]
    Srgb,
}

/// Source orientation after decoder normalization.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum NormalizedOrientation {
    #[default]
    TopLeft,
}

/// A deterministic, owned, straight-alpha sRGB RGBA16 raster.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CanonicalImage {
    width: u32,
    height: u32,
    pixels: Vec<Rgba16>,
    color_space: CanonicalColorSpace,
    orientation: NormalizedOrientation,
}

impl CanonicalImage {
    pub fn new(
        width: u32,
        height: u32,
        pixels: Vec<Rgba16>,
    ) -> std::result::Result<Self, ImageValidationError> {
        Self::with_metadata(
            width,
            height,
            pixels,
            CanonicalColorSpace::Srgb,
            NormalizedOrientation::TopLeft,
        )
    }

    pub fn with_metadata(
        width: u32,
        height: u32,
        pixels: Vec<Rgba16>,
        color_space: CanonicalColorSpace,
        orientation: NormalizedOrientation,
    ) -> std::result::Result<Self, ImageValidationError> {
        let expected = Self::checked_pixel_count(width, height)?;
        if pixels.len() != expected {
            return Err(ImageValidationError::PixelCountMismatch {
                expected,
                actual: pixels.len(),
            });
        }
        Ok(Self {
            width,
            height,
            pixels,
            color_space,
            orientation,
        })
    }

    pub fn checked_pixel_count(
        width: u32,
        height: u32,
    ) -> std::result::Result<usize, ImageValidationError> {
        if width == 0 || height == 0 {
            return Err(ImageValidationError::ZeroDimension { width, height });
        }
        let count = u64::from(width)
            .checked_mul(u64::from(height))
            .ok_or(ImageValidationError::BufferSizeOverflow { width, height })?;
        let count = usize::try_from(count)
            .map_err(|_| ImageValidationError::BufferSizeOverflow { width, height })?;
        count
            .checked_mul(std::mem::size_of::<Rgba16>())
            .ok_or(ImageValidationError::BufferSizeOverflow { width, height })?;
        Ok(count)
    }

    pub const fn width(&self) -> u32 {
        self.width
    }

    pub const fn height(&self) -> u32 {
        self.height
    }

    pub fn pixels(&self) -> &[Rgba16] {
        &self.pixels
    }

    pub const fn color_space(&self) -> CanonicalColorSpace {
        self.color_space
    }

    pub const fn orientation(&self) -> NormalizedOrientation {
        self.orientation
    }
}

/// Alpha representation supplied by a decoder before canonical normalization.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum DecodedAlphaMode {
    /// RGB channels already represent unassociated (straight) sRGB samples.
    #[default]
    Straight,
    /// RGB channels are multiplied by alpha and must be unassociated.
    Premultiplied,
}

/// EXIF-compatible source orientation supplied by a decoder.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum SourceOrientation {
    #[default]
    TopLeft,
    TopRight,
    BottomRight,
    BottomLeft,
    LeftTop,
    RightTop,
    RightBottom,
    LeftBottom,
}

impl SourceOrientation {
    const fn output_dimensions(self, width: u32, height: u32) -> (u32, u32) {
        match self {
            Self::TopLeft | Self::TopRight | Self::BottomRight | Self::BottomLeft => {
                (width, height)
            }
            Self::LeftTop | Self::RightTop | Self::RightBottom | Self::LeftBottom => {
                (height, width)
            }
        }
    }

    const fn destination(self, width: u32, height: u32, x: u32, y: u32) -> (u32, u32) {
        match self {
            Self::TopLeft => (x, y),
            Self::TopRight => (width - 1 - x, y),
            Self::BottomRight => (width - 1 - x, height - 1 - y),
            Self::BottomLeft => (x, height - 1 - y),
            Self::LeftTop => (y, x),
            Self::RightTop => (height - 1 - y, x),
            Self::RightBottom => (height - 1 - y, width - 1 - x),
            Self::LeftBottom => (y, width - 1 - x),
        }
    }
}

/// Owned decoder output before it becomes a canonical image.
///
/// Codec adapters must convert source color data to sRGB before creating this
/// value. The shared core then owns orientation and alpha normalization, so
/// every later edit sees a top-left, straight-alpha RGBA16 raster.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DecodedImage {
    width: u32,
    height: u32,
    pixels: Vec<Rgba16>,
    alpha_mode: DecodedAlphaMode,
    orientation: SourceOrientation,
}

impl DecodedImage {
    pub fn new(
        width: u32,
        height: u32,
        pixels: Vec<Rgba16>,
        alpha_mode: DecodedAlphaMode,
        orientation: SourceOrientation,
    ) -> std::result::Result<Self, ImageValidationError> {
        let expected = CanonicalImage::checked_pixel_count(width, height)?;
        if pixels.len() != expected {
            return Err(ImageValidationError::PixelCountMismatch {
                expected,
                actual: pixels.len(),
            });
        }
        Ok(Self {
            width,
            height,
            pixels,
            alpha_mode,
            orientation,
        })
    }

    pub const fn width(&self) -> u32 {
        self.width
    }

    pub const fn height(&self) -> u32 {
        self.height
    }

    pub fn pixels(&self) -> &[Rgba16] {
        &self.pixels
    }

    pub const fn alpha_mode(&self) -> DecodedAlphaMode {
        self.alpha_mode
    }

    pub const fn orientation(&self) -> SourceOrientation {
        self.orientation
    }
}

/// Converts decoder output once into the owned, top-left straight-alpha sRGB
/// representation used by previews and exports.
pub fn normalize_decoded_image(
    decoded: DecodedImage,
) -> std::result::Result<CanonicalImage, ImageValidationError> {
    let (output_width, output_height) = decoded
        .orientation
        .output_dimensions(decoded.width, decoded.height);
    let output_len = CanonicalImage::checked_pixel_count(output_width, output_height)?;
    let mut output = vec![Rgba16::default(); output_len];

    for y in 0..decoded.height {
        for x in 0..decoded.width {
            let source_index = y as usize * decoded.width as usize + x as usize;
            let (destination_x, destination_y) =
                decoded
                    .orientation
                    .destination(decoded.width, decoded.height, x, y);
            let destination_index =
                destination_y as usize * output_width as usize + destination_x as usize;
            output[destination_index] =
                normalize_alpha(decoded.pixels[source_index], decoded.alpha_mode);
        }
    }

    CanonicalImage::new(output_width, output_height, output)
}

fn normalize_alpha(pixel: Rgba16, alpha_mode: DecodedAlphaMode) -> Rgba16 {
    if alpha_mode == DecodedAlphaMode::Straight {
        return pixel;
    }
    if pixel.alpha == 0 {
        return Rgba16::new(0, 0, 0, 0);
    }

    let unassociate = |component: u16| {
        let numerator = i64::from(component) * i64::from(u16::MAX);
        let value = (numerator + i64::from(pixel.alpha) / 2) / i64::from(pixel.alpha);
        value.clamp(0, i64::from(u16::MAX)) as u16
    };
    Rgba16::new(
        unassociate(pixel.red),
        unassociate(pixel.green),
        unassociate(pixel.blue),
        pixel.alpha,
    )
}

/// A replay failure caused by malformed historical data rather than mutable UI
/// state. Valid reducer-produced histories cannot trigger this error.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ImagePipelineError {
    InvalidCrop(CropValidationError),
    InvalidImage(ImageValidationError),
}

impl fmt::Display for ImagePipelineError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidCrop(error) => {
                write!(formatter, "invalid crop in edit history: {error:?}")
            }
            Self::InvalidImage(error) => {
                write!(formatter, "invalid image during replay: {error:?}")
            }
        }
    }
}

impl std::error::Error for ImagePipelineError {}

/// Replays the immutable base image, committed history, and uncommitted drafts
/// at full resolution. Drafts always follow history in brightness-then-contrast
/// order, regardless of which adjustment currently has focus.
pub fn render_current_editing_result(
    source: &CanonicalImage,
    history: &[EditOperation],
    draft: &DraftAdjustments,
) -> std::result::Result<CanonicalImage, ImagePipelineError> {
    let mut image = source.clone();
    for operation in history {
        image = apply_edit_operation(&image, operation)?;
    }
    image = apply_adjustment(image, AdjustmentKind::Brightness, draft.brightness())?;
    apply_adjustment(image, AdjustmentKind::Contrast, draft.contrast())
}

/// A platform-neutral, byte-for-byte comparable rendering artifact.
///
/// This deliberately records the rendered image rather than an encoded file,
/// so codec metadata and container-level differences cannot hide a pipeline
/// difference. Crop operations remain in the artifact because their original
/// source-coordinate bounds are part of the cross-platform editing result.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConformanceResult {
    width: u32,
    height: u32,
    orientation: NormalizedOrientation,
    crop_history: Vec<CropRect>,
    pixels: Vec<Rgba16>,
}

impl ConformanceResult {
    /// Captures a rendered canonical result and the crop operations that
    /// produced it, preserving their application order.
    pub fn from_rendered(image: &CanonicalImage, history: &[EditOperation]) -> Self {
        Self {
            width: image.width(),
            height: image.height(),
            orientation: image.orientation(),
            crop_history: history
                .iter()
                .filter_map(|operation| match operation {
                    EditOperation::Crop(crop) => Some(*crop),
                    _ => None,
                })
                .collect(),
            pixels: image.pixels().to_vec(),
        }
    }

    pub const fn width(&self) -> u32 {
        self.width
    }

    pub const fn height(&self) -> u32 {
        self.height
    }

    pub const fn orientation(&self) -> NormalizedOrientation {
        self.orientation
    }

    pub fn crop_history(&self) -> &[CropRect] {
        &self.crop_history
    }

    pub fn pixels(&self) -> &[Rgba16] {
        &self.pixels
    }

    /// Serializes every equivalence-relevant value in a stable text format.
    ///
    /// The format has a fixed field order, decimal integer representation, and
    /// row-major RGBA16 samples. It is suitable for CI artifacts produced on
    /// separate macOS and Linux runners.
    pub fn serialize(&self) -> String {
        let orientation = match self.orientation {
            NormalizedOrientation::TopLeft => "top-left",
        };
        let mut output = String::from("image-editor-conformance-v1\n");
        output.push_str("dimensions=");
        output.push_str(&self.width.to_string());
        output.push('x');
        output.push_str(&self.height.to_string());
        output.push('\n');
        output.push_str("orientation=");
        output.push_str(orientation);
        output.push('\n');
        output.push_str("crop-history=");
        for (index, crop) in self.crop_history.iter().enumerate() {
            if index != 0 {
                output.push(';');
            }
            output.push_str(&crop.left().to_string());
            output.push(',');
            output.push_str(&crop.top().to_string());
            output.push(',');
            output.push_str(&crop.right().to_string());
            output.push(',');
            output.push_str(&crop.bottom().to_string());
        }
        output.push('\n');
        output.push_str("rgba16=");
        for (index, pixel) in self.pixels.iter().enumerate() {
            if index != 0 {
                output.push(';');
            }
            output.push_str(&pixel.red.to_string());
            output.push(',');
            output.push_str(&pixel.green.to_string());
            output.push(',');
            output.push_str(&pixel.blue.to_string());
            output.push(',');
            output.push_str(&pixel.alpha.to_string());
        }
        output.push('\n');
        output
    }
}

/// Applies exactly one shared edit operation without mutating its input.
pub fn apply_edit_operation(
    image: &CanonicalImage,
    operation: &EditOperation,
) -> std::result::Result<CanonicalImage, ImagePipelineError> {
    match operation {
        EditOperation::FlipHorizontal => {
            transform_pixels(image, image.width, image.height, |x, y| {
                (image.width - 1 - x, y)
            })
        }
        EditOperation::FlipVertical => {
            transform_pixels(image, image.width, image.height, |x, y| {
                (x, image.height - 1 - y)
            })
        }
        EditOperation::RotateClockwise90 => {
            transform_pixels(image, image.height, image.width, |x, y| {
                (image.height - 1 - y, x)
            })
        }
        EditOperation::RotateCounterclockwise90 => {
            transform_pixels(image, image.height, image.width, |x, y| {
                (y, image.width - 1 - x)
            })
        }
        EditOperation::Crop(crop) => {
            let crop = CropRect::new(
                image.width,
                image.height,
                crop.left,
                crop.top,
                crop.right,
                crop.bottom,
            )
            .map_err(ImagePipelineError::InvalidCrop)?;
            let mut pixels = Vec::with_capacity(crop.width() as usize * crop.height() as usize);
            for y in crop.top()..crop.bottom() {
                let offset = y as usize * image.width as usize + crop.left() as usize;
                let end = offset + crop.width() as usize;
                pixels.extend_from_slice(&image.pixels[offset..end]);
            }
            CanonicalImage::new(crop.width(), crop.height(), pixels)
                .map_err(ImagePipelineError::InvalidImage)
        }
        EditOperation::Brightness(value) => {
            apply_adjustment(image.clone(), AdjustmentKind::Brightness, *value)
        }
        EditOperation::Contrast(value) => {
            apply_adjustment(image.clone(), AdjustmentKind::Contrast, *value)
        }
    }
}

fn transform_pixels(
    image: &CanonicalImage,
    output_width: u32,
    output_height: u32,
    destination: impl Fn(u32, u32) -> (u32, u32),
) -> std::result::Result<CanonicalImage, ImagePipelineError> {
    let output_len = CanonicalImage::checked_pixel_count(output_width, output_height)
        .map_err(ImagePipelineError::InvalidImage)?;
    let mut output = vec![Rgba16::default(); output_len];
    for y in 0..image.height {
        for x in 0..image.width {
            let source_index = y as usize * image.width as usize + x as usize;
            let (destination_x, destination_y) = destination(x, y);
            let destination_index =
                destination_y as usize * output_width as usize + destination_x as usize;
            output[destination_index] = image.pixels[source_index];
        }
    }
    CanonicalImage::new(output_width, output_height, output)
        .map_err(ImagePipelineError::InvalidImage)
}

fn apply_adjustment(
    mut image: CanonicalImage,
    kind: AdjustmentKind,
    value: AdjustmentValue,
) -> std::result::Result<CanonicalImage, ImagePipelineError> {
    if value == AdjustmentValue::ZERO {
        return Ok(image);
    }

    for pixel in &mut image.pixels {
        let adjust = |sample: u16| match kind {
            AdjustmentKind::Brightness => {
                let delta = round_nearest(i64::from(value.get()) * i64::from(u16::MAX), 100);
                (i64::from(sample) + delta).clamp(0, i64::from(u16::MAX)) as u16
            }
            AdjustmentKind::Contrast => {
                const MIDPOINT: i64 = 32_768;
                let scaled = round_nearest(
                    (i64::from(sample) - MIDPOINT) * (100 + i64::from(value.get())),
                    100,
                );
                (MIDPOINT + scaled).clamp(0, i64::from(u16::MAX)) as u16
            }
        };
        pixel.red = adjust(pixel.red);
        pixel.green = adjust(pixel.green);
        pixel.blue = adjust(pixel.blue);
    }
    Ok(image)
}

/// Fixed-point division rounded to nearest, with half values away from zero.
fn round_nearest(numerator: i64, denominator: i64) -> i64 {
    debug_assert!(denominator > 0);
    let magnitude = numerator.unsigned_abs();
    let rounded = (magnitude + denominator as u64 / 2) / denominator as u64;
    if numerator.is_negative() {
        -(rounded as i64)
    } else {
        rounded as i64
    }
}

/// Revision and draft values identifying one deterministic replay result.
#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct ReplayCacheKey {
    image_id: ImageId,
    revision: Revision,
    brightness: AdjustmentValue,
    contrast: AdjustmentValue,
}

impl ReplayCacheKey {
    pub fn new(image_id: ImageId, revision: Revision, draft: &DraftAdjustments) -> Self {
        Self {
            image_id,
            revision,
            brightness: draft.brightness(),
            contrast: draft.contrast(),
        }
    }

    pub fn image_id(&self) -> &ImageId {
        &self.image_id
    }

    pub const fn revision(&self) -> Revision {
        self.revision
    }

    pub const fn brightness(&self) -> AdjustmentValue {
        self.brightness
    }

    pub const fn contrast(&self) -> AdjustmentValue {
        self.contrast
    }
}

/// A revision-keyed cache that uses the same replay function for every miss.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ReplayCache {
    entries: BTreeMap<ReplayCacheKey, CanonicalImage>,
}

impl ReplayCache {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn evaluate_preview(
        &mut self,
        request: &PreviewRequest,
    ) -> std::result::Result<CanonicalImage, ImagePipelineError> {
        let key = ReplayCacheKey::new(request.image_id.clone(), request.revision, &request.draft);
        if let Some(cached) = self.entries.get(&key) {
            return Ok(cached.clone());
        }

        let result =
            render_current_editing_result(&request.source, &request.history, &request.draft)?;
        self.entries.insert(key, result.clone());
        Ok(result)
    }
}

/// Canonical image invariant violation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ImageValidationError {
    ZeroDimension { width: u32, height: u32 },
    BufferSizeOverflow { width: u32, height: u32 },
    PixelCountMismatch { expected: usize, actual: usize },
}

/// A validated nonempty half-open crop rectangle in source-pixel coordinates.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub struct CropRect {
    left: u32,
    top: u32,
    right: u32,
    bottom: u32,
}

impl CropRect {
    pub fn new(
        width: u32,
        height: u32,
        left: u32,
        top: u32,
        right: u32,
        bottom: u32,
    ) -> std::result::Result<Self, CropValidationError> {
        if left >= right {
            return Err(CropValidationError::EmptyOrReversedHorizontal { left, right });
        }
        if top >= bottom {
            return Err(CropValidationError::EmptyOrReversedVertical { top, bottom });
        }
        if right > width || bottom > height {
            return Err(CropValidationError::OutOfBounds {
                width,
                height,
                left,
                top,
                right,
                bottom,
            });
        }
        Ok(Self {
            left,
            top,
            right,
            bottom,
        })
    }

    pub const fn left(self) -> u32 {
        self.left
    }
    pub const fn top(self) -> u32 {
        self.top
    }
    pub const fn right(self) -> u32 {
        self.right
    }
    pub const fn bottom(self) -> u32 {
        self.bottom
    }
    pub const fn width(self) -> u32 {
        self.right - self.left
    }
    pub const fn height(self) -> u32 {
        self.bottom - self.top
    }
}

/// Crop validation failure for untrusted crop bounds.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CropValidationError {
    EmptyOrReversedHorizontal {
        left: u32,
        right: u32,
    },
    EmptyOrReversedVertical {
        top: u32,
        bottom: u32,
    },
    OutOfBounds {
        width: u32,
        height: u32,
        left: u32,
        top: u32,
        right: u32,
        bottom: u32,
    },
}

/// A closed-range adjustment value accepted by the pure core.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct AdjustmentValue(i16);

impl AdjustmentValue {
    pub const MIN: i16 = -100;
    pub const MAX: i16 = 100;
    pub const ZERO: Self = Self(0);

    pub fn new(value: i16) -> std::result::Result<Self, AdjustmentValidationError> {
        if !(Self::MIN..=Self::MAX).contains(&value) {
            return Err(AdjustmentValidationError::OutOfRange { value });
        }
        Ok(Self(value))
    }

    pub const fn get(self) -> i16 {
        self.0
    }

    pub fn increase(self) -> Self {
        Self(self.0.saturating_add(1).min(Self::MAX))
    }

    pub fn decrease(self) -> Self {
        Self(self.0.saturating_sub(1).max(Self::MIN))
    }
}

impl Default for AdjustmentValue {
    fn default() -> Self {
        Self::ZERO
    }
}

/// Adjustment input was outside the requirement-defined closed range.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AdjustmentValidationError {
    OutOfRange { value: i16 },
}

/// The adjustment field currently being edited.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AdjustmentKind {
    Brightness,
    Contrast,
}

/// An immutable operation recorded in a document's history.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EditOperation {
    FlipHorizontal,
    FlipVertical,
    RotateClockwise90,
    RotateCounterclockwise90,
    Crop(CropRect),
    Brightness(AdjustmentValue),
    Contrast(AdjustmentValue),
}

impl EditOperation {
    pub fn brightness(value: i16) -> std::result::Result<Self, AdjustmentValidationError> {
        Ok(Self::Brightness(AdjustmentValue::new(value)?))
    }

    pub fn contrast(value: i16) -> std::result::Result<Self, AdjustmentValidationError> {
        Ok(Self::Contrast(AdjustmentValue::new(value)?))
    }
}

/// Per-document, uncommitted adjustment values and optional focus.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct DraftAdjustments {
    brightness: AdjustmentValue,
    contrast: AdjustmentValue,
    focused: Option<AdjustmentKind>,
}

impl DraftAdjustments {
    pub const fn new() -> Self {
        Self {
            brightness: AdjustmentValue::ZERO,
            contrast: AdjustmentValue::ZERO,
            focused: None,
        }
    }

    pub const fn brightness(&self) -> AdjustmentValue {
        self.brightness
    }
    pub const fn contrast(&self) -> AdjustmentValue {
        self.contrast
    }
    pub const fn focused(&self) -> Option<AdjustmentKind> {
        self.focused
    }

    pub fn focus(&mut self, kind: AdjustmentKind) {
        self.focused = Some(kind);
    }

    pub fn set(
        &mut self,
        kind: AdjustmentKind,
        value: i16,
    ) -> std::result::Result<(), AdjustmentValidationError> {
        let value = AdjustmentValue::new(value)?;
        match kind {
            AdjustmentKind::Brightness => self.brightness = value,
            AdjustmentKind::Contrast => self.contrast = value,
        }
        Ok(())
    }

    pub fn increase_focused(&mut self) {
        match self.focused {
            Some(AdjustmentKind::Brightness) => self.brightness = self.brightness.increase(),
            Some(AdjustmentKind::Contrast) => self.contrast = self.contrast.increase(),
            None => {}
        }
    }

    pub fn decrease_focused(&mut self) {
        match self.focused {
            Some(AdjustmentKind::Brightness) => self.brightness = self.brightness.decrease(),
            Some(AdjustmentKind::Contrast) => self.contrast = self.contrast.decrease(),
            None => {}
        }
    }

    /// Produces the operation for the focused adjustment and resets only it.
    pub fn commit_focused(&mut self) -> Option<EditOperation> {
        match self.focused.take() {
            Some(AdjustmentKind::Brightness) => {
                let value = std::mem::take(&mut self.brightness);
                Some(EditOperation::Brightness(value))
            }
            Some(AdjustmentKind::Contrast) => {
                let value = std::mem::take(&mut self.contrast);
                Some(EditOperation::Contrast(value))
            }
            None => None,
        }
    }

    /// Commits pending drafts in render order so committing one adjustment does
    /// not change the currently visible result when the other draft is nonzero.
    pub fn commit_for_stable_preview(&mut self) -> Vec<EditOperation> {
        let Some(focused) = self.focused.take() else {
            return Vec::new();
        };

        let operation = match focused {
            AdjustmentKind::Brightness => {
                EditOperation::Brightness(std::mem::take(&mut self.brightness))
            }
            AdjustmentKind::Contrast => EditOperation::Contrast(std::mem::take(&mut self.contrast)),
        };
        vec![operation]
    }
}

/// A user-visible message severity.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NoticeSeverity {
    Availability,
    Error,
    Info,
}

/// A stable, structured subject for a visible notice.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum NoticeSubject {
    FileName(Utf8FileName),
    Path(AbsolutePath),
    Capability(CapabilityName),
    Command(CommandName),
}

/// A non-modal, user-safe visible notice.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VisibleNotice {
    pub severity: NoticeSeverity,
    pub subject: NoticeSubject,
    pub message: SafeError,
}

impl VisibleNotice {
    pub fn new(severity: NoticeSeverity, subject: NoticeSubject, message: SafeError) -> Self {
        Self {
            severity,
            subject,
            message,
        }
    }
}

/// The semantic command named in a no-active-image error.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CommandName {
    FlipHorizontal,
    FlipVertical,
    RotateClockwise90,
    RotateCounterclockwise90,
    EnterCrop,
    ConfirmCrop,
    FocusBrightness,
    FocusContrast,
    IncreaseAdjustment,
    DecreaseAdjustment,
    CommitAdjustment,
    Undo,
    Redo,
    Export,
}

impl fmt::Display for CommandName {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::FlipHorizontal => "horizontal flip",
            Self::FlipVertical => "vertical flip",
            Self::RotateClockwise90 => "clockwise rotation",
            Self::RotateCounterclockwise90 => "counterclockwise rotation",
            Self::EnterCrop => "crop",
            Self::ConfirmCrop => "confirm crop",
            Self::FocusBrightness => "brightness adjustment",
            Self::FocusContrast => "contrast adjustment",
            Self::IncreaseAdjustment => "increase adjustment",
            Self::DecreaseAdjustment => "decrease adjustment",
            Self::CommitAdjustment => "commit adjustment",
            Self::Undo => "undo",
            Self::Redo => "redo",
            Self::Export => "export",
        })
    }
}

/// The kind of resource constrained while decoding.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ResourceLimitKind {
    InputBytes,
    Dimensions,
    TotalPixels,
    IntermediateAllocation,
}

/// An export target conflict identified before opening a writer.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TargetConflict {
    SourceImage,
    ExistingLocalFile,
}

/// The common application-level failure envelope used across crate boundaries.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ApplicationError {
    UnavailableCapability {
        capability: &'static str,
        cause: SafeError,
    },
    Boundary {
        operation: &'static str,
        cause: SafeError,
    },
    FolderEnumeration {
        folder: AbsolutePath,
        cause: SafeError,
    },
    Decode {
        file_name: Utf8FileName,
        cause: SafeError,
    },
    MissingActiveImage {
        command: CommandName,
    },
    InvalidCrop {
        reason: CropValidationError,
    },
    ExportTargetConflict {
        path: AbsolutePath,
        kind: TargetConflict,
    },
    ExportWrite {
        path: AbsolutePath,
        cause: SafeError,
    },
    PlatformOperation {
        capability: CapabilityName,
        cause: SafeError,
    },
    ResourceLimit {
        subject: Utf8FileName,
        limit: ResourceLimitKind,
    },
}

impl ApplicationError {
    pub fn unavailable(capability: &'static str, cause: SafeError) -> Self {
        Self::UnavailableCapability { capability, cause }
    }

    pub fn boundary(operation: &'static str, cause: SafeError) -> Self {
        Self::Boundary { operation, cause }
    }

    pub fn to_notice(&self) -> VisibleNotice {
        match self {
            Self::FolderEnumeration { folder, cause } => VisibleNotice::new(
                NoticeSeverity::Error,
                NoticeSubject::Path(folder.clone()),
                cause.clone(),
            ),
            Self::Decode { file_name, cause } => VisibleNotice::new(
                NoticeSeverity::Error,
                NoticeSubject::FileName(file_name.clone()),
                cause.clone(),
            ),
            Self::PlatformOperation { capability, cause } => VisibleNotice::new(
                NoticeSeverity::Error,
                NoticeSubject::Capability(*capability),
                cause.clone(),
            ),
            _ => VisibleNotice::new(
                NoticeSeverity::Error,
                NoticeSubject::Command(CommandName::Export),
                SafeError::new(ErrorCategory::Invariant, self.to_string()),
            ),
        }
    }
}

impl fmt::Display for ApplicationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnavailableCapability { capability, cause } => {
                write!(formatter, "{capability} is unavailable: {cause}")
            }
            Self::Boundary { operation, cause } => write!(formatter, "{operation} failed: {cause}"),
            Self::FolderEnumeration { folder, cause } => {
                write!(formatter, "could not enumerate {folder}: {cause}")
            }
            Self::Decode { file_name, cause } => {
                write!(formatter, "could not decode {file_name}: {cause}")
            }
            Self::MissingActiveImage { command } => {
                write!(formatter, "cannot {command} without an active image")
            }
            Self::InvalidCrop { reason } => write!(formatter, "invalid crop: {reason:?}"),
            Self::ExportTargetConflict { path, kind } => {
                write!(formatter, "cannot export to {path}: {kind:?}")
            }
            Self::ExportWrite { path, cause } => {
                write!(formatter, "could not export to {path}: {cause}")
            }
            Self::PlatformOperation { capability, cause } => {
                write!(formatter, "{capability} failed: {cause}")
            }
            Self::ResourceLimit { subject, limit } => {
                write!(formatter, "{subject} exceeded decode limit: {limit:?}")
            }
        }
    }
}

impl std::error::Error for ApplicationError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn image_constructor_rejects_zero_or_mismatched_dimensions() {
        assert_eq!(
            CanonicalImage::new(0, 1, vec![]),
            Err(ImageValidationError::ZeroDimension {
                width: 0,
                height: 1
            })
        );
        assert_eq!(
            CanonicalImage::new(2, 2, vec![Rgba16::default(); 3]),
            Err(ImageValidationError::PixelCountMismatch {
                expected: 4,
                actual: 3
            })
        );
    }

    #[test]
    fn crop_constructor_uses_nonempty_half_open_bounds() {
        let crop = CropRect::new(8, 6, 2, 1, 7, 5).unwrap();
        assert_eq!((crop.width(), crop.height()), (5, 4));
        assert!(matches!(
            CropRect::new(8, 6, 2, 1, 2, 5),
            Err(CropValidationError::EmptyOrReversedHorizontal { .. })
        ));
        assert!(matches!(
            CropRect::new(8, 6, 2, 1, 9, 5),
            Err(CropValidationError::OutOfBounds { .. })
        ));
    }

    #[test]
    fn adjustment_values_enforce_range_and_drafts_clamp() {
        assert_eq!(
            AdjustmentValue::new(101),
            Err(AdjustmentValidationError::OutOfRange { value: 101 })
        );
        let mut draft = DraftAdjustments::new();
        draft.focus(AdjustmentKind::Brightness);
        draft.set(AdjustmentKind::Brightness, 100).unwrap();
        draft.increase_focused();
        assert_eq!(draft.brightness().get(), 100);
        assert!(
            matches!(draft.commit_focused(), Some(EditOperation::Brightness(value)) if value.get() == 100)
        );
        assert_eq!(draft.brightness(), AdjustmentValue::ZERO);
    }

    #[test]
    fn collection_entries_require_matching_utf8_name_format_and_identity_path() {
        let path = AbsolutePath::new("/images/photo.png").unwrap();
        let filename = Utf8FileName::new("photo.png").unwrap();
        let identity = SourceIdentity::new(path.clone(), None);
        assert!(CollectionEntry::new(identity, path, filename, ImageFormat::Png).is_ok());
    }

    #[test]
    fn visible_notices_use_safe_messages() {
        let safe = SafeError::new(ErrorCategory::PortableCodec, "bad\ninput");
        assert_eq!(safe.summary(), "bad input");
        let name = Utf8FileName::new("image.png").unwrap();
        let notice = VisibleNotice::new(NoticeSeverity::Error, NoticeSubject::FileName(name), safe);
        assert_eq!(notice.severity, NoticeSeverity::Error);
    }
}

/// A monotonically increasing revision of mutable editor data.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct Revision(u64);

impl Revision {
    pub const INITIAL: Self = Self(0);

    pub const fn get(self) -> u64 {
        self.0
    }

    fn next(self) -> Self {
        Self(self.0.checked_add(1).expect("editor revision overflow"))
    }
}

/// A monotonically increasing identifier for one asynchronous effect request.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct RequestId(u64);

impl RequestId {
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// The value an effect worker must return with every completion.
///
/// `revision` scopes a request to the browsing generation or image-document
/// revision from which it was created. A completion is accepted only when both
/// this token and the current pending request still match.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct RequestToken {
    request_id: RequestId,
    revision: Revision,
}

impl RequestToken {
    pub const fn request_id(self) -> RequestId {
        self.request_id
    }

    pub const fn revision(self) -> Revision {
        self.revision
    }
}

/// A mutable crop selection expressed in source-pixel coordinates.
///
/// Unlike `CropRect`, this intentionally permits invalid and empty values while
/// the user is moving crop handles. Confirmation validates it before history is
/// changed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CropDraft {
    pub left: u32,
    pub top: u32,
    pub right: u32,
    pub bottom: u32,
}

impl CropDraft {
    pub const fn new(left: u32, top: u32, right: u32, bottom: u32) -> Self {
        Self {
            left,
            top,
            right,
            bottom,
        }
    }

    /// Clamps each source-pixel boundary independently to the current image.
    ///
    /// This intentionally preserves an empty or reversed selection while a user
    /// moves handles; `CropRect::new` validates the selection at confirmation.
    pub fn clamped(self, width: u32, height: u32) -> Self {
        Self {
            left: self.left.min(width),
            top: self.top.min(height),
            right: self.right.min(width),
            bottom: self.bottom.min(height),
        }
    }
}

/// The editor interaction that currently owns keyboard and control input.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum InteractionMode {
    #[default]
    Browse,
    Crop(CropDraft),
    Adjust(AdjustmentKind),
}

/// The immutable per-image document retained for the entire open folder session.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ImageDocument {
    source: CanonicalImage,
    history: Vec<EditOperation>,
    redo: Vec<EditOperation>,
    draft: DraftAdjustments,
    revision: Revision,
}

impl ImageDocument {
    pub fn new(source: CanonicalImage) -> Self {
        Self {
            source,
            history: Vec::new(),
            redo: Vec::new(),
            draft: DraftAdjustments::new(),
            revision: Revision::INITIAL,
        }
    }

    pub fn source(&self) -> &CanonicalImage {
        &self.source
    }

    pub fn history(&self) -> &[EditOperation] {
        &self.history
    }

    pub fn redo(&self) -> &[EditOperation] {
        &self.redo
    }

    pub fn draft(&self) -> &DraftAdjustments {
        &self.draft
    }

    pub const fn revision(&self) -> Revision {
        self.revision
    }

    /// Advances the document revision after a reducer-owned edit transition.
    ///
    /// This is public for future reducer extensions, but callers receive an
    /// owned document only and cannot mutate state held by `EditorState`.
    pub fn mark_changed(&mut self) {
        self.revision = self.revision.next();
    }
}

/// The content currently shown in the preview region.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PreviewState {
    EmptyCollection,
    NoActiveImage,
    Pending {
        image_id: ImageId,
        revision: Revision,
        token: RequestToken,
    },
    Rendered {
        image_id: ImageId,
        revision: Revision,
        image: CanonicalImage,
    },
}

impl PreviewState {
    fn for_collection(collection: &ImageCollection) -> Self {
        if collection.entries().is_empty() {
            Self::EmptyCollection
        } else {
            Self::NoActiveImage
        }
    }
}

/// The complete browsing data that must remain intact while I/O is pending.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BrowsingState {
    source_folder: Option<AbsolutePath>,
    collection: ImageCollection,
    active: Option<ImageId>,
    documents: BTreeMap<ImageId, ImageDocument>,
    preview: PreviewState,
    revision: Revision,
}

impl Default for BrowsingState {
    fn default() -> Self {
        Self {
            source_folder: None,
            collection: ImageCollection::default(),
            active: None,
            documents: BTreeMap::new(),
            preview: PreviewState::EmptyCollection,
            revision: Revision::INITIAL,
        }
    }
}

impl BrowsingState {
    pub fn source_folder(&self) -> Option<&AbsolutePath> {
        self.source_folder.as_ref()
    }

    pub fn collection(&self) -> &ImageCollection {
        &self.collection
    }

    pub fn active(&self) -> Option<&ImageId> {
        self.active.as_ref()
    }

    pub fn documents(&self) -> &BTreeMap<ImageId, ImageDocument> {
        &self.documents
    }

    pub fn document(&self, image_id: &ImageId) -> Option<&ImageDocument> {
        self.documents.get(image_id)
    }

    pub fn preview(&self) -> &PreviewState {
        &self.preview
    }

    pub const fn revision(&self) -> Revision {
        self.revision
    }
}

/// Snapshot inputs for a pure, off-thread preview render.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PreviewRequest {
    pub image_id: ImageId,
    pub revision: Revision,
    pub source: CanonicalImage,
    pub history: Vec<EditOperation>,
    pub draft: DraftAdjustments,
}

impl PreviewRequest {
    fn from_document(image_id: ImageId, document: &ImageDocument) -> Self {
        Self {
            image_id,
            revision: document.revision,
            source: document.source.clone(),
            history: document.history.clone(),
            draft: document.draft.clone(),
        }
    }
}

/// Snapshot inputs for a guarded, off-thread export write.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExportRequest {
    pub image_id: ImageId,
    pub revision: Revision,
    pub source: CanonicalImage,
    pub history: Vec<EditOperation>,
    pub draft: DraftAdjustments,
    pub target: AbsolutePath,
    pub format: ImageFormat,
}

impl ExportRequest {
    fn from_document(
        image_id: ImageId,
        document: &ImageDocument,
        target: AbsolutePath,
        format: ImageFormat,
    ) -> Self {
        Self {
            image_id,
            revision: document.revision,
            source: document.source.clone(),
            history: document.history.clone(),
            draft: document.draft.clone(),
            target,
            format,
        }
    }
}

/// A declarative, side-effecting operation emitted by the pure reducer.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Effect {
    EnumerateFolder {
        token: RequestToken,
        folder: AbsolutePath,
    },
    DecodeImage {
        token: RequestToken,
        candidate: CollectionEntry,
    },
    RenderPreview {
        token: RequestToken,
        request: PreviewRequest,
    },
    WriteExport {
        token: RequestToken,
        request: ExportRequest,
    },
}

impl Effect {
    pub const fn token(&self) -> RequestToken {
        match self {
            Self::EnumerateFolder { token, .. }
            | Self::DecodeImage { token, .. }
            | Self::RenderPreview { token, .. }
            | Self::WriteExport { token, .. } => *token,
        }
    }
}

/// The state recorded for an effect that is in flight.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PendingRequest {
    FolderEnumeration {
        token: RequestToken,
        folder: AbsolutePath,
    },
    Decode {
        token: RequestToken,
        candidate: CollectionEntry,
    },
    Preview {
        token: RequestToken,
        image_id: ImageId,
    },
    Export {
        token: RequestToken,
        image_id: ImageId,
        target: AbsolutePath,
    },
}

impl PendingRequest {
    pub const fn token(&self) -> RequestToken {
        match self {
            Self::FolderEnumeration { token, .. }
            | Self::Decode { token, .. }
            | Self::Preview { token, .. }
            | Self::Export { token, .. } => *token,
        }
    }
}

/// A directional navigation request within the ordered image collection.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NavigationDirection {
    Left,
    Right,
    Home,
    End,
}

/// The pure result of resolving a navigation request against browsing state.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum NavigationTarget {
    EmptyCollection,
    NoActiveImage,
    NoTarget,
    Candidate(ImageId),
}

/// Plans a non-wrapping navigation target without mutating browsing state.
pub fn plan_navigation(
    collection: &ImageCollection,
    active: Option<&ImageId>,
    direction: NavigationDirection,
) -> NavigationTarget {
    let entries = collection.entries();
    if entries.is_empty() {
        return NavigationTarget::EmptyCollection;
    }

    let Some(active) = active else {
        return NavigationTarget::NoActiveImage;
    };
    let Some(index) = entries.iter().position(|entry| &entry.id == active) else {
        return NavigationTarget::NoActiveImage;
    };

    let candidate = match direction {
        NavigationDirection::Left => index.checked_sub(1),
        NavigationDirection::Right => (index + 1 < entries.len()).then_some(index + 1),
        NavigationDirection::Home => (index != 0).then_some(0),
        NavigationDirection::End => (index + 1 != entries.len()).then_some(entries.len() - 1),
    };
    candidate
        .map(|index| NavigationTarget::Candidate(entries[index].id.clone()))
        .unwrap_or(NavigationTarget::NoTarget)
}

/// The OS family whose keyboard conventions are active for this session.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RuntimePlatform {
    MacOs,
    Linux,
}

/// A platform-neutral key identity supplied by the desktop event adapter.
///
/// The adapter may derive this identity from either the native logical key or
/// its physical key code. Letter matching is case-insensitive so both paths
/// resolve to the same keyboard intent.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum ShortcutKey {
    Character(char),
    ArrowUp,
    ArrowDown,
    ArrowLeft,
    ArrowRight,
    PageUp,
    PageDown,
    Home,
    End,
    Enter,
    Space,
    F11,
}

impl ShortcutKey {
    pub fn normalized(self) -> Self {
        match self {
            Self::Character(character) => Self::Character(character.to_ascii_lowercase()),
            key => key,
        }
    }
}

/// Modifier state normalized from a native key event.
#[derive(Clone, Copy, Debug, Default, Eq, Ord, PartialEq, PartialOrd)]
pub struct KeyModifiers {
    pub command: bool,
    pub control: bool,
    pub option: bool,
    pub alt: bool,
    pub shift: bool,
}

impl KeyModifiers {
    pub const fn command() -> Self {
        Self {
            command: true,
            control: false,
            option: false,
            alt: false,
            shift: false,
        }
    }

    pub const fn control() -> Self {
        Self {
            command: false,
            control: true,
            option: false,
            alt: false,
            shift: false,
        }
    }

    pub const fn option() -> Self {
        Self {
            command: false,
            control: false,
            option: true,
            alt: false,
            shift: false,
        }
    }

    pub const fn alt() -> Self {
        Self {
            command: false,
            control: false,
            option: false,
            alt: true,
            shift: false,
        }
    }

    pub const fn with_shift(mut self) -> Self {
        self.shift = true;
        self
    }

    fn is_plain(self, shift: bool) -> bool {
        !self.command && !self.control && !self.option && !self.alt && self.shift == shift
    }

    fn is_macos_primary(self, shift: bool) -> bool {
        self.command && !self.control && !self.option && !self.alt && self.shift == shift
    }

    fn is_linux_primary(self, shift: bool) -> bool {
        !self.command && self.control && !self.option && !self.alt && self.shift == shift
    }

    fn is_macos_adjustment(self) -> bool {
        !self.command && !self.control && self.option && !self.alt && !self.shift
    }

    fn is_linux_adjustment(self) -> bool {
        !self.command && !self.control && !self.option && self.alt && !self.shift
    }
}

/// A stable ASCII action identifier used by keybinding configuration and diagnostics.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum KeybindingAction {
    FitToWindow,
    ZoomActual,
    Zoom200,
    ZoomIn,
    ZoomOut,
    PanLeft,
    PanDown,
    PanUp,
    PanRight,
    PreviousImage,
    NextImage,
    FirstImage,
    LastImage,
    ToggleFullscreen,
    FlipHorizontal,
    FlipVertical,
    RotateClockwise90,
    RotateCounterclockwise90,
    EnterCrop,
    FocusBrightness,
    FocusContrast,
    CommitAdjustment,
    Undo,
    Redo,
    IncreaseAdjustment,
    DecreaseAdjustment,
}

impl KeybindingAction {
    /// Returns the stable ASCII identifier persisted in keybinding files.
    pub const fn stable_name(self) -> &'static str {
        match self {
            Self::FitToWindow => "fit_to_window",
            Self::ZoomActual => "zoom_actual",
            Self::Zoom200 => "zoom_200",
            Self::ZoomIn => "zoom_in",
            Self::ZoomOut => "zoom_out",
            Self::PanLeft => "pan_left",
            Self::PanDown => "pan_down",
            Self::PanUp => "pan_up",
            Self::PanRight => "pan_right",
            Self::PreviousImage => "previous_image",
            Self::NextImage => "next_image",
            Self::FirstImage => "first_image",
            Self::LastImage => "last_image",
            Self::ToggleFullscreen => "toggle_fullscreen",
            Self::FlipHorizontal => "flip_horizontal",
            Self::FlipVertical => "flip_vertical",
            Self::RotateClockwise90 => "rotate_clockwise_90",
            Self::RotateCounterclockwise90 => "rotate_counterclockwise_90",
            Self::EnterCrop => "enter_crop",
            Self::FocusBrightness => "focus_brightness",
            Self::FocusContrast => "focus_contrast",
            Self::CommitAdjustment => "commit_adjustment",
            Self::Undo => "undo",
            Self::Redo => "redo",
            Self::IncreaseAdjustment => "increase_adjustment",
            Self::DecreaseAdjustment => "decrease_adjustment",
        }
    }
}

/// A normalized non-modifier key with its complete modifier set.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct KeybindingGesture {
    pub key: ShortcutKey,
    pub modifiers: KeyModifiers,
}

impl KeybindingGesture {
    pub fn new(key: ShortcutKey, modifiers: KeyModifiers) -> Self {
        Self {
            key: key.normalized(),
            modifiers,
        }
    }
}

/// A source from which a keybinding declaration was obtained.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum KeybindingSource {
    ExplicitCli(AbsolutePath),
    Project(AbsolutePath),
    User(AbsolutePath),
    BuiltIn,
}

/// The safe category of a keybinding configuration diagnostic.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum KeybindingDiagnosticKind {
    ReadFailed,
    InvalidToml,
    UnknownAction,
    UnknownKey,
    IllegalModifier,
    DuplicateGesture,
    BlockedByHigherPriority,
}

/// A source-aware, safe diagnostic emitted while resolving keybindings.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct KeybindingDiagnostic {
    pub source: KeybindingSource,
    pub action: Option<KeybindingAction>,
    pub gesture: Option<String>,
    pub category: KeybindingDiagnosticKind,
    pub safe_message: String,
}

impl KeybindingDiagnostic {
    pub fn new(
        source: KeybindingSource,
        action: Option<KeybindingAction>,
        gesture: Option<String>,
        category: KeybindingDiagnosticKind,
        safe_message: impl Into<String>,
    ) -> Self {
        Self {
            source,
            action,
            gesture,
            category,
            safe_message: safe_message.into(),
        }
    }
}

/// An immutable one-to-many action index and exclusive gesture index.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct EffectiveKeybindingMap {
    by_gesture: BTreeMap<KeybindingGesture, KeybindingAction>,
    by_action: BTreeMap<KeybindingAction, Vec<KeybindingGesture>>,
}

impl EffectiveKeybindingMap {
    pub fn try_from_bindings(
        bindings: BTreeMap<KeybindingAction, Vec<KeybindingGesture>>,
    ) -> std::result::Result<Self, KeybindingGesture> {
        let mut by_gesture = BTreeMap::new();
        let mut by_action = BTreeMap::new();

        for (action, mut gestures) in bindings {
            gestures.sort_unstable();
            gestures.dedup();
            if gestures.is_empty() {
                continue;
            }
            for gesture in &gestures {
                if let Some(existing) = by_gesture.insert(*gesture, action) {
                    if existing != action {
                        return Err(*gesture);
                    }
                }
            }
            by_action.insert(action, gestures);
        }

        Ok(Self {
            by_gesture,
            by_action,
        })
    }

    pub fn action_for(&self, gesture: KeybindingGesture) -> Option<KeybindingAction> {
        self.by_gesture.get(&gesture).copied()
    }

    pub fn gestures_for(&self, action: KeybindingAction) -> &[KeybindingGesture] {
        self.by_action
            .get(&action)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    pub fn by_gesture(&self) -> &BTreeMap<KeybindingGesture, KeybindingAction> {
        &self.by_gesture
    }

    pub fn by_action(&self) -> &BTreeMap<KeybindingAction, Vec<KeybindingGesture>> {
        &self.by_action
    }
}

/// Returns the complete, deterministic built-in binding table for one platform.
pub fn built_in_keybinding_map(platform: RuntimePlatform) -> EffectiveKeybindingMap {
    let plain = KeyModifiers::default();
    let shift = plain.with_shift();
    let primary = match platform {
        RuntimePlatform::MacOs => KeyModifiers::command(),
        RuntimePlatform::Linux => KeyModifiers::control(),
    };
    let adjustment = match platform {
        RuntimePlatform::MacOs => KeyModifiers::option(),
        RuntimePlatform::Linux => KeyModifiers::alt(),
    };
    let mut bindings = BTreeMap::new();
    let mut add = |action, gestures: Vec<KeybindingGesture>| {
        bindings.insert(action, gestures);
    };
    let gesture = KeybindingGesture::new;

    add(
        KeybindingAction::FitToWindow,
        vec![gesture(ShortcutKey::Character('0'), plain)],
    );
    add(
        KeybindingAction::ZoomActual,
        vec![gesture(ShortcutKey::Character('1'), plain)],
    );
    add(
        KeybindingAction::Zoom200,
        vec![gesture(ShortcutKey::Character('2'), plain)],
    );
    add(
        KeybindingAction::ZoomIn,
        vec![
            gesture(ShortcutKey::Character('+'), plain),
            gesture(ShortcutKey::Character('='), plain),
        ],
    );
    add(
        KeybindingAction::ZoomOut,
        vec![gesture(ShortcutKey::Character('-'), plain)],
    );
    add(
        KeybindingAction::PanLeft,
        vec![gesture(ShortcutKey::Character('h'), plain)],
    );
    add(
        KeybindingAction::PanDown,
        vec![gesture(ShortcutKey::Character('j'), plain)],
    );
    add(
        KeybindingAction::PanUp,
        vec![gesture(ShortcutKey::Character('k'), plain)],
    );
    add(
        KeybindingAction::PanRight,
        vec![gesture(ShortcutKey::Character('l'), plain)],
    );
    add(
        KeybindingAction::PreviousImage,
        vec![
            gesture(ShortcutKey::ArrowLeft, plain),
            gesture(ShortcutKey::ArrowUp, plain),
            gesture(ShortcutKey::PageUp, plain),
        ],
    );
    add(
        KeybindingAction::NextImage,
        vec![
            gesture(ShortcutKey::ArrowRight, plain),
            gesture(ShortcutKey::ArrowDown, plain),
            gesture(ShortcutKey::PageDown, plain),
            gesture(ShortcutKey::Space, plain),
        ],
    );
    add(
        KeybindingAction::FirstImage,
        vec![gesture(ShortcutKey::Home, plain)],
    );
    add(
        KeybindingAction::LastImage,
        vec![gesture(ShortcutKey::End, plain)],
    );
    let mut fullscreen = vec![gesture(ShortcutKey::F11, plain)];
    if platform == RuntimePlatform::MacOs {
        fullscreen.push(gesture(
            ShortcutKey::Character('f'),
            KeyModifiers {
                command: true,
                control: true,
                option: false,
                alt: false,
                shift: false,
            },
        ));
    }
    add(KeybindingAction::ToggleFullscreen, fullscreen);
    add(
        KeybindingAction::FlipHorizontal,
        vec![gesture(ShortcutKey::Character('f'), plain)],
    );
    add(
        KeybindingAction::FlipVertical,
        vec![gesture(ShortcutKey::Character('f'), shift)],
    );
    add(
        KeybindingAction::RotateClockwise90,
        vec![gesture(ShortcutKey::Character('r'), plain)],
    );
    add(
        KeybindingAction::RotateCounterclockwise90,
        vec![gesture(ShortcutKey::Character('r'), shift)],
    );
    add(
        KeybindingAction::EnterCrop,
        vec![gesture(ShortcutKey::Character('c'), plain)],
    );
    add(
        KeybindingAction::FocusBrightness,
        vec![gesture(ShortcutKey::Character('b'), plain)],
    );
    add(
        KeybindingAction::FocusContrast,
        vec![gesture(ShortcutKey::Character('d'), plain)],
    );
    add(
        KeybindingAction::CommitAdjustment,
        vec![gesture(ShortcutKey::Enter, plain)],
    );
    add(
        KeybindingAction::Undo,
        vec![gesture(ShortcutKey::Character('z'), primary)],
    );
    add(
        KeybindingAction::Redo,
        vec![gesture(ShortcutKey::Character('z'), primary.with_shift())],
    );
    add(
        KeybindingAction::IncreaseAdjustment,
        vec![gesture(ShortcutKey::ArrowUp, adjustment)],
    );
    add(
        KeybindingAction::DecreaseAdjustment,
        vec![gesture(ShortcutKey::ArrowDown, adjustment)],
    );

    EffectiveKeybindingMap::try_from_bindings(bindings)
        .expect("built-in keybindings must not contain duplicate gestures")
}

/// A logical preview or rendered-image size used by the view reducer.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct LogicalSize {
    pub width: u32,
    pub height: u32,
}

impl LogicalSize {
    pub const fn new(width: u32, height: u32) -> Self {
        Self { width, height }
    }

    pub const fn is_empty(self) -> bool {
        self.width == 0 || self.height == 0
    }
}

/// A logical canvas translation from the centered image position.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct LogicalVector {
    pub x: i64,
    pub y: i64,
}

/// A positive scale stored exactly as a reduced rational number.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RationalScale {
    numerator: u32,
    denominator: u32,
}

impl RationalScale {
    pub const MIN: Self = Self {
        numerator: 1,
        denominator: 4,
    };
    pub const MAX: Self = Self {
        numerator: 8,
        denominator: 1,
    };
    pub const ONE: Self = Self {
        numerator: 1,
        denominator: 1,
    };
    pub const TWO: Self = Self {
        numerator: 2,
        denominator: 1,
    };

    pub fn new(numerator: u32, denominator: u32) -> Option<Self> {
        if numerator == 0 || denominator == 0 {
            return None;
        }
        let divisor = gcd(numerator, denominator);
        Some(Self {
            numerator: numerator / divisor,
            denominator: denominator / divisor,
        })
    }

    pub const fn numerator(self) -> u32 {
        self.numerator
    }

    pub const fn denominator(self) -> u32 {
        self.denominator
    }

    pub fn clamp(self) -> Self {
        if self.less_than(Self::MIN) {
            Self::MIN
        } else if self.greater_than(Self::MAX) {
            Self::MAX
        } else {
            self
        }
    }

    fn multiplied(self, numerator: u32, denominator: u32) -> Self {
        let value = Self::new(
            self.numerator.saturating_mul(numerator),
            self.denominator.saturating_mul(denominator),
        )
        .expect("a positive scale multiplied by a positive ratio remains positive");
        value.clamp()
    }

    fn less_than(self, other: Self) -> bool {
        u64::from(self.numerator) * u64::from(other.denominator)
            < u64::from(other.numerator) * u64::from(self.denominator)
    }

    fn greater_than(self, other: Self) -> bool {
        u64::from(self.numerator) * u64::from(other.denominator)
            > u64::from(other.numerator) * u64::from(self.denominator)
    }
}

const fn gcd(mut left: u32, mut right: u32) -> u32 {
    while right != 0 {
        let remainder = left % right;
        left = right;
        right = remainder;
    }
    left
}

/// Whether the scale follows the preview fit calculation or a manual value.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ZoomMode {
    FitToWindow,
    Manual,
}

/// A fixed view-command zoom direction.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ZoomDirection {
    In,
    Out,
}

/// A directional canvas movement constrained to the rendered image bounds.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PanDirection {
    Left,
    Down,
    Up,
    Right,
}

/// View-only state; it does not own image pixels, history, or source data.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ViewState {
    pub zoom: ZoomMode,
    pub manual_scale: RationalScale,
    pub canvas_offset: LogicalVector,
    pub preview_size: LogicalSize,
}

impl Default for ViewState {
    fn default() -> Self {
        Self::for_preview_size(LogicalSize::default())
    }
}

impl ViewState {
    pub const fn for_preview_size(preview_size: LogicalSize) -> Self {
        Self {
            zoom: ZoomMode::FitToWindow,
            manual_scale: RationalScale::ONE,
            canvas_offset: LogicalVector { x: 0, y: 0 },
            preview_size,
        }
    }

    pub fn effective_scale(&self, image_size: LogicalSize) -> RationalScale {
        match self.zoom {
            ZoomMode::Manual => self.manual_scale.clamp(),
            ZoomMode::FitToWindow => fit_scale(image_size, self.preview_size),
        }
    }

    pub fn with_preview_size(&self, preview_size: LogicalSize, image_size: LogicalSize) -> Self {
        Self {
            preview_size,
            ..self.clone()
        }
        .clamped_to_image(image_size)
    }

    pub fn fit_to_window(&self, image_size: LogicalSize) -> Self {
        Self {
            zoom: ZoomMode::FitToWindow,
            canvas_offset: LogicalVector::default(),
            ..self.clone()
        }
        .clamped_to_image(image_size)
    }

    pub fn set_manual_zoom(&self, percent: u16, image_size: LogicalSize) -> Self {
        let requested = RationalScale::new(u32::from(percent.max(1)), 100)
            .expect("a nonzero percent creates a valid scale")
            .clamp();
        Self {
            zoom: ZoomMode::Manual,
            manual_scale: requested,
            ..self.clone()
        }
        .clamped_to_image(image_size)
    }

    pub fn zoom_by_step(&self, direction: ZoomDirection, image_size: LogicalSize) -> Self {
        let base = self.effective_scale(image_size);
        let manual_scale = match direction {
            ZoomDirection::In => base.multiplied(5, 4),
            ZoomDirection::Out => base.multiplied(4, 5),
        };
        Self {
            zoom: ZoomMode::Manual,
            manual_scale,
            ..self.clone()
        }
        .clamped_to_image(image_size)
    }

    pub fn pan(&self, direction: PanDirection, image_size: LogicalSize) -> Self {
        let mut next = self.clone().clamped_to_image(image_size);
        let (maximum_x, maximum_y) = next.offset_limits(image_size);
        match direction {
            PanDirection::Left if maximum_x > 0 => {
                next.canvas_offset.x -= pan_step(next.preview_size.width);
            }
            PanDirection::Right if maximum_x > 0 => {
                next.canvas_offset.x += pan_step(next.preview_size.width);
            }
            PanDirection::Up if maximum_y > 0 => {
                next.canvas_offset.y -= pan_step(next.preview_size.height);
            }
            PanDirection::Down if maximum_y > 0 => {
                next.canvas_offset.y += pan_step(next.preview_size.height);
            }
            _ => return next,
        }
        next.clamped_to_image(image_size)
    }

    pub fn clamped_to_image(&self, image_size: LogicalSize) -> Self {
        let mut next = self.clone();
        let (maximum_x, maximum_y) = next.offset_limits(image_size);
        next.canvas_offset.x = next.canvas_offset.x.clamp(-maximum_x, maximum_x);
        next.canvas_offset.y = next.canvas_offset.y.clamp(-maximum_y, maximum_y);
        next
    }

    fn offset_limits(&self, image_size: LogicalSize) -> (i64, i64) {
        let scale = self.effective_scale(image_size);
        let scaled_width = scaled_extent(image_size.width, scale);
        let scaled_height = scaled_extent(image_size.height, scale);
        (
            half_overflow(scaled_width, self.preview_size.width),
            half_overflow(scaled_height, self.preview_size.height),
        )
    }
}

fn fit_scale(image_size: LogicalSize, preview_size: LogicalSize) -> RationalScale {
    if image_size.is_empty() || preview_size.is_empty() {
        return RationalScale::MIN;
    }
    let width = RationalScale::new(preview_size.width, image_size.width)
        .expect("nonzero dimensions create a valid scale");
    let height = RationalScale::new(preview_size.height, image_size.height)
        .expect("nonzero dimensions create a valid scale");
    if width.less_than(height) {
        width.clamp()
    } else {
        height.clamp()
    }
}

fn scaled_extent(dimension: u32, scale: RationalScale) -> u64 {
    let numerator = u64::from(dimension) * u64::from(scale.numerator());
    let denominator = u64::from(scale.denominator());
    numerator.div_ceil(denominator)
}

fn half_overflow(scaled: u64, preview: u32) -> i64 {
    let overflow = scaled.saturating_sub(u64::from(preview));
    i64::try_from(overflow.div_ceil(2)).expect("logical dimensions fit in i64")
}

fn pan_step(preview_dimension: u32) -> i64 {
    i64::try_from(u64::from(preview_dimension).div_ceil(10).max(1))
        .expect("logical dimensions fit in i64")
}

/// A raw key event normalized at the desktop boundary without importing UI APIs.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RawKeyEvent {
    pub key: ShortcutKey,
    pub modifiers: KeyModifiers,
    pub pressed: bool,
    pub repeat: bool,
    /// Whether the focused text-capable control has already consumed this event.
    pub consumed_by_text_control: bool,
}

impl RawKeyEvent {
    pub const fn press(key: ShortcutKey, modifiers: KeyModifiers) -> Self {
        Self {
            key,
            modifiers,
            pressed: true,
            repeat: false,
            consumed_by_text_control: false,
        }
    }
}

/// Resolves normalized desktop key events through an immutable effective map.
///
/// A resolver returns at most one command for an event. It only accepts a
/// non-repeat press that was not consumed by a text-capable control; releases,
/// repeats, and all other key/modifier combinations are ignored. The map is the
/// sole source of routing truth, so configured aliases and overrides use the
/// same command path as built-in bindings.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ShortcutResolver {
    keybindings: EffectiveKeybindingMap,
}

impl ShortcutResolver {
    pub fn new(keybindings: EffectiveKeybindingMap) -> Self {
        Self { keybindings }
    }

    pub fn keybindings(&self) -> &EffectiveKeybindingMap {
        &self.keybindings
    }

    pub fn resolve(&self, event: RawKeyEvent) -> Option<EditorCommand> {
        if !event.pressed || event.repeat || event.consumed_by_text_control {
            return None;
        }

        let gesture = KeybindingGesture::new(event.key, event.modifiers);
        self.keybindings
            .action_for(gesture)
            .map(editor_command_for_keybinding_action)
    }
}

/// Resolves one raw event through the supplied immutable effective map.
pub fn resolve_shortcut(
    keybindings: &EffectiveKeybindingMap,
    event: RawKeyEvent,
) -> Option<EditorCommand> {
    if !event.pressed || event.repeat || event.consumed_by_text_control {
        return None;
    }
    keybindings
        .action_for(KeybindingGesture::new(event.key, event.modifiers))
        .map(editor_command_for_keybinding_action)
}

/// Maps every keyboard-configurable action to its shared reducer command.
pub fn editor_command_for_keybinding_action(action: KeybindingAction) -> EditorCommand {
    match action {
        KeybindingAction::FitToWindow => EditorCommand::SetFitToWindow,
        KeybindingAction::ZoomActual => EditorCommand::SetManualZoom { percent: 100 },
        KeybindingAction::Zoom200 => EditorCommand::SetManualZoom { percent: 200 },
        KeybindingAction::ZoomIn => EditorCommand::ZoomByStep {
            direction: ZoomDirection::In,
        },
        KeybindingAction::ZoomOut => EditorCommand::ZoomByStep {
            direction: ZoomDirection::Out,
        },
        KeybindingAction::PanLeft => EditorCommand::PanCanvas {
            direction: PanDirection::Left,
        },
        KeybindingAction::PanDown => EditorCommand::PanCanvas {
            direction: PanDirection::Down,
        },
        KeybindingAction::PanUp => EditorCommand::PanCanvas {
            direction: PanDirection::Up,
        },
        KeybindingAction::PanRight => EditorCommand::PanCanvas {
            direction: PanDirection::Right,
        },
        KeybindingAction::PreviousImage => EditorCommand::Navigate {
            direction: NavigationDirection::Left,
        },
        KeybindingAction::NextImage => EditorCommand::Navigate {
            direction: NavigationDirection::Right,
        },
        KeybindingAction::FirstImage => EditorCommand::Navigate {
            direction: NavigationDirection::Home,
        },
        KeybindingAction::LastImage => EditorCommand::Navigate {
            direction: NavigationDirection::End,
        },
        KeybindingAction::ToggleFullscreen => EditorCommand::ToggleFullscreen,
        KeybindingAction::FlipHorizontal => EditorCommand::FlipHorizontal,
        KeybindingAction::FlipVertical => EditorCommand::FlipVertical,
        KeybindingAction::RotateClockwise90 => EditorCommand::RotateClockwise90,
        KeybindingAction::RotateCounterclockwise90 => EditorCommand::RotateCounterclockwise90,
        KeybindingAction::EnterCrop => EditorCommand::EnterCrop,
        KeybindingAction::FocusBrightness => {
            EditorCommand::FocusAdjustment(AdjustmentKind::Brightness)
        }
        KeybindingAction::FocusContrast => EditorCommand::FocusAdjustment(AdjustmentKind::Contrast),
        KeybindingAction::CommitAdjustment => EditorCommand::CommitAdjustment,
        KeybindingAction::Undo => EditorCommand::Undo,
        KeybindingAction::Redo => EditorCommand::Redo,
        KeybindingAction::IncreaseAdjustment => EditorCommand::IncreaseAdjustment,
        KeybindingAction::DecreaseAdjustment => EditorCommand::DecreaseAdjustment,
    }
}

/// Finds the configured action corresponding to a command control.
///
/// Commands without a keyboard action, such as crop confirmation and export,
/// intentionally return `None` so the UI never advertises a fabricated key.
pub fn keybinding_action_for_command(command: &EditorCommand) -> Option<KeybindingAction> {
    match command {
        EditorCommand::SetFitToWindow => Some(KeybindingAction::FitToWindow),
        EditorCommand::SetManualZoom { percent: 100 } => Some(KeybindingAction::ZoomActual),
        EditorCommand::SetManualZoom { percent: 200 } => Some(KeybindingAction::Zoom200),
        EditorCommand::ZoomByStep {
            direction: ZoomDirection::In,
        } => Some(KeybindingAction::ZoomIn),
        EditorCommand::ZoomByStep {
            direction: ZoomDirection::Out,
        } => Some(KeybindingAction::ZoomOut),
        EditorCommand::PanCanvas {
            direction: PanDirection::Left,
        } => Some(KeybindingAction::PanLeft),
        EditorCommand::PanCanvas {
            direction: PanDirection::Down,
        } => Some(KeybindingAction::PanDown),
        EditorCommand::PanCanvas {
            direction: PanDirection::Up,
        } => Some(KeybindingAction::PanUp),
        EditorCommand::PanCanvas {
            direction: PanDirection::Right,
        } => Some(KeybindingAction::PanRight),
        EditorCommand::Navigate {
            direction: NavigationDirection::Left,
        } => Some(KeybindingAction::PreviousImage),
        EditorCommand::Navigate {
            direction: NavigationDirection::Right,
        } => Some(KeybindingAction::NextImage),
        EditorCommand::Navigate {
            direction: NavigationDirection::Home,
        } => Some(KeybindingAction::FirstImage),
        EditorCommand::Navigate {
            direction: NavigationDirection::End,
        } => Some(KeybindingAction::LastImage),
        EditorCommand::ToggleFullscreen => Some(KeybindingAction::ToggleFullscreen),
        EditorCommand::FlipHorizontal => Some(KeybindingAction::FlipHorizontal),
        EditorCommand::FlipVertical => Some(KeybindingAction::FlipVertical),
        EditorCommand::RotateClockwise90 => Some(KeybindingAction::RotateClockwise90),
        EditorCommand::RotateCounterclockwise90 => Some(KeybindingAction::RotateCounterclockwise90),
        EditorCommand::EnterCrop => Some(KeybindingAction::EnterCrop),
        EditorCommand::FocusAdjustment(AdjustmentKind::Brightness) => {
            Some(KeybindingAction::FocusBrightness)
        }
        EditorCommand::FocusAdjustment(AdjustmentKind::Contrast) => {
            Some(KeybindingAction::FocusContrast)
        }
        EditorCommand::CommitAdjustment => Some(KeybindingAction::CommitAdjustment),
        EditorCommand::Undo => Some(KeybindingAction::Undo),
        EditorCommand::Redo => Some(KeybindingAction::Redo),
        EditorCommand::IncreaseAdjustment => Some(KeybindingAction::IncreaseAdjustment),
        EditorCommand::DecreaseAdjustment => Some(KeybindingAction::DecreaseAdjustment),
        _ => None,
    }
}

/// Returns all configured, platform-visible labels for one action.
///
/// macOS labels use Command and Option; Linux labels use Control and Alt as
/// represented by the current effective map. Actions without bindings return
/// `None` rather than a misleading fixed-table label.
pub fn shortcut_label(
    platform: RuntimePlatform,
    keybindings: &EffectiveKeybindingMap,
    action: KeybindingAction,
) -> Option<String> {
    let gestures = keybindings.gestures_for(action);
    (!gestures.is_empty()).then(|| {
        gestures
            .iter()
            .copied()
            .map(|gesture| format_shortcut_gesture(platform, gesture))
            .collect::<Vec<_>>()
            .join(" / ")
    })
}

fn format_shortcut_gesture(platform: RuntimePlatform, gesture: KeybindingGesture) -> String {
    let mut parts = Vec::<String>::with_capacity(6);
    let mut push = |label: &str| parts.push(label.to_owned());
    match platform {
        RuntimePlatform::MacOs => {
            if gesture.modifiers.control {
                push("Control");
            }
            if gesture.modifiers.command {
                push("Command");
            }
            if gesture.modifiers.option {
                push("Option");
            }
            if gesture.modifiers.alt {
                push("Alt");
            }
        }
        RuntimePlatform::Linux => {
            if gesture.modifiers.control {
                push("Control");
            }
            if gesture.modifiers.command {
                push("Command");
            }
            if gesture.modifiers.alt {
                push("Alt");
            }
            if gesture.modifiers.option {
                push("Option");
            }
        }
    }
    if gesture.modifiers.shift {
        push("Shift");
    }
    parts.push(match gesture.key {
        ShortcutKey::Character(character) => match character {
            '+' => "+".to_owned(),
            character => character.to_ascii_uppercase().to_string(),
        },
        ShortcutKey::ArrowUp => "Up".to_owned(),
        ShortcutKey::ArrowDown => "Down".to_owned(),
        ShortcutKey::ArrowLeft => "Left".to_owned(),
        ShortcutKey::ArrowRight => "Right".to_owned(),
        ShortcutKey::PageUp => "PageUp".to_owned(),
        ShortcutKey::PageDown => "PageDown".to_owned(),
        ShortcutKey::Home => "Home".to_owned(),
        ShortcutKey::End => "End".to_owned(),
        ShortcutKey::Enter => "Return".to_owned(),
        ShortcutKey::Space => "Space".to_owned(),
        ShortcutKey::F11 => "F11".to_owned(),
    });
    parts.join("+")
}

/// A semantic request or typed completion handled by the pure editor reducer.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EditorCommand {
    BeginFolderEnumeration {
        folder: AbsolutePath,
    },
    FolderEnumerated {
        token: RequestToken,
        result: FolderEnumerationPlan,
    },
    /// Selects a collection entry; activation waits for decode completion.
    SelectImage {
        image_id: ImageId,
    },
    /// Resolves a bounded target and requests its decode when one exists.
    Navigate {
        direction: NavigationDirection,
    },
    /// Low-level decode request retained for adapter compatibility.
    BeginDecode {
        candidate: CollectionEntry,
    },
    ImageDecoded {
        token: RequestToken,
        image: CanonicalImage,
    },
    RequestPreview {
        image_id: ImageId,
    },
    PreviewRendered {
        token: RequestToken,
        image: CanonicalImage,
    },
    BeginExport {
        target: AbsolutePath,
        format: ImageFormat,
    },
    ExportWritten {
        token: RequestToken,
    },
    OperationFailed {
        token: RequestToken,
        error: ApplicationError,
    },
    FlipHorizontal,
    FlipVertical,
    RotateClockwise90,
    RotateCounterclockwise90,
    EnterCrop,
    /// Replaces the in-progress crop selection after source-coordinate clamping.
    UpdateCropDraft {
        draft: CropDraft,
    },
    ConfirmCrop,
    CancelCrop,
    FocusAdjustment(AdjustmentKind),
    IncreaseAdjustment,
    DecreaseAdjustment,
    CommitAdjustment,
    Undo,
    Redo,
    /// Updates the preview's available logical size and reclamps any active canvas offset.
    SetPreviewSize {
        preview_size: LogicalSize,
    },
    SetFitToWindow,
    SetManualZoom {
        percent: u16,
    },
    ZoomByStep {
        direction: ZoomDirection,
    },
    PanCanvas {
        direction: PanDirection,
    },
    /// Requests a host-owned full-screen transition without mutating document state.
    ToggleFullscreen,
}

/// The complete output of a pure state transition.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Reduction {
    pub state: EditorState,
    pub effects: Vec<Effect>,
}

/// Immutable root state owned by the desktop adapter.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EditorState {
    capabilities: CapabilitySnapshot,
    browsing: BrowsingState,
    view: ViewState,
    mode: InteractionMode,
    pending: BTreeMap<RequestId, PendingRequest>,
    notices: Vec<VisibleNotice>,
    next_request_id: u64,
    current_enumeration: Option<RequestId>,
    current_decode: Option<RequestId>,
    current_preview: Option<RequestId>,
}

impl EditorState {
    pub fn new(capabilities: CapabilitySnapshot) -> Self {
        Self {
            notices: capabilities.diagnostics().to_vec(),
            capabilities,
            browsing: BrowsingState::default(),
            view: ViewState::default(),
            mode: InteractionMode::Browse,
            pending: BTreeMap::new(),
            next_request_id: 0,
            current_enumeration: None,
            current_decode: None,
            current_preview: None,
        }
    }

    pub fn capabilities(&self) -> &CapabilitySnapshot {
        &self.capabilities
    }

    pub fn browsing(&self) -> &BrowsingState {
        &self.browsing
    }

    pub fn view_state(&self) -> &ViewState {
        &self.view
    }

    pub const fn mode(&self) -> InteractionMode {
        self.mode
    }

    pub fn pending(&self) -> &BTreeMap<RequestId, PendingRequest> {
        &self.pending
    }

    pub fn notices(&self) -> &[VisibleNotice] {
        &self.notices
    }

    fn issue_token(&mut self, revision: Revision) -> RequestToken {
        let request_id = RequestId(self.next_request_id);
        self.next_request_id = self
            .next_request_id
            .checked_add(1)
            .expect("editor request identifier overflow");
        RequestToken {
            request_id,
            revision,
        }
    }

    fn add_error(&mut self, error: ApplicationError) {
        self.notices.push(error.to_notice());
    }

    fn start_preview(&mut self, image_id: ImageId) -> Option<Effect> {
        let document = self.browsing.documents.get(&image_id)?;
        let request = PreviewRequest::from_document(image_id.clone(), document);
        let token = self.issue_token(document.revision());
        self.pending.insert(
            token.request_id,
            PendingRequest::Preview {
                token,
                image_id: image_id.clone(),
            },
        );
        self.current_preview = Some(token.request_id);
        self.browsing.preview = PreviewState::Pending {
            image_id,
            revision: token.revision,
            token,
        };
        Some(Effect::RenderPreview { token, request })
    }

    fn takes_matching_pending(&mut self, token: RequestToken) -> Option<PendingRequest> {
        let pending = self.pending.get(&token.request_id)?;
        if pending.token() != token {
            return None;
        }
        self.pending.remove(&token.request_id)
    }

    fn active_image_size(&self) -> Option<LogicalSize> {
        let image_id = self.browsing.active.as_ref()?;
        let document = self.browsing.documents.get(image_id)?;
        let mut size = LogicalSize::new(document.source.width(), document.source.height());
        for operation in &document.history {
            match operation {
                EditOperation::RotateClockwise90 | EditOperation::RotateCounterclockwise90 => {
                    size = LogicalSize::new(size.height, size.width);
                }
                EditOperation::Crop(crop) => {
                    size = LogicalSize::new(crop.width(), crop.height());
                }
                EditOperation::FlipHorizontal
                | EditOperation::FlipVertical
                | EditOperation::Brightness(_)
                | EditOperation::Contrast(_) => {}
            }
        }
        Some(size)
    }

    fn reset_view_for_active_image(&mut self) {
        if let Some(image_size) = self.active_image_size() {
            self.view = self.view.fit_to_window(image_size);
        }
    }

    fn clamp_view_to_active_image(&mut self) {
        if let Some(image_size) = self.active_image_size() {
            self.view = self.view.clamped_to_image(image_size);
        }
    }

    fn start_decode(&mut self, candidate: CollectionEntry) -> Effect {
        let token = self.issue_token(self.browsing.revision);
        self.pending.insert(
            token.request_id,
            PendingRequest::Decode {
                token,
                candidate: candidate.clone(),
            },
        );
        self.current_decode = Some(token.request_id);
        Effect::DecodeImage { token, candidate }
    }
}

/// Reduces one semantic command without performing I/O or mutating the input.
pub fn reduce(state: &EditorState, command: EditorCommand) -> Reduction {
    let mut state = state.clone();
    let mut effects = Vec::new();

    match command {
        EditorCommand::SetPreviewSize { preview_size } => {
            if let Some(image_size) = state.active_image_size() {
                state.view = state.view.with_preview_size(preview_size, image_size);
            } else {
                state.view.preview_size = preview_size;
                state.view.canvas_offset = LogicalVector::default();
            }
        }
        EditorCommand::SetFitToWindow => {
            if let Some(image_size) = state.active_image_size() {
                state.view = state.view.fit_to_window(image_size);
            }
        }
        EditorCommand::SetManualZoom { percent } => {
            if let Some(image_size) = state.active_image_size() {
                state.view = state.view.set_manual_zoom(percent, image_size);
            }
        }
        EditorCommand::ZoomByStep { direction } => {
            if let Some(image_size) = state.active_image_size() {
                state.view = state.view.zoom_by_step(direction, image_size);
            }
        }
        EditorCommand::PanCanvas { direction } => {
            if let Some(image_size) = state.active_image_size() {
                state.view = state.view.pan(direction, image_size);
            }
        }
        EditorCommand::ToggleFullscreen => {}
        EditorCommand::BeginFolderEnumeration { folder } => {
            state.browsing.revision = state.browsing.revision.next();
            let token = state.issue_token(state.browsing.revision);
            state.pending.insert(
                token.request_id,
                PendingRequest::FolderEnumeration {
                    token,
                    folder: folder.clone(),
                },
            );
            state.current_enumeration = Some(token.request_id);
            effects.push(Effect::EnumerateFolder { token, folder });
        }
        EditorCommand::FolderEnumerated { token, result } => {
            let Some(PendingRequest::FolderEnumeration { folder, .. }) =
                state.takes_matching_pending(token)
            else {
                return Reduction { state, effects };
            };
            let is_current = state.current_enumeration == Some(token.request_id)
                && state.browsing.revision == token.revision;
            if !is_current {
                return Reduction { state, effects };
            }
            state.current_enumeration = None;
            match result {
                FolderEnumerationPlan::Ready(plan) if plan.source_folder() == &folder => {
                    state.browsing.source_folder = Some(plan.source_folder().clone());
                    state.browsing.collection = plan.collection().clone();
                    state.browsing.active = None;
                    state.browsing.documents.clear();
                    state.browsing.preview =
                        PreviewState::for_collection(&state.browsing.collection);
                    state.view = ViewState::for_preview_size(state.view.preview_size);
                    state.mode = InteractionMode::Browse;
                    state.notices = state.capabilities.diagnostics().to_vec();
                    state.notices.extend(plan.availability_notices());
                }
                FolderEnumerationPlan::Ready(_) => state.add_error(ApplicationError::boundary(
                    "folder enumeration",
                    SafeError::new(
                        ErrorCategory::Invariant,
                        "completed folder does not match request",
                    ),
                )),
                FolderEnumerationPlan::Failed(error) => state.add_error(error),
            }
        }
        EditorCommand::SelectImage { image_id } => {
            let candidate = state
                .browsing
                .collection
                .entries()
                .iter()
                .find(|entry| entry.id == image_id)
                .cloned();
            match candidate {
                Some(candidate) => effects.push(state.start_decode(candidate)),
                None => state.add_error(ApplicationError::boundary(
                    "image selection",
                    SafeError::new(
                        ErrorCategory::Invariant,
                        "selected image is not in the current collection",
                    ),
                )),
            }
        }
        EditorCommand::Navigate { direction } => {
            match plan_navigation(
                state.browsing.collection(),
                state.browsing.active(),
                direction,
            ) {
                NavigationTarget::Candidate(image_id) => {
                    let candidate = state
                        .browsing
                        .collection
                        .entries()
                        .iter()
                        .find(|entry| entry.id == image_id)
                        .expect("navigation planner returned a collection image")
                        .clone();
                    effects.push(state.start_decode(candidate));
                }
                NavigationTarget::EmptyCollection => {
                    state.browsing.preview = PreviewState::EmptyCollection;
                }
                NavigationTarget::NoActiveImage | NavigationTarget::NoTarget => {}
            }
        }
        EditorCommand::BeginDecode { candidate } => {
            let is_current_entry = state
                .browsing
                .collection
                .entries()
                .iter()
                .any(|entry| entry == &candidate);
            if !is_current_entry {
                state.add_error(ApplicationError::boundary(
                    "image decode",
                    SafeError::new(
                        ErrorCategory::Invariant,
                        "candidate is not in the current collection",
                    ),
                ));
            } else {
                effects.push(state.start_decode(candidate));
            }
        }
        EditorCommand::ImageDecoded { token, image } => {
            let Some(PendingRequest::Decode { candidate, .. }) =
                state.takes_matching_pending(token)
            else {
                return Reduction { state, effects };
            };
            let is_current = state.current_decode == Some(token.request_id)
                && state.browsing.revision == token.revision
                && state
                    .browsing
                    .collection
                    .entries()
                    .iter()
                    .any(|entry| entry == &candidate);
            if !is_current {
                return Reduction { state, effects };
            }
            state.current_decode = None;
            let image_id = candidate.id.clone();
            state
                .browsing
                .documents
                .entry(image_id.clone())
                .or_insert_with(|| ImageDocument::new(image));
            state.browsing.active = Some(image_id.clone());
            state.mode = InteractionMode::Browse;
            state.reset_view_for_active_image();
            if let Some(effect) = state.start_preview(image_id) {
                effects.push(effect);
            }
        }
        EditorCommand::RequestPreview { image_id } => {
            if let Some(effect) = state.start_preview(image_id) {
                effects.push(effect);
            }
        }
        EditorCommand::PreviewRendered { token, image } => {
            let Some(PendingRequest::Preview { image_id, .. }) =
                state.takes_matching_pending(token)
            else {
                return Reduction { state, effects };
            };
            let is_current = state.current_preview == Some(token.request_id)
                && state
                    .browsing
                    .document(&image_id)
                    .is_some_and(|document| document.revision() == token.revision)
                && matches!(
                    &state.browsing.preview,
                    PreviewState::Pending { token: pending, .. } if *pending == token
                );
            if !is_current {
                return Reduction { state, effects };
            }
            state.current_preview = None;
            state.browsing.preview = PreviewState::Rendered {
                image_id,
                revision: token.revision,
                image,
            };
        }
        EditorCommand::BeginExport { target, format } => {
            let Some(image_id) = state.browsing.active.clone() else {
                state.add_error(ApplicationError::MissingActiveImage {
                    command: CommandName::Export,
                });
                return Reduction { state, effects };
            };
            let document = state
                .browsing
                .document(&image_id)
                .expect("active image must have a document");
            let request =
                ExportRequest::from_document(image_id.clone(), document, target.clone(), format);
            let token = state.issue_token(document.revision());
            state.pending.insert(
                token.request_id,
                PendingRequest::Export {
                    token,
                    image_id,
                    target,
                },
            );
            effects.push(Effect::WriteExport { token, request });
        }
        EditorCommand::ExportWritten { token } => {
            let Some(PendingRequest::Export {
                image_id, target, ..
            }) = state.takes_matching_pending(token)
            else {
                return Reduction { state, effects };
            };
            if state
                .browsing
                .document(&image_id)
                .is_some_and(|document| document.revision() == token.revision)
            {
                state.notices.push(VisibleNotice::new(
                    NoticeSeverity::Info,
                    NoticeSubject::Path(target),
                    SafeError::new(ErrorCategory::FileSystem, "export completed"),
                ));
            }
        }
        EditorCommand::OperationFailed { token, error } => {
            let Some(pending) = state.takes_matching_pending(token) else {
                return Reduction { state, effects };
            };
            let is_current = match &pending {
                PendingRequest::FolderEnumeration { .. } => {
                    state.current_enumeration == Some(token.request_id)
                        && state.browsing.revision == token.revision
                }
                PendingRequest::Decode { .. } => {
                    state.current_decode == Some(token.request_id)
                        && state.browsing.revision == token.revision
                }
                PendingRequest::Preview { image_id, .. } => {
                    state.current_preview == Some(token.request_id)
                        && state
                            .browsing
                            .document(image_id)
                            .is_some_and(|document| document.revision() == token.revision)
                }
                PendingRequest::Export { image_id, .. } => state
                    .browsing
                    .document(image_id)
                    .is_some_and(|document| document.revision() == token.revision),
            };
            if is_current {
                match pending {
                    PendingRequest::FolderEnumeration { .. } => {
                        state.current_enumeration = None;
                        state.add_error(error);
                    }
                    PendingRequest::Decode { candidate, .. } => {
                        state.current_decode = None;
                        let decode_error = match error {
                            ApplicationError::Decode { cause, .. } => ApplicationError::Decode {
                                file_name: candidate.file_name,
                                cause,
                            },
                            ApplicationError::ResourceLimit { .. } => ApplicationError::Decode {
                                file_name: candidate.file_name,
                                cause: SafeError::new(
                                    ErrorCategory::PortableCodec,
                                    "image decode exceeded a resource limit",
                                ),
                            },
                            _ => ApplicationError::Decode {
                                file_name: candidate.file_name,
                                cause: SafeError::new(
                                    ErrorCategory::PortableCodec,
                                    "could not decode image",
                                ),
                            },
                        };
                        state.add_error(decode_error);
                    }
                    PendingRequest::Preview { .. } => {
                        state.current_preview = None;
                        state.add_error(error);
                    }
                    PendingRequest::Export { .. } => state.add_error(error),
                }
            }
        }
        EditorCommand::EnterCrop
        | EditorCommand::UpdateCropDraft { .. }
        | EditorCommand::ConfirmCrop
        | EditorCommand::CancelCrop => {
            let command_name = match command {
                EditorCommand::EnterCrop | EditorCommand::UpdateCropDraft { .. } => {
                    CommandName::EnterCrop
                }
                EditorCommand::ConfirmCrop => CommandName::ConfirmCrop,
                EditorCommand::CancelCrop => CommandName::EnterCrop,
                _ => unreachable!("only crop commands reach this reducer branch"),
            };
            let Some(image_id) = state.browsing.active.clone() else {
                state.add_error(ApplicationError::MissingActiveImage {
                    command: command_name,
                });
                return Reduction { state, effects };
            };

            let crop_dimensions = |state: &EditorState| {
                let document = state
                    .browsing
                    .documents
                    .get(&image_id)
                    .expect("active image must have a document");
                let mut image = document.source.clone();
                for operation in &document.history {
                    image = apply_edit_operation(&image, operation)
                        .expect("reducer-produced edit history must be replayable");
                }
                (image.width, image.height)
            };

            match command {
                EditorCommand::EnterCrop => {
                    let (width, height) = crop_dimensions(&state);
                    state.mode = InteractionMode::Crop(CropDraft::new(0, 0, width, height));
                }
                EditorCommand::UpdateCropDraft { draft } => {
                    if matches!(state.mode, InteractionMode::Crop(_)) {
                        let (width, height) = crop_dimensions(&state);
                        state.mode = InteractionMode::Crop(draft.clamped(width, height));
                    } else {
                        state.add_error(ApplicationError::boundary(
                            "crop selection",
                            SafeError::new(
                                ErrorCategory::Invariant,
                                "crop selection is not active",
                            ),
                        ));
                    }
                }
                EditorCommand::ConfirmCrop => {
                    let InteractionMode::Crop(draft) = state.mode else {
                        state.add_error(ApplicationError::boundary(
                            "crop confirmation",
                            SafeError::new(
                                ErrorCategory::Invariant,
                                "crop selection is not active",
                            ),
                        ));
                        return Reduction { state, effects };
                    };
                    let (width, height) = crop_dimensions(&state);
                    match CropRect::new(
                        width,
                        height,
                        draft.left,
                        draft.top,
                        draft.right,
                        draft.bottom,
                    ) {
                        Ok(crop) => {
                            let document = state
                                .browsing
                                .documents
                                .get_mut(&image_id)
                                .expect("active image must have a document");
                            document.history.push(EditOperation::Crop(crop));
                            document.redo.clear();
                            document.mark_changed();
                            state.mode = InteractionMode::Browse;
                            if let Some(effect) = state.start_preview(image_id) {
                                effects.push(effect);
                            }
                        }
                        Err(reason) => state.add_error(ApplicationError::InvalidCrop { reason }),
                    }
                }
                EditorCommand::CancelCrop => {
                    if matches!(state.mode, InteractionMode::Crop(_)) {
                        state.mode = InteractionMode::Browse;
                    }
                }
                _ => unreachable!("only crop commands reach this reducer branch"),
            }
        }
        command => {
            let geometric_operation = match command {
                EditorCommand::FlipHorizontal => Some(EditOperation::FlipHorizontal),
                EditorCommand::FlipVertical => Some(EditOperation::FlipVertical),
                EditorCommand::RotateClockwise90 => Some(EditOperation::RotateClockwise90),
                EditorCommand::RotateCounterclockwise90 => {
                    Some(EditOperation::RotateCounterclockwise90)
                }
                _ => None,
            };
            let command_name = match command {
                EditorCommand::FlipHorizontal => CommandName::FlipHorizontal,
                EditorCommand::FlipVertical => CommandName::FlipVertical,
                EditorCommand::RotateClockwise90 => CommandName::RotateClockwise90,
                EditorCommand::RotateCounterclockwise90 => CommandName::RotateCounterclockwise90,
                EditorCommand::EnterCrop => CommandName::EnterCrop,
                EditorCommand::FocusAdjustment(AdjustmentKind::Brightness) => {
                    CommandName::FocusBrightness
                }
                EditorCommand::FocusAdjustment(AdjustmentKind::Contrast) => {
                    CommandName::FocusContrast
                }
                EditorCommand::IncreaseAdjustment => CommandName::IncreaseAdjustment,
                EditorCommand::DecreaseAdjustment => CommandName::DecreaseAdjustment,
                EditorCommand::CommitAdjustment => CommandName::CommitAdjustment,
                EditorCommand::Undo => CommandName::Undo,
                EditorCommand::Redo => CommandName::Redo,
                _ => unreachable!("all non-edit commands were handled above"),
            };
            let Some(image_id) = state.browsing.active.clone() else {
                state.add_error(ApplicationError::MissingActiveImage {
                    command: command_name,
                });
                return Reduction { state, effects };
            };

            if let Some(operation) = geometric_operation {
                let document = state
                    .browsing
                    .documents
                    .get_mut(&image_id)
                    .expect("active image must have a document");
                document.history.push(operation);
                document.redo.clear();
                document.mark_changed();
                if let Some(effect) = state.start_preview(image_id) {
                    effects.push(effect);
                }
            } else {
                match command {
                    EditorCommand::FocusAdjustment(kind) => {
                        let document = state
                            .browsing
                            .documents
                            .get_mut(&image_id)
                            .expect("active image must have a document");
                        document.draft.focus(kind);
                        state.mode = InteractionMode::Adjust(kind);
                        if let Some(effect) = state.start_preview(image_id) {
                            effects.push(effect);
                        }
                    }
                    EditorCommand::IncreaseAdjustment | EditorCommand::DecreaseAdjustment => {
                        let changed = {
                            let document = state
                                .browsing
                                .documents
                                .get_mut(&image_id)
                                .expect("active image must have a document");
                            let draft_before = document.draft.clone();
                            match command {
                                EditorCommand::IncreaseAdjustment => {
                                    document.draft.increase_focused();
                                }
                                EditorCommand::DecreaseAdjustment => {
                                    document.draft.decrease_focused();
                                }
                                _ => {
                                    unreachable!("only adjustment step commands reach this branch")
                                }
                            }
                            document.draft != draft_before
                        };
                        // A clamped endpoint is deliberately a no-op: keep the
                        // already-rendered preview visible rather than issuing
                        // a redundant pending preview request.
                        if changed {
                            if let Some(effect) = state.start_preview(image_id) {
                                effects.push(effect);
                            }
                        }
                    }
                    EditorCommand::CommitAdjustment => {
                        let operations = {
                            let document = state
                                .browsing
                                .documents
                                .get_mut(&image_id)
                                .expect("active image must have a document");
                            let operations = document.draft.commit_for_stable_preview();
                            if !operations.is_empty() {
                                document.history.extend(operations.iter().cloned());
                                document.redo.clear();
                                document.mark_changed();
                            }
                            operations
                        };
                        if !operations.is_empty() {
                            state.mode = InteractionMode::Browse;
                            if let Some(effect) = state.start_preview(image_id) {
                                effects.push(effect);
                            }
                        }
                    }
                    EditorCommand::Undo | EditorCommand::Redo => {
                        let changed = {
                            let document = state
                                .browsing
                                .documents
                                .get_mut(&image_id)
                                .expect("active image must have a document");
                            let operation = match command {
                                EditorCommand::Undo => document.history.pop(),
                                EditorCommand::Redo => document.redo.pop(),
                                _ => unreachable!("only history commands reach this branch"),
                            };

                            if let Some(operation) = operation {
                                match command {
                                    EditorCommand::Undo => document.redo.push(operation),
                                    EditorCommand::Redo => document.history.push(operation),
                                    _ => unreachable!("only history commands reach this branch"),
                                }
                                document.mark_changed();
                                true
                            } else {
                                false
                            }
                        };

                        if changed {
                            if let Some(effect) = state.start_preview(image_id) {
                                effects.push(effect);
                            }
                        }
                    }
                    _ => unreachable!("only edit commands reach this reducer branch"),
                }
            }
        }
    }

    state.clamp_view_to_active_image();
    Reduction { state, effects }
}
