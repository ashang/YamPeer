//! Codec capability detection and bounded image adapter registration.
//!
//! `portable-codecs` provides JPEG, PNG, and TIFF through `image-rs`. HEIC is
//! intentionally an injected optional adapter: it is registered only after its
//! runtime probe reports a decoder and/or encoder, so linking never implies
//! runtime availability.

use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

#[cfg(feature = "portable-codecs")]
use image::ImageDecoder;
#[cfg(any(feature = "portable-codecs", feature = "heic"))]
use image_editor_core::Rgba16;
use image_editor_core::{
    AbsolutePath, Availability, AvailabilityReason, CanonicalImage, CapabilitySnapshot,
    CodecProvider, FormatCapability, ImageFormat, PlatformCapability, ResourceLimitKind,
};
#[cfg(feature = "heic")]
use libheif_rs::{
    Channel, ColorSpace, CompressionFormat, EncoderQuality, HeifContext, Image as HeifImage,
    LibHeif, RgbChroma,
};

pub use image_editor_core::{ApplicationError, ErrorCategory, Result, SafeError};

type ProbeResult<T> = std::result::Result<T, CodecProbeError>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CompiledCodecFeatures {
    pub portable_codecs: bool,
    pub heic_adapter: bool,
}

/// Compile-time linkage facts only; these are not runtime capability claims.
pub const COMPILED_FEATURES: CompiledCodecFeatures = CompiledCodecFeatures {
    portable_codecs: cfg!(feature = "portable-codecs"),
    heic_adapter: cfg!(feature = "heic"),
};

/// Platform capabilities obtained before open-folder or export commands are enabled.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StartupPlatformCapabilities {
    pub folder_picker: PlatformCapability,
    pub save_picker: PlatformCapability,
}

impl StartupPlatformCapabilities {
    pub fn new(folder_picker: PlatformCapability, save_picker: PlatformCapability) -> Self {
        Self {
            folder_picker,
            save_picker,
        }
    }
}

/// Error from a capability self-check or runtime codec probe.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CodecProbeError {
    message: String,
}

impl CodecProbeError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl std::fmt::Display for CodecProbeError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}
impl std::error::Error for CodecProbeError {}

/// Hard limits applied before and during image decode.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DecodeLimits {
    pub max_input_bytes: u64,
    pub max_width: u32,
    pub max_height: u32,
    pub max_total_pixels: u64,
    /// Covers the decoder output and canonical RGBA16 conversion allocation.
    pub max_intermediate_bytes: u64,
}

impl DecodeLimits {
    pub const DEFAULT: Self = Self {
        max_input_bytes: 64 * 1024 * 1024,
        max_width: 16_384,
        max_height: 16_384,
        max_total_pixels: 100_000_000,
        max_intermediate_bytes: 512 * 1024 * 1024,
    };

    #[cfg(any(feature = "portable-codecs", feature = "heic"))]
    fn check_dimensions(self, width: u32, height: u32) -> std::result::Result<(), CodecError> {
        if width > self.max_width || height > self.max_height {
            return Err(CodecError::ResourceLimit(ResourceLimitKind::Dimensions));
        }
        let pixels = u64::from(width)
            .checked_mul(u64::from(height))
            .ok_or(CodecError::ResourceLimit(ResourceLimitKind::TotalPixels))?;
        if pixels > self.max_total_pixels {
            return Err(CodecError::ResourceLimit(ResourceLimitKind::TotalPixels));
        }
        let canonical_bytes = pixels.checked_mul(8).ok_or(CodecError::ResourceLimit(
            ResourceLimitKind::IntermediateAllocation,
        ))?;
        if canonical_bytes > self.max_intermediate_bytes {
            return Err(CodecError::ResourceLimit(
                ResourceLimitKind::IntermediateAllocation,
            ));
        }
        Ok(())
    }
}

impl Default for DecodeLimits {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// The operation that was unavailable on a registered codec.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CodecOperation {
    Decode,
    Encode,
}

/// A typed codec failure. In particular, `Unavailable` is never used for
/// malformed content, which must remain a `Content` failure.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CodecError {
    Unavailable {
        format: ImageFormat,
        operation: CodecOperation,
        reason: String,
    },
    ResourceLimit(ResourceLimitKind),
    Content {
        format: ImageFormat,
        message: String,
    },
    Input {
        message: String,
    },
    Output {
        format: ImageFormat,
        message: String,
    },
}

impl CodecError {
    fn unavailable(
        format: ImageFormat,
        operation: CodecOperation,
        availability: &Availability,
    ) -> Self {
        let reason = match availability {
            Availability::Available => "no registered adapter is available".to_owned(),
            Availability::Unavailable { reason } => reason.summary().to_owned(),
        };
        Self::Unavailable {
            format,
            operation,
            reason,
        }
    }
}

impl std::fmt::Display for CodecError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unavailable {
                format,
                operation,
                reason,
            } => write!(
                formatter,
                "{} {:?} is unavailable: {reason}",
                format.display_name(),
                operation
            ),
            Self::ResourceLimit(limit) => {
                write!(formatter, "decode resource limit exceeded: {limit:?}")
            }
            Self::Content { format, message } => write!(
                formatter,
                "invalid {} content: {message}",
                format.display_name()
            ),
            Self::Input { message } => write!(formatter, "could not read image input: {message}"),
            Self::Output { format, message } => write!(
                formatter,
                "could not encode {} output: {message}",
                format.display_name()
            ),
        }
    }
}
impl std::error::Error for CodecError {}

/// The bounded, normalized source consumed by the shared pipeline.
pub type DecodedSource = CanonicalImage;

