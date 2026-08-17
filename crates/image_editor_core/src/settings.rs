//! Versioned application-settings values and bounded JSON decoding.
//!
//! Filesystem adapters distinguish absence and read failure, retain at most
//! [`APP_SETTINGS_BOUNDED_READ_LIMIT`] bytes, and pass that bounded prefix to
//! this pure module. Diagnostics contain only stable categories and vetted
//! messages; untrusted settings content is never retained in an outcome.

use serde::{Deserialize, Serialize};

use crate::{AbsolutePath, PathValidationError};

/// Current on-disk schema version.
pub const APP_SETTINGS_VERSION: u16 = 1;
/// Maximum accepted settings payload: exactly 1 MiB.
pub const APP_SETTINGS_SIZE_LIMIT: usize = 1024 * 1024;
/// Maximum prefix needed to distinguish an accepted payload from overflow.
pub const APP_SETTINGS_BOUNDED_READ_LIMIT: usize = APP_SETTINGS_SIZE_LIMIT + 1;

/// A supported field used to order an image collection.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SortField {
    FullFileName,
    ModifiedTime,
    FileSize,
}

impl SortField {
    pub const fn stable_name(self) -> &'static str {
        match self {
            Self::FullFileName => "full_file_name",
            Self::ModifiedTime => "modified_time",
            Self::FileSize => "file_size",
        }
    }

    fn parse(value: &str) -> Result<Self, SettingsValidationError> {
        match value {
            "full_file_name" => Ok(Self::FullFileName),
            "modified_time" => Ok(Self::ModifiedTime),
            "file_size" => Ok(Self::FileSize),
            _ => Err(SettingsValidationError::InvalidSortField),
        }
    }
}

/// A supported direction used to order an image collection.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SortDirection {
    Ascending,
    Descending,
}

impl SortDirection {
    pub const fn stable_name(self) -> &'static str {
        match self {
            Self::Ascending => "ascending",
            Self::Descending => "descending",
        }
    }

    fn parse(value: &str) -> Result<Self, SettingsValidationError> {
        match value {
            "ascending" => Ok(Self::Ascending),
            "descending" => Ok(Self::Descending),
            _ => Err(SettingsValidationError::InvalidSortDirection),
        }
    }
}

/// A validated deterministic collection-order preference.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SortSettings {
    pub field: SortField,
    pub direction: SortDirection,
}

impl SortSettings {
    pub const fn new(field: SortField, direction: SortDirection) -> Self {
        Self { field, direction }
    }
}

impl Default for SortSettings {
    fn default() -> Self {
        Self::new(SortField::FullFileName, SortDirection::Ascending)
    }
}

/// The complete versioned settings value persisted between launches.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AppSettings {
    version: u16,
    sort: SortSettings,
    last_successful_source_folder: Option<AbsolutePath>,
}

impl AppSettings {
    pub const fn new(
        sort: SortSettings,
        last_successful_source_folder: Option<AbsolutePath>,
    ) -> Self {
        Self {
            version: APP_SETTINGS_VERSION,
            sort,
            last_successful_source_folder,
        }
    }

    pub const fn version(&self) -> u16 {
        self.version
    }

    pub const fn sort(&self) -> SortSettings {
        self.sort
    }

    pub fn last_successful_source_folder(&self) -> Option<&AbsolutePath> {
        self.last_successful_source_folder.as_ref()
    }
}

impl Default for AppSettings {
    fn default() -> Self {
        Self::new(SortSettings::default(), None)
    }
}

/// A bounded settings prefix captured by an adapter.
///
/// At most one byte beyond the accepted payload is retained. That sentinel is
/// sufficient to classify the input as oversized, regardless of its total size.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BoundedSettingsBytes {
    prefix: Vec<u8>,
}

impl BoundedSettingsBytes {
    /// Captures no more than the 1 MiB limit plus one sentinel byte.
    pub fn capture(bytes: &[u8]) -> Self {
        Self {
            prefix: bytes[..bytes.len().min(APP_SETTINGS_BOUNDED_READ_LIMIT)].to_vec(),
        }
    }

