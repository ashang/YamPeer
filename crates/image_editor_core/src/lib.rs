//! Platform-independent domain contracts for the Image Editor.
//!
//! This crate deliberately has no UI, OS, codec, or asynchronous-runtime
//! dependencies. Its constructors make the invariant-bearing values used at
//! every adapter boundary impossible to construct accidentally.

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