/// Dispatch boundary between platform workers and registered codecs.
pub trait ImageCodec: Send + Sync {
    fn capability(&self, format: ImageFormat) -> FormatCapability;
    fn decode(
        &self,
        path: &AbsolutePath,
        limits: DecodeLimits,
    ) -> std::result::Result<DecodedSource, CodecError>;
    fn encode(
        &self,
        image: &CanonicalImage,
        format: ImageFormat,
        destination: &mut dyn Write,
    ) -> std::result::Result<(), CodecError>;
}

/// In-process JPEG/PNG/TIFF self-check boundary.
pub trait PortableCodecSelfCheck: Send + Sync {
    fn check_decode(&self, format: ImageFormat, fixture: &[u8]) -> ProbeResult<()>;
    fn check_encode(&self, format: ImageFormat) -> ProbeResult<()>;
}

/// Runtime probe boundary for libheif and its independently installed plugins.
pub trait HeicRuntimeProbe: Send + Sync {
    fn check_decode(&self) -> ProbeResult<()>;
    fn check_encode(&self) -> ProbeResult<()>;
}

/// Capability registry plus codecs accepted for the current application session.
#[derive(Clone)]
pub struct CodecRegistry {
    snapshot: CapabilitySnapshot,
    codecs: BTreeMap<ImageFormat, Arc<dyn ImageCodec>>,
}

impl std::fmt::Debug for CodecRegistry {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("CodecRegistry")
            .field("snapshot", &self.snapshot)
            .field("registered_formats", &self.codecs.keys())
            .finish()
    }
}

impl CodecRegistry {
    /// Runs portable self-checks and registers portable dispatch where compiled.
    pub fn detect(platform: StartupPlatformCapabilities) -> Self {
        #[cfg(feature = "portable-codecs")]
        let portable = ImageRsPortableSelfCheck;
        #[cfg(feature = "portable-codecs")]
        let portable = Some(&portable as &dyn PortableCodecSelfCheck);
        #[cfg(not(feature = "portable-codecs"))]
        let portable: Option<&dyn PortableCodecSelfCheck> = None;

        let mut registry = Self::detect_with(platform, portable, &UnconfiguredHeicProbe);
        #[cfg(feature = "portable-codecs")]
        registry.register(Arc::new(PortableImageCodec));
        #[cfg(feature = "heic")]
        registry.register_optional_heic(&LibHeifRuntimeProbe, Arc::new(HeicImageCodec));
        registry
    }

    /// Builds a complete immutable snapshot from real or test probe adapters.
    pub fn detect_with(
        platform: StartupPlatformCapabilities,
        portable: Option<&dyn PortableCodecSelfCheck>,
        heic: &dyn HeicRuntimeProbe,
    ) -> Self {
        let mut formats = BTreeMap::new();
        for format in [ImageFormat::Jpeg, ImageFormat::Png, ImageFormat::Tiff] {
            formats.insert(format, portable_capability(format, portable));
        }
        formats.insert(ImageFormat::Heic, heic_capability(heic));
        Self {
            snapshot: CapabilitySnapshot::new(
                formats,
                platform.folder_picker,
                platform.save_picker,
            ),
            codecs: BTreeMap::new(),
        }
    }

    /// Registers an adapter only for format directions that the startup snapshot
    /// independently marked available.
    pub fn register(&mut self, codec: Arc<dyn ImageCodec>) {
        for format in [
            ImageFormat::Jpeg,
            ImageFormat::Png,
            ImageFormat::Tiff,
            ImageFormat::Heic,
        ] {
            let detected = self.snapshot.format(format);
            let provided = codec.capability(format);
            if (detected.can_decode() && provided.can_decode())
                || (detected.can_encode() && provided.can_encode())
            {
                self.codecs.insert(format, Arc::clone(&codec));
            }
        }
    }

    /// Probes HEIC directions at runtime, replaces its snapshot capability, and
    /// registers the adapter only if at least one operation is actually usable.
    pub fn register_optional_heic(
        &mut self,
        probe: &dyn HeicRuntimeProbe,
        codec: Arc<dyn ImageCodec>,
    ) {
        let mut formats = self.snapshot.formats().clone();
        let runtime = heic_capability(probe);
        let provided = codec.capability(ImageFormat::Heic);
        let capability = FormatCapability::new(
            combine_availability(&runtime.decode, &provided.decode),
            combine_availability(&runtime.encode, &provided.encode),
            Some(CodecProvider::Libheif),
        );
        formats.insert(ImageFormat::Heic, capability.clone());
        self.snapshot = CapabilitySnapshot::new(
            formats,
            self.snapshot.folder_picker().clone(),
            self.snapshot.save_picker().clone(),
        );
        self.codecs.remove(&ImageFormat::Heic);
        if capability.can_decode() || capability.can_encode() {
            self.codecs.insert(ImageFormat::Heic, codec);
        }
    }

    pub fn snapshot(&self) -> &CapabilitySnapshot {
        &self.snapshot
    }
    pub fn into_snapshot(self) -> CapabilitySnapshot {
        self.snapshot
    }

    pub fn decode(
        &self,
        format: ImageFormat,
        path: &AbsolutePath,
        limits: DecodeLimits,
    ) -> std::result::Result<DecodedSource, CodecError> {
        let capability = self.snapshot.format(format);
        if !capability.can_decode() {
            return Err(CodecError::unavailable(
                format,
                CodecOperation::Decode,
                &capability.decode,
            ));
        }
        let codec = self.codecs.get(&format).ok_or_else(|| {
            CodecError::unavailable(format, CodecOperation::Decode, &capability.decode)
        })?;
        codec.decode(path, limits)
    }