    pub fn is_oversized(&self) -> bool {
        self.prefix.len() > APP_SETTINGS_SIZE_LIMIT
    }

    pub fn retained_len(&self) -> usize {
        self.prefix.len()
    }

    fn accepted_bytes(&self) -> Option<&[u8]> {
        (!self.is_oversized()).then_some(self.prefix.as_slice())
    }
}

/// The adapter-observed state of the settings file before pure decoding.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SettingsInput {
    Absent,
    Unreadable,
    Present(BoundedSettingsBytes),
}

/// Semantic validation failures that can be safely reported without values from
/// the untrusted settings document.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SettingsValidationError {
    UnsupportedVersion,
    InvalidSortField,
    InvalidSortDirection,
    InvalidLastSuccessfulSourceFolder,
}

/// Stable categories rendered by the desktop host without raw settings data.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SettingsDiagnosticKind {
    Unreadable,
    Oversized,
    Malformed,
    UnsupportedVersion,
    InvalidSort,
    InvalidLastSuccessfulSourceFolder,
}

/// A vetted settings failure suitable for a non-blocking startup notice.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SettingsDiagnostic {
    kind: SettingsDiagnosticKind,
    validation_error: Option<SettingsValidationError>,
    safe_message: &'static str,
}

impl SettingsDiagnostic {
    pub const fn kind(&self) -> SettingsDiagnosticKind {
        self.kind
    }

    pub const fn validation_error(&self) -> Option<SettingsValidationError> {
        self.validation_error
    }

    pub const fn safe_message(&self) -> &'static str {
        self.safe_message
    }

    const fn unreadable() -> Self {
        Self {
            kind: SettingsDiagnosticKind::Unreadable,
            validation_error: None,
            safe_message: "settings could not be read",
        }
    }

    const fn oversized() -> Self {
        Self {
            kind: SettingsDiagnosticKind::Oversized,
            validation_error: None,
            safe_message: "settings exceeded the size limit",
        }
    }

    const fn malformed() -> Self {
        Self {
            kind: SettingsDiagnosticKind::Malformed,
            validation_error: None,
            safe_message: "settings are not valid JSON for the supported schema",
        }
    }

    const fn from_validation(error: SettingsValidationError) -> Self {
        let (kind, safe_message) = match error {
            SettingsValidationError::UnsupportedVersion => (
                SettingsDiagnosticKind::UnsupportedVersion,
                "settings use an unsupported version",
            ),
            SettingsValidationError::InvalidSortField
            | SettingsValidationError::InvalidSortDirection => (
                SettingsDiagnosticKind::InvalidSort,
                "settings contain an invalid sort preference",
            ),
            SettingsValidationError::InvalidLastSuccessfulSourceFolder => (
                SettingsDiagnosticKind::InvalidLastSuccessfulSourceFolder,
                "settings contain an invalid remembered folder",
            ),
        };
        Self {
            kind,
            validation_error: Some(error),
            safe_message,
        }
    }
}

/// Complete pure-core result of loading settings.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SettingsLoadOutcome {
    Absent,
    Valid(AppSettings),
    Invalid(SettingsDiagnostic),
}

impl SettingsLoadOutcome {
    /// Returns the session sort, selecting the required safe default on every
    /// absence or failure path.
    pub fn effective_sort(&self) -> SortSettings {
        match self {
            Self::Valid(settings) => settings.sort(),
            Self::Absent | Self::Invalid(_) => SortSettings::default(),
        }
    }

    pub fn settings(&self) -> Option<&AppSettings> {
        match self {
            Self::Valid(settings) => Some(settings),
            Self::Absent | Self::Invalid(_) => None,
        }
    }