    pub fn encode(
        &self,
        image: &CanonicalImage,
        format: ImageFormat,
        destination: &mut dyn Write,
    ) -> std::result::Result<(), CodecError> {
        let capability = self.snapshot.format(format);
        if !capability.can_encode() {
            return Err(CodecError::unavailable(
                format,
                CodecOperation::Encode,
                &capability.encode,
            ));
        }
        let codec = self.codecs.get(&format).ok_or_else(|| {
            CodecError::unavailable(format, CodecOperation::Encode, &capability.encode)
        })?;
        codec.encode(image, format, destination)
    }
}

/// Executes one reducer-authorized export request without mutating editor state.
///
/// The request is already an immutable snapshot of the selected document. This
/// worker replays that snapshot at full resolution, encodes to an exclusively
/// created sibling temporary file, flushes it, then creates the target through
/// a no-replacement hard link. The temporary link is removed only after the
/// target exists. Every failure removes only the temporary file and (if that
/// final cleanup itself fails) the target created by this attempt.
pub fn execute_export_request(
    registry: &CodecRegistry,
    request: &image_editor_core::ExportRequest,
) -> image_editor_core::Result<()> {
    let rendered = image_editor_core::render_current_editing_result(
        &request.source,
        &request.history,
        &request.draft,
    )
    .map_err(|_| export_error(request, ErrorCategory::Invariant, "could not render export"))?;

    let (temporary_path, mut file) = create_sibling_temporary_file(request)?;
    if let Err(error) = registry.encode(&rendered, request.format, &mut file) {
        discard_attempt_file(&temporary_path);
        return Err(export_codec_error(request, error));
    }
    if let Err(error) = file.flush() {
        discard_attempt_file(&temporary_path);
        return Err(export_io_error(request, "could not flush export", error));
    }
    if let Err(error) = file.sync_all() {
        discard_attempt_file(&temporary_path);
        return Err(export_io_error(request, "could not sync export", error));
    }
    drop(file);

    if let Err(error) =
        publish_without_replacement(&temporary_path, Path::new(request.target.as_str()))
    {
        discard_attempt_file(&temporary_path);
        return Err(export_io_error(request, "could not publish export", error));
    }

    Ok(())
}

/// Converts export worker output into the typed completion consumed by the
/// reducer. The reducer checks the original request token and revision before
/// showing either completion, so stale worker results cannot alter new state.
pub fn complete_export_request(
    registry: &CodecRegistry,
    token: image_editor_core::RequestToken,
    request: image_editor_core::ExportRequest,
) -> image_editor_core::EditorCommand {
    match execute_export_request(registry, &request) {
        Ok(()) => image_editor_core::EditorCommand::ExportWritten { token },
        Err(error) => image_editor_core::EditorCommand::OperationFailed { token, error },
    }
}

static NEXT_TEMPORARY_EXPORT_ID: AtomicU64 = AtomicU64::new(0);

fn create_sibling_temporary_file(
    request: &image_editor_core::ExportRequest,
) -> image_editor_core::Result<(PathBuf, File)> {
    let target = Path::new(request.target.as_str());
    let parent = target.parent().ok_or_else(|| {
        export_error(
            request,
            ErrorCategory::FileSystem,
            "export target has no parent directory",
        )
    })?;
    let name = target
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| {
            export_error(
                request,
                ErrorCategory::FileSystem,
                "export target has no UTF-8 filename",
            )
        })?;

    for _ in 0..128 {
        let sequence = NEXT_TEMPORARY_EXPORT_ID.fetch_add(1, Ordering::Relaxed);
        let path = parent.join(format!(
            ".{name}.image-editor-{}-{sequence}.tmp",
            std::process::id()
        ));
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(file) => return Ok((path, file)),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(export_io_error(request, "could not create export", error)),
        }
    }

    Err(export_error(
        request,
        ErrorCategory::FileSystem,
        "could not allocate a unique temporary export file",
    ))
}

/// Publishes by linking the attempt-created sibling file to a new target.
/// `hard_link` fails if the target appeared after planning, unlike rename which
/// can replace it. macOS and Linux both support this for same-directory files.
fn publish_without_replacement(temporary_path: &Path, target: &Path) -> std::io::Result<()> {
    fs::hard_link(temporary_path, target)?;
    if let Err(error) = fs::remove_file(temporary_path) {
        // Both files are known to have been created by this attempt. Remove
        // them best-effort so a failed publication leaves no export behind.
        let _ = fs::remove_file(target);
        let _ = fs::remove_file(temporary_path);
        return Err(error);
    }
    Ok(())
}

fn discard_attempt_file(path: &Path) {
    let _ = fs::remove_file(path);
}

fn export_error(
    request: &image_editor_core::ExportRequest,
    category: ErrorCategory,
    summary: impl Into<String>,
) -> image_editor_core::ApplicationError {
    image_editor_core::ApplicationError::ExportWrite {
        path: request.target.clone(),
        cause: SafeError::new(category, summary),
    }
}

fn export_io_error(
    request: &image_editor_core::ExportRequest,
    operation: &str,
    error: std::io::Error,
) -> image_editor_core::ApplicationError {
    export_error(
        request,
        ErrorCategory::FileSystem,
        format!("{operation}: {}", error.kind()),
    )
}

fn export_codec_error(
    request: &image_editor_core::ExportRequest,
    error: CodecError,
) -> image_editor_core::ApplicationError {
    let category = match error {
        CodecError::Unavailable { .. } => ErrorCategory::OptionalDependency,
        CodecError::ResourceLimit(_) => ErrorCategory::ResourceLimit,
        CodecError::Content { .. } | CodecError::Input { .. } | CodecError::Output { .. } => {
            ErrorCategory::PortableCodec
        }
    };
    export_error(
        request,
        category,
        format!("could not encode {} export", request.format.display_name()),
    )
}