    pub fn diagnostic(&self) -> Option<&SettingsDiagnostic> {
        match self {
            Self::Invalid(diagnostic) => Some(diagnostic),
            Self::Absent | Self::Valid(_) => None,
        }
    }
}

#[derive(Serialize)]
struct CanonicalAppSettings<'a> {
    version: u16,
    sort: CanonicalSortSettings,
    last_successful_source_folder: Option<&'a str>,
}

#[derive(Serialize)]
struct CanonicalSortSettings {
    field: &'static str,
    direction: &'static str,
}

/// Encodes a complete value as compact canonical JSON with stable field order
/// and stable ASCII enum names.
pub fn encode_app_settings(settings: &AppSettings) -> Vec<u8> {
    let canonical = CanonicalAppSettings {
        version: settings.version(),
        sort: CanonicalSortSettings {
            field: settings.sort().field.stable_name(),
            direction: settings.sort().direction.stable_name(),
        },
        last_successful_source_folder: settings
            .last_successful_source_folder()
            .map(AbsolutePath::as_str),
    };
    serde_json::to_vec(&canonical).expect("validated settings are always JSON serializable")
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawAppSettings {
    version: u16,
    sort: RawSortSettings,
    #[serde(default)]
    last_successful_source_folder: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawSortSettings {
    field: String,
    direction: String,
}

/// Decodes the adapter-observed settings state and applies all fallback rules.
pub fn decode_app_settings(input: SettingsInput) -> SettingsLoadOutcome {
    match input {
        SettingsInput::Absent => SettingsLoadOutcome::Absent,
        SettingsInput::Unreadable => SettingsLoadOutcome::Invalid(SettingsDiagnostic::unreadable()),
        SettingsInput::Present(bytes) => decode_present_settings(bytes),
    }
}

fn decode_present_settings(bytes: BoundedSettingsBytes) -> SettingsLoadOutcome {
    let Some(bytes) = bytes.accepted_bytes() else {
        return SettingsLoadOutcome::Invalid(SettingsDiagnostic::oversized());
    };

    let value: serde_json::Value = match serde_json::from_slice(bytes) {
        Ok(value) => value,
        Err(_) => return SettingsLoadOutcome::Invalid(SettingsDiagnostic::malformed()),
    };
    let Some(version) = value.get("version").and_then(serde_json::Value::as_u64) else {
        return SettingsLoadOutcome::Invalid(SettingsDiagnostic::malformed());
    };
    if version != u64::from(APP_SETTINGS_VERSION) {
        return SettingsLoadOutcome::Invalid(SettingsDiagnostic::from_validation(
            SettingsValidationError::UnsupportedVersion,
        ));
    }

    let raw: RawAppSettings = match serde_json::from_value(value) {
        Ok(raw) => raw,
        Err(_) => return SettingsLoadOutcome::Invalid(SettingsDiagnostic::malformed()),
    };
    debug_assert_eq!(raw.version, APP_SETTINGS_VERSION);

    match validate_raw_settings(raw) {
        Ok(settings) => SettingsLoadOutcome::Valid(settings),
        Err(error) => SettingsLoadOutcome::Invalid(SettingsDiagnostic::from_validation(error)),
    }
}

fn validate_raw_settings(raw: RawAppSettings) -> Result<AppSettings, SettingsValidationError> {
    let sort = SortSettings::new(
        SortField::parse(&raw.sort.field)?,
        SortDirection::parse(&raw.sort.direction)?,
    );
    let last_successful_source_folder = raw
        .last_successful_source_folder
        .map(AbsolutePath::new)
        .transpose()
        .map_err(|error| match error {
            PathValidationError::Empty
            | PathValidationError::ContainsNul
            | PathValidationError::NotAbsolute => {
                SettingsValidationError::InvalidLastSuccessfulSourceFolder
            }
        })?;
    Ok(AppSettings::new(sort, last_successful_source_folder))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decode(bytes: &[u8]) -> SettingsLoadOutcome {
        decode_app_settings(SettingsInput::Present(BoundedSettingsBytes::capture(bytes)))
    }

    #[test]
    fn canonical_json_round_trip_has_stable_names_and_field_order() {
        let settings = AppSettings::new(
            SortSettings::new(SortField::ModifiedTime, SortDirection::Descending),
            Some(AbsolutePath::new("/images/示例").unwrap()),
        );

        let encoded = encode_app_settings(&settings);
        assert_eq!(
            String::from_utf8(encoded.clone()).unwrap(),
            r#"{"version":1,"sort":{"field":"modified_time","direction":"descending"},"last_successful_source_folder":"/images/示例"}"#
        );
        assert_eq!(decode(&encoded), SettingsLoadOutcome::Valid(settings));
    }

    #[test]
    fn absent_has_no_diagnostic_and_every_failure_uses_default_sort() {
        let cases = [
            (decode_app_settings(SettingsInput::Absent), None),
            (
                decode_app_settings(SettingsInput::Unreadable),
                Some(SettingsDiagnosticKind::Unreadable),
            ),
            (
                decode(br#"{"version":1"#),
                Some(SettingsDiagnosticKind::Malformed),
            ),
            (
                decode(br#"{"version":2,"sort":{"field":"file_size","direction":"descending"}}"#),
                Some(SettingsDiagnosticKind::UnsupportedVersion),
            ),
            (
                decode(br#"{"version":1,"sort":{"field":"unknown","direction":"ascending"}}"#),
                Some(SettingsDiagnosticKind::InvalidSort),
            ),
        ];

        for (outcome, expected_kind) in cases {
            assert_eq!(outcome.effective_sort(), SortSettings::default());
            assert_eq!(
                outcome.diagnostic().map(SettingsDiagnostic::kind),
                expected_kind
            );
        }
    }

    #[test]
    fn bounded_capture_retains_exactly_one_overflow_sentinel() {
        let at_limit = BoundedSettingsBytes::capture(&vec![b' '; APP_SETTINGS_SIZE_LIMIT]);
        assert_eq!(at_limit.retained_len(), APP_SETTINGS_SIZE_LIMIT);
        assert!(!at_limit.is_oversized());

        let far_over_limit =
            BoundedSettingsBytes::capture(&vec![b'x'; APP_SETTINGS_SIZE_LIMIT + 4096]);
        assert_eq!(
            far_over_limit.retained_len(),
            APP_SETTINGS_BOUNDED_READ_LIMIT
        );
        assert!(far_over_limit.is_oversized());
        let outcome = decode_app_settings(SettingsInput::Present(far_over_limit));
        assert_eq!(
            outcome.diagnostic().map(SettingsDiagnostic::kind),
            Some(SettingsDiagnosticKind::Oversized)
        );
        assert_eq!(outcome.effective_sort(), SortSettings::default());
    }

    #[test]
    fn invalid_sort_direction_and_remembered_folder_have_safe_categories() {
        let invalid_direction = decode(
            br#"{"version":1,"sort":{"field":"file_size","direction":"sideways"},"secret":"must-not-appear"}"#,
        );
        let diagnostic = invalid_direction.diagnostic().unwrap();
        // The unknown top-level field makes the complete schema malformed before
        // semantic sort validation, and the diagnostic remains content-free.
        assert_eq!(diagnostic.kind(), SettingsDiagnosticKind::Malformed);
        assert!(!diagnostic.safe_message().contains("secret"));
        assert!(!diagnostic.safe_message().contains("sideways"));

        let invalid_folder = decode(
            br#"{"version":1,"sort":{"field":"file_size","direction":"ascending"},"last_successful_source_folder":"relative/private"}"#,
        );
        let diagnostic = invalid_folder.diagnostic().unwrap();
        assert_eq!(
            diagnostic.kind(),
            SettingsDiagnosticKind::InvalidLastSuccessfulSourceFolder
        );
        assert!(!diagnostic.safe_message().contains("relative/private"));
    }
}