fn combine_availability(runtime: &Availability, adapter: &Availability) -> Availability {
    match (runtime, adapter) {
        (Availability::Available, Availability::Available) => Availability::Available,
        (Availability::Unavailable { reason }, _) | (_, Availability::Unavailable { reason }) => {
            Availability::Unavailable {
                reason: reason.clone(),
            }
        }
    }
}

fn portable_capability(
    format: ImageFormat,
    portable: Option<&dyn PortableCodecSelfCheck>,
) -> FormatCapability {
    let Some(portable) = portable else {
        let reason = AvailabilityReason::new(format!(
            "{} decoding and encoding are unavailable because portable codecs were not compiled",
            format.display_name()
        ));
        return FormatCapability::new(
            Availability::Unavailable {
                reason: reason.clone(),
            },
            Availability::Unavailable { reason },
            None,
        );
    };
    let decode = match portable_fixture(format) {
        Some(fixture) => availability_from(portable.check_decode(format, &fixture)),
        None => unavailable(format!(
            "{} decoding self-check fixture is not available",
            format.display_name()
        )),
    };
    let encode = availability_from(portable.check_encode(format));
    FormatCapability::new(decode, encode, Some(CodecProvider::PortableRust))
}

fn heic_capability(heic: &dyn HeicRuntimeProbe) -> FormatCapability {
    FormatCapability::new(
        availability_from(heic.check_decode()),
        availability_from(heic.check_encode()),
        Some(CodecProvider::Libheif),
    )
}
fn availability_from(result: ProbeResult<()>) -> Availability {
    match result {
        Ok(()) => Availability::Available,
        Err(error) => unavailable(error.message),
    }
}
fn unavailable(reason: impl Into<String>) -> Availability {
    Availability::Unavailable {
        reason: AvailabilityReason::new(reason),
    }
}

struct UnconfiguredHeicProbe;
impl HeicRuntimeProbe for UnconfiguredHeicProbe {
    fn check_decode(&self) -> ProbeResult<()> {
        Err(CodecProbeError::new(
            "HEIC decoding is unavailable because no libheif runtime probe was configured",
        ))
    }
    fn check_encode(&self) -> ProbeResult<()> {
        Err(CodecProbeError::new(
            "HEIC encoding is unavailable because no libheif runtime probe was configured",
        ))
    }
}

#[cfg(feature = "portable-codecs")]
struct ImageRsPortableSelfCheck;
#[cfg(feature = "portable-codecs")]
impl PortableCodecSelfCheck for ImageRsPortableSelfCheck {
    fn check_decode(&self, format: ImageFormat, fixture: &[u8]) -> ProbeResult<()> {
        image::load_from_memory_with_format(fixture, image_format(format))
            .map(|_| ())
            .map_err(|error| {
                CodecProbeError::new(format!(
                    "{} decoding self-check failed: {error}",
                    format.display_name()
                ))
            })
    }
    fn check_encode(&self, format: ImageFormat) -> ProbeResult<()> {
        let image = image::DynamicImage::new_rgb8(1, 1);
        image
            .write_to(&mut std::io::Cursor::new(Vec::new()), image_format(format))
            .map_err(|error| {
                CodecProbeError::new(format!(
                    "{} encoding self-check failed: {error}",
                    format.display_name()
                ))
            })
    }
}

#[cfg(feature = "portable-codecs")]
fn portable_fixture(format: ImageFormat) -> Option<Vec<u8>> {
    let image = image::DynamicImage::new_rgb8(1, 1);
    let mut sink = std::io::Cursor::new(Vec::new());
    image.write_to(&mut sink, image_format(format)).ok()?;
    Some(sink.into_inner())
}
#[cfg(not(feature = "portable-codecs"))]
fn portable_fixture(_: ImageFormat) -> Option<Vec<u8>> {
    None
}

#[cfg(feature = "portable-codecs")]
fn image_format(format: ImageFormat) -> image::ImageFormat {
    match format {
        ImageFormat::Jpeg => image::ImageFormat::Jpeg,
        ImageFormat::Png => image::ImageFormat::Png,
        ImageFormat::Tiff => image::ImageFormat::Tiff,
        ImageFormat::Heic => unreachable!("HEIC is never sent to image-rs portable codecs"),
    }
}

/// `image-rs` adapter for all portable codecs. It never guesses by extension:
/// the registry dispatches a capability-checked requested `ImageFormat`.
#[cfg(feature = "portable-codecs")]
#[derive(Debug, Default)]
pub struct PortableImageCodec;

#[cfg(feature = "portable-codecs")]
impl ImageCodec for PortableImageCodec {
    fn capability(&self, format: ImageFormat) -> FormatCapability {
        if matches!(
            format,
            ImageFormat::Jpeg | ImageFormat::Png | ImageFormat::Tiff
        ) {
            FormatCapability::new(
                Availability::Available,
                Availability::Available,
                Some(CodecProvider::PortableRust),
            )
        } else {
            let reason =
                AvailabilityReason::new("HEIC is not provided by the portable image-rs adapter");
            FormatCapability::new(
                Availability::Unavailable {
                    reason: reason.clone(),
                },
                Availability::Unavailable { reason },
                None,
            )
        }
    }

    fn decode(
        &self,
        path: &AbsolutePath,
        limits: DecodeLimits,
    ) -> std::result::Result<DecodedSource, CodecError> {
        let format = Path::new(path.as_str())
            .extension()
            .and_then(|value| value.to_str())
            .and_then(ImageFormat::from_extension)
            .ok_or_else(|| CodecError::Input {
                message: "source path has no supported image extension".to_owned(),
            })?;
        let bytes = read_bounded(Path::new(path.as_str()), limits.max_input_bytes)?;
        let mut reader =
            image::ImageReader::with_format(std::io::Cursor::new(bytes), image_format(format));
        let mut image_limits = image::Limits::default();
        image_limits.max_image_width = Some(limits.max_width);
        image_limits.max_image_height = Some(limits.max_height);
        image_limits.max_alloc = Some(limits.max_intermediate_bytes);
        reader.limits(image_limits);
        let mut decoder = reader
            .into_decoder()
            .map_err(|error| map_decode_error(error, format))?;
        let (width, height) = decoder.dimensions();
        limits.check_dimensions(width, height)?;
        let orientation = decoder
            .orientation()
            .map_err(|error| map_decode_error(error, format))?;
        let dynamic = image::DynamicImage::from_decoder(decoder)
            .map_err(|error| map_decode_error(error, format))?;
        canonical_from_dynamic(dynamic, orientation)
    }

    fn encode(
        &self,
        image: &CanonicalImage,
        format: ImageFormat,
        destination: &mut dyn Write,
    ) -> std::result::Result<(), CodecError> {
        let mut sink = std::io::Cursor::new(Vec::new());
        match format {
            ImageFormat::Png | ImageFormat::Tiff => {
                dynamic_rgba16(image)?.write_to(&mut sink, image_format(format))
            }
            ImageFormat::Jpeg => dynamic_rgb8(image)?.write_to(&mut sink, image_format(format)),
            ImageFormat::Heic => {
                return Err(CodecError::Unavailable {
                    format,
                    operation: CodecOperation::Encode,
                    reason: "HEIC is not provided by the portable image-rs adapter".to_owned(),
                });
            }
        }
        .map_err(|error| CodecError::Output {
            format,
            message: error.to_string(),
        })?;
        destination
            .write_all(&sink.into_inner())
            .map_err(|error| CodecError::Output {
                format,
                message: error.to_string(),
            })
    }
}

#[cfg(any(feature = "portable-codecs", feature = "heic"))]
fn read_bounded(path: &Path, max_input_bytes: u64) -> std::result::Result<Vec<u8>, CodecError> {
    let file = std::fs::File::open(path).map_err(|error| CodecError::Input {
        message: error.to_string(),
    })?;
    let mut reader = std::io::Read::take(file, max_input_bytes.saturating_add(1));
    let mut bytes = Vec::new();
    std::io::Read::read_to_end(&mut reader, &mut bytes).map_err(|error| CodecError::Input {
        message: error.to_string(),
    })?;
    if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > max_input_bytes {
        return Err(CodecError::ResourceLimit(ResourceLimitKind::InputBytes));
    }
    Ok(bytes)
}

#[cfg(feature = "portable-codecs")]
fn map_decode_error(error: image::ImageError, format: ImageFormat) -> CodecError {
    if matches!(error, image::ImageError::Limits(_)) {
        CodecError::ResourceLimit(ResourceLimitKind::IntermediateAllocation)
    } else {
        CodecError::Content {
            format,
            message: error.to_string(),
        }
    }
}

#[cfg(feature = "portable-codecs")]
fn canonical_from_dynamic(
    dynamic: image::DynamicImage,
    orientation: image::metadata::Orientation,
) -> std::result::Result<CanonicalImage, CodecError> {
    let rgba = dynamic.into_rgba16();
    let (width, height) = rgba.dimensions();
    let pixels = rgba
        .as_raw()
        .chunks_exact(4)
        .map(|sample| Rgba16::new(sample[0], sample[1], sample[2], sample[3]))
        .collect();
    let decoded = image_editor_core::DecodedImage::new(
        width,
        height,
        pixels,
        image_editor_core::DecodedAlphaMode::Straight,
        source_orientation(orientation),
    )
    .map_err(|error| CodecError::Content {
        format: ImageFormat::Png,
        message: format!("decoded image violates canonical invariants: {error:?}"),
    })?;
    image_editor_core::normalize_decoded_image(decoded).map_err(|error| CodecError::Content {
        format: ImageFormat::Png,
        message: format!("could not normalize decoded image: {error:?}"),
    })
}

#[cfg(feature = "portable-codecs")]
fn source_orientation(
    orientation: image::metadata::Orientation,
) -> image_editor_core::SourceOrientation {
    use image::metadata::Orientation::*;
    match orientation {
        NoTransforms => image_editor_core::SourceOrientation::TopLeft,
        FlipHorizontal => image_editor_core::SourceOrientation::TopRight,
        Rotate180 => image_editor_core::SourceOrientation::BottomRight,
        FlipVertical => image_editor_core::SourceOrientation::BottomLeft,
        Rotate90FlipH => image_editor_core::SourceOrientation::LeftTop,
        Rotate90 => image_editor_core::SourceOrientation::RightTop,
        Rotate270FlipH => image_editor_core::SourceOrientation::RightBottom,
        Rotate270 => image_editor_core::SourceOrientation::LeftBottom,
    }
}

#[cfg(feature = "portable-codecs")]
fn dynamic_rgba16(image: &CanonicalImage) -> std::result::Result<image::DynamicImage, CodecError> {
    let samples = image
        .pixels()
        .iter()
        .flat_map(|pixel| [pixel.red, pixel.green, pixel.blue, pixel.alpha])
        .collect();
    let buffer = image::ImageBuffer::<image::Rgba<u16>, Vec<u16>>::from_raw(
        image.width(),
        image.height(),
        samples,
    )
    .ok_or_else(|| CodecError::Output {
        format: ImageFormat::Png,
        message: "canonical image pixel count is invalid".to_owned(),
    })?;
    Ok(image::DynamicImage::ImageRgba16(buffer))
}

#[cfg(feature = "portable-codecs")]
fn dynamic_rgb8(image: &CanonicalImage) -> std::result::Result<image::DynamicImage, CodecError> {
    let samples = image
        .pixels()
        .iter()
        .flat_map(|pixel| {
            [
                (pixel.red >> 8) as u8,
                (pixel.green >> 8) as u8,
                (pixel.blue >> 8) as u8,
            ]
        })
        .collect();
    let buffer =
        image::RgbImage::from_raw(image.width(), image.height(), samples).ok_or_else(|| {
            CodecError::Output {
                format: ImageFormat::Jpeg,
                message: "canonical image pixel count is invalid".to_owned(),
            }
        })?;
    Ok(image::DynamicImage::ImageRgb8(buffer))
}

/// Feature-gated runtime probe for libheif plugins. A linked library is not
/// enough: the required HEVC decoder or encoder plugin must be discoverable.
#[cfg(feature = "heic")]
pub struct LibHeifRuntimeProbe;

#[cfg(feature = "heic")]
impl HeicRuntimeProbe for LibHeifRuntimeProbe {
    fn check_decode(&self) -> ProbeResult<()> {
        let libheif = LibHeif::new_checked().map_err(heic_probe_error)?;
        if libheif
            .decoder_descriptors(1, Some(CompressionFormat::Hevc))
            .is_empty()
        {
            return Err(CodecProbeError::new(
                "libheif has no runtime HEVC decoder plugin",
            ));
        }
        Ok(())
    }

    fn check_encode(&self) -> ProbeResult<()> {
        let libheif = LibHeif::new_checked().map_err(heic_probe_error)?;
        if libheif
            .encoder_descriptors(1, Some(CompressionFormat::Hevc), None)
            .is_empty()
        {
            return Err(CodecProbeError::new(
                "libheif has no runtime HEVC encoder plugin",
            ));
        }
        Ok(())
    }
}

#[cfg(feature = "heic")]
fn heic_probe_error(error: libheif_rs::HeifError) -> CodecProbeError {
    CodecProbeError::new(format!("libheif runtime initialization failed: {error}"))
}

/// Optional HEIC implementation. It is registered only after
/// `LibHeifRuntimeProbe` confirms the corresponding plugin direction. The
/// adapter constructs a fresh libheif guard for each operation, avoiding a
/// process-wide dependency when the `heic` feature is disabled.
#[cfg(feature = "heic")]
#[derive(Debug, Default)]
pub struct HeicImageCodec;

#[cfg(feature = "heic")]
impl ImageCodec for HeicImageCodec {
    fn capability(&self, format: ImageFormat) -> FormatCapability {
        if format == ImageFormat::Heic {
            FormatCapability::new(
                Availability::Available,
                Availability::Available,
                Some(CodecProvider::Libheif),
            )
        } else {
            let reason = AvailabilityReason::new("this adapter only provides HEIC");
            FormatCapability::new(
                Availability::Unavailable {
                    reason: reason.clone(),
                },
                Availability::Unavailable { reason },
                None,
            )
        }
    }

    fn decode(
        &self,
        path: &AbsolutePath,
        limits: DecodeLimits,
    ) -> std::result::Result<DecodedSource, CodecError> {
        let bytes = read_bounded(Path::new(path.as_str()), limits.max_input_bytes)?;
        let libheif = LibHeif::new_checked().map_err(|error| CodecError::Content {
            format: ImageFormat::Heic,
            message: format!("libheif initialization failed: {error}"),
        })?;
        let context =
            HeifContext::read_from_bytes(&bytes).map_err(|error| CodecError::Content {
                format: ImageFormat::Heic,
                message: error.to_string(),
            })?;
        let handle = context
            .primary_image_handle()
            .map_err(|error| CodecError::Content {
                format: ImageFormat::Heic,
                message: error.to_string(),
            })?;
        limits.check_dimensions(handle.width(), handle.height())?;
        let decoded = libheif
            .decode(&handle, ColorSpace::Rgb(RgbChroma::Rgba), None)
            .map_err(|error| CodecError::Content {
                format: ImageFormat::Heic,
                message: error.to_string(),
            })?;
        canonical_from_heif(decoded, limits)
    }

    fn encode(
        &self,
        image: &CanonicalImage,
        format: ImageFormat,
        destination: &mut dyn Write,
    ) -> std::result::Result<(), CodecError> {
        if format != ImageFormat::Heic {
            return Err(CodecError::Unavailable {
                format,
                operation: CodecOperation::Encode,
                reason: "the libheif adapter only encodes HEIC".to_owned(),
            });
        }
        let libheif = LibHeif::new_checked().map_err(|error| CodecError::Output {
            format,
            message: format!("libheif initialization failed: {error}"),
        })?;
        let mut encoded = HeifImage::new(
            image.width(),
            image.height(),
            ColorSpace::Rgb(RgbChroma::C444),
        )
        .map_err(|error| CodecError::Output {
            format,
            message: error.to_string(),
        })?;
        for channel in [Channel::R, Channel::G, Channel::B] {
            encoded
                .create_plane(channel, image.width(), image.height(), 8)
                .map_err(|error| CodecError::Output {
                    format,
                    message: error.to_string(),
                })?;
        }
        {
            let mut planes = encoded.planes_mut();
            copy_heif_channel(
                planes.r.as_mut().ok_or_else(|| CodecError::Output {
                    format,
                    message: "libheif did not create a red plane".to_owned(),
                })?,
                image,
                |pixel| pixel.red,
                format,
            )?;
            copy_heif_channel(
                planes.g.as_mut().ok_or_else(|| CodecError::Output {
                    format,
                    message: "libheif did not create a green plane".to_owned(),
                })?,
                image,
                |pixel| pixel.green,
                format,
            )?;
            copy_heif_channel(
                planes.b.as_mut().ok_or_else(|| CodecError::Output {
                    format,
                    message: "libheif did not create a blue plane".to_owned(),
                })?,
                image,
                |pixel| pixel.blue,
                format,
            )?;
        }

        let mut context = HeifContext::new().map_err(|error| CodecError::Output {
            format,
            message: error.to_string(),
        })?;
        let mut encoder = libheif
            .encoder_for_format(CompressionFormat::Hevc)
            .map_err(|error| CodecError::Output {
                format,
                message: error.to_string(),
            })?;
        encoder
            .set_quality(EncoderQuality::Lossy(90))
            .map_err(|error| CodecError::Output {
                format,
                message: error.to_string(),
            })?;
        context
            .encode_image(&encoded, &mut encoder, None)
            .map_err(|error| CodecError::Output {
                format,
                message: error.to_string(),
            })?;
        let bytes = context
            .write_to_bytes()
            .map_err(|error| CodecError::Output {
                format,
                message: error.to_string(),
            })?;
        destination
            .write_all(&bytes)
            .map_err(|error| CodecError::Output {
                format,
                message: error.to_string(),
            })
    }
}

#[cfg(feature = "heic")]
fn canonical_from_heif(
    image: HeifImage,
    limits: DecodeLimits,
) -> std::result::Result<CanonicalImage, CodecError> {
    let width = image.width();
    let height = image.height();
    limits.check_dimensions(width, height)?;
    let plane = image
        .planes()
        .interleaved
        .ok_or_else(|| CodecError::Content {
            format: ImageFormat::Heic,
            message: "libheif did not produce an RGBA interleaved plane".to_owned(),
        })?;
    let row_bytes = usize::try_from(width)
        .ok()
        .and_then(|width| width.checked_mul(4))
        .ok_or(CodecError::ResourceLimit(
            ResourceLimitKind::IntermediateAllocation,
        ))?;
    let pixel_count = usize::try_from(u64::from(width) * u64::from(height))
        .map_err(|_| CodecError::ResourceLimit(ResourceLimitKind::IntermediateAllocation))?;
    let mut pixels = Vec::with_capacity(pixel_count);
    for row in 0..usize::try_from(height).expect("u32 fits in usize on supported targets") {
        let start = row
            .checked_mul(plane.stride)
            .ok_or(CodecError::ResourceLimit(
                ResourceLimitKind::IntermediateAllocation,
            ))?;
        let samples = plane
            .data
            .get(start..start.saturating_add(row_bytes))
            .ok_or_else(|| CodecError::Content {
                format: ImageFormat::Heic,
                message: "libheif returned a truncated RGBA plane".to_owned(),
            })?;
        for pixel in samples.chunks_exact(4) {
            pixels.push(Rgba16::new(
                u16::from(pixel[0]) * 257,
                u16::from(pixel[1]) * 257,
                u16::from(pixel[2]) * 257,
                u16::from(pixel[3]) * 257,
            ));
        }
    }
    CanonicalImage::new(width, height, pixels).map_err(|error| CodecError::Content {
        format: ImageFormat::Heic,
        message: format!("decoded HEIC violates canonical invariants: {error:?}"),
    })
}

#[cfg(feature = "heic")]
fn copy_heif_channel(
    plane: &mut libheif_rs::Plane<&mut [u8]>,
    image: &CanonicalImage,
    component: impl Fn(&Rgba16) -> u16,
    format: ImageFormat,
) -> std::result::Result<(), CodecError> {
    let width = usize::try_from(image.width()).expect("u32 fits in usize on supported targets");
    for (row, pixels) in image.pixels().chunks_exact(width).enumerate() {
        let start = row.checked_mul(plane.stride).ok_or(CodecError::Output {
            format,
            message: "libheif plane stride overflowed".to_owned(),
        })?;
        let destination = plane
            .data
            .get_mut(start..start.saturating_add(width))
            .ok_or_else(|| CodecError::Output {
                format,
                message: "libheif returned a truncated output plane".to_owned(),
            })?;
        for (output, pixel) in destination.iter_mut().zip(pixels) {
            *output = (component(pixel) >> 8) as u8;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use image_editor_core::{CapabilityName, NoticeSubject};
    use std::sync::atomic::{AtomicU64, Ordering};

    fn available_platform() -> StartupPlatformCapabilities {
        StartupPlatformCapabilities::new(
            PlatformCapability::available("native-folder-dialog"),
            PlatformCapability::available("native-save-dialog"),
        )
    }
    struct PassingPortable;
    impl PortableCodecSelfCheck for PassingPortable {
        fn check_decode(&self, _: ImageFormat, _: &[u8]) -> ProbeResult<()> {
            Ok(())
        }
        fn check_encode(&self, _: ImageFormat) -> ProbeResult<()> {
            Ok(())
        }
    }
    struct IndependentHeic;
    impl HeicRuntimeProbe for IndependentHeic {
        fn check_decode(&self) -> ProbeResult<()> {
            Ok(())
        }
        fn check_encode(&self) -> ProbeResult<()> {
            Err(CodecProbeError::new(
                "libheif HEIC encoder plugin is missing",
            ))
        }
    }

    #[test]
    fn registry_records_heic_decode_and_encode_independently() {
        let snapshot = CodecRegistry::detect_with(
            available_platform(),
            Some(&PassingPortable),
            &IndependentHeic,
        )
        .into_snapshot();
        assert!(snapshot.format(ImageFormat::Heic).can_decode());
        assert!(!snapshot.format(ImageFormat::Heic).can_encode());
        assert_eq!(
            snapshot.format(ImageFormat::Heic).provider,
            Some(CodecProvider::Libheif)
        );
        assert!(snapshot.diagnostics().iter().any(|notice| {
            notice.subject
                == NoticeSubject::Capability(CapabilityName::FormatEncode(ImageFormat::Heic))
                && notice
                    .message
                    .summary()
                    .contains("encoder plugin is missing")
        }));
    }

    #[cfg(feature = "portable-codecs")]
    fn sample_image() -> CanonicalImage {
        CanonicalImage::new(
            2,
            1,
            vec![
                Rgba16::new(257, 513, 1_027, u16::MAX),
                Rgba16::new(u16::MAX, 0, 32_768, 32_000),
            ],
        )
        .unwrap()
    }
    #[cfg(feature = "portable-codecs")]
    fn temp_path(extension: &str) -> AbsolutePath {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "image-editor-codec-{}-{}.{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed),
            extension
        ));
        AbsolutePath::new(path.to_string_lossy().into_owned()).unwrap()
    }

    #[cfg(feature = "portable-codecs")]
    #[test]
    fn portable_png_and_tiff_round_trip_exact_rgba16_samples() {
        let registry = CodecRegistry::detect(available_platform());
        let source = sample_image();
        for (format, extension) in [(ImageFormat::Png, "png"), (ImageFormat::Tiff, "tiff")] {
            let path = temp_path(extension);
            let mut bytes = Vec::new();
            registry.encode(&source, format, &mut bytes).unwrap();
            std::fs::write(path.as_str(), bytes).unwrap();
            let decoded = registry
                .decode(format, &path, DecodeLimits::DEFAULT)
                .unwrap();
            assert_eq!(decoded, source, "{format:?} must preserve RGBA16 samples");
            std::fs::remove_file(path.as_str()).unwrap();
        }
    }

    #[cfg(all(feature = "heic", feature = "portable-codecs"))]
    #[test]
    fn runtime_registered_heic_adapter_round_trips_when_both_plugins_are_available() {
        let registry = CodecRegistry::detect(available_platform());
        let capability = registry.snapshot().format(ImageFormat::Heic);
        if !(capability.can_decode() && capability.can_encode()) {
            return;
        }

        let source = sample_image();
        let path = temp_path("heic");
        let mut bytes = Vec::new();
        registry
            .encode(&source, ImageFormat::Heic, &mut bytes)
            .expect("runtime-advertised HEIC encoder must accept canonical images");
        assert!(!bytes.is_empty());
        std::fs::write(path.as_str(), bytes).unwrap();
        let decoded = registry
            .decode(ImageFormat::Heic, &path, DecodeLimits::DEFAULT)
            .expect("runtime-advertised HEIC decoder must reopen its encoded output");
        assert_eq!(
            (decoded.width(), decoded.height()),
            (source.width(), source.height())
        );
        std::fs::remove_file(path.as_str()).unwrap();
    }

    #[cfg(feature = "portable-codecs")]
    #[test]
    fn malformed_content_and_resource_limits_do_not_become_unavailable_capabilities() {
        let registry = CodecRegistry::detect(available_platform());
        let malformed = temp_path("png");
        std::fs::write(malformed.as_str(), b"not a png").unwrap();
        assert!(matches!(
            registry.decode(ImageFormat::Png, &malformed, DecodeLimits::DEFAULT),
            Err(CodecError::Content {
                format: ImageFormat::Png,
                ..
            })
        ));
        std::fs::remove_file(malformed.as_str()).unwrap();

        let path = temp_path("png");
        let mut bytes = Vec::new();
        registry
            .encode(&sample_image(), ImageFormat::Png, &mut bytes)
            .unwrap();
        std::fs::write(path.as_str(), bytes).unwrap();
        assert_eq!(
            registry.decode(
                ImageFormat::Png,
                &path,
                DecodeLimits {
                    max_total_pixels: 1,
                    ..DecodeLimits::DEFAULT
                }
            ),
            Err(CodecError::ResourceLimit(ResourceLimitKind::TotalPixels))
        );
        std::fs::remove_file(path.as_str()).unwrap();
    }

    #[cfg(feature = "portable-codecs")]
    #[test]
    fn jpeg_encoding_and_heic_dispatch_follow_detected_capability() {
        let registry = CodecRegistry::detect(available_platform());
        let mut jpeg = Vec::new();
        registry
            .encode(&sample_image(), ImageFormat::Jpeg, &mut jpeg)
            .unwrap();
        assert!(jpeg.starts_with(&[0xff, 0xd8]));

        let result = registry.decode(ImageFormat::Heic, &temp_path("heic"), DecodeLimits::DEFAULT);
        if registry.snapshot().format(ImageFormat::Heic).can_decode() {
            assert!(
                !matches!(result, Err(CodecError::Unavailable { .. })),
                "a runtime-registered HEIC decoder must attempt to decode content"
            );
        } else {
            assert!(matches!(
                result,
                Err(CodecError::Unavailable {
                    format: ImageFormat::Heic,
                    operation: CodecOperation::Decode,
                    ..
                })
            ));
        }
    }
}
