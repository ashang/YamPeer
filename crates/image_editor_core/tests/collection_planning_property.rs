use std::collections::BTreeMap;

use image_editor_core::{
    AbsolutePath, Availability, AvailabilityReason, CapabilitySnapshot, CodecProvider,
    DirectoryEntry, DirectoryEntryKind, DirectoryEntryLocation, FolderEnumerationInput,
    FolderEnumerationPlan, FormatCapability, ImageFormat, PlatformCapability, Utf8FileName,
    plan_folder_enumeration,
};
use proptest::prelude::*;

type ReferenceEntry = (String, String, ImageFormat);
type ReferencePlan = (Vec<ReferenceEntry>, Vec<ReferenceEntry>);

#[derive(Clone, Debug)]
struct GeneratedDirectoryEntry {
    file_stem: String,
    extension: String,
    path_component: String,
    is_direct: bool,
    is_regular_file: bool,
}

impl GeneratedDirectoryEntry {
    fn file_name(&self) -> String {
        format!("{}.{}", self.file_stem, self.extension)
    }

    fn absolute_path(&self) -> String {
        let location = if self.is_direct {
            "direct"
        } else {
            "descendant"
        };
        format!(
            "/photos/{location}/{}/{}",
            self.path_component,
            self.file_name()
        )
    }

    fn format(&self) -> ImageFormat {
        ImageFormat::from_extension(&self.extension)
            .expect("the generator only creates supported image extensions")
    }

    fn to_directory_entry(&self) -> DirectoryEntry {
        DirectoryEntry::new(
            AbsolutePath::new(self.absolute_path()).expect("generated paths are absolute UTF-8"),
            Utf8FileName::new(self.file_name()).expect("generated filenames are valid UTF-8"),
            if self.is_direct {
                DirectoryEntryLocation::Direct
            } else {
                DirectoryEntryLocation::Descendant
            },
            if self.is_regular_file {
                DirectoryEntryKind::RegularFile
            } else {
                DirectoryEntryKind::Directory
            },
            None,
        )
    }
}

fn supported_extension() -> impl Strategy<Value = String> {
    prop_oneof![
        Just("jpg".to_owned()),
        Just("JPEG".to_owned()),
        Just("png".to_owned()),
        Just("PNG".to_owned()),
        Just("tif".to_owned()),
        Just("TIFF".to_owned()),
        Just("heic".to_owned()),
        Just("HEIC".to_owned()),
    ]
}

fn utf8_component() -> impl Strategy<Value = String> {
    prop_oneof![
        "[A-Za-z0-9_]{1,12}".prop_map(|value| value),
        Just("café".to_owned()),
        Just("画像".to_owned()),
    ]
}

fn generated_directory_entry() -> impl Strategy<Value = GeneratedDirectoryEntry> {
    (
        utf8_component(),
        supported_extension(),
        utf8_component(),
        any::<bool>(),
        any::<bool>(),
    )
        .prop_map(
            |(file_stem, extension, path_component, is_direct, is_regular_file)| {
                GeneratedDirectoryEntry {
                    file_stem,
                    extension,
                    path_component,
                    is_direct,
                    is_regular_file,
                }
            },
        )
}

fn availability(available: bool, format: ImageFormat) -> Availability {
    if available {
        Availability::Available
    } else {
        Availability::Unavailable {
            reason: AvailabilityReason::new(format!(
                "{} capability is unavailable for this test",
                format.display_name()
            )),
        }
    }
}

fn capabilities(capability_bits: [bool; 8]) -> CapabilitySnapshot {
    let mut formats = BTreeMap::new();
    for (index, format) in [
        ImageFormat::Jpeg,
        ImageFormat::Png,
        ImageFormat::Tiff,
        ImageFormat::Heic,
    ]
    .into_iter()
    .enumerate()
    {
        formats.insert(
            format,
            FormatCapability::new(
                availability(capability_bits[index], format),
                availability(capability_bits[index + 4], format),
                Some(if format == ImageFormat::Heic {
                    CodecProvider::Libheif
                } else {
                    CodecProvider::PortableRust
                }),
            ),
        );
    }

    CapabilitySnapshot::new(
        formats,
        PlatformCapability::available("test-folder-picker"),
        PlatformCapability::available("test-save-picker"),
    )
}

fn reference_plan(
    entries: &[GeneratedDirectoryEntry],
    decode_capabilities: [bool; 4],
) -> ReferencePlan {
    let can_decode = |format| match format {
        ImageFormat::Jpeg => decode_capabilities[0],
        ImageFormat::Png => decode_capabilities[1],
        ImageFormat::Tiff => decode_capabilities[2],
        ImageFormat::Heic => decode_capabilities[3],
    };

    let mut supported = Vec::new();
    let mut unavailable = Vec::new();
    for entry in entries {
        if !entry.is_direct || !entry.is_regular_file {
            continue;
        }

        let item = (entry.file_name(), entry.absolute_path(), entry.format());
        if can_decode(item.2) {
            supported.push(item);
        } else {
            unavailable.push(item);
        }
    }

    supported.sort_by(|left, right| {
        left.0
            .as_bytes()
            .cmp(right.0.as_bytes())
            .then_with(|| left.1.as_bytes().cmp(right.1.as_bytes()))
    });
    (supported, unavailable)
}

// Feature: macos-image-editor, Property 1: Capability-filtered collection is complete and ordered
proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    #[test]
    fn capability_filtered_collection_matches_reference_ordering_oracle(
        entries in prop::collection::vec(generated_directory_entry(), 0..64),
        capability_bits in prop::array::uniform8(any::<bool>()),
    ) {
        let capabilities = capabilities(capability_bits);
        let expected = reference_plan(&entries, [
            capability_bits[0],
            capability_bits[1],
            capability_bits[2],
            capability_bits[3],
        ]);
        let input_entries = entries
            .iter()
            .map(GeneratedDirectoryEntry::to_directory_entry)
            .collect();
        let result = plan_folder_enumeration(
            &capabilities,
            FolderEnumerationInput::Succeeded {
                folder: AbsolutePath::new("/photos").expect("test folder is absolute UTF-8"),
                entries: input_entries,
            },
        );

        let FolderEnumerationPlan::Ready(plan) = result else {
            prop_assert!(false, "successful enumeration must produce a collection plan");
            unreachable!();
        };
        let actual_supported = plan
            .collection()
            .entries()
            .iter()
            .map(|entry| (
                entry.file_name.as_str().to_owned(),
                entry.absolute_path.as_str().to_owned(),
                entry.format,
            ))
            .collect::<Vec<_>>();
        let actual_unavailable = plan
            .unavailable()
            .iter()
            .map(|entry| (
                entry.file_name().as_str().to_owned(),
                entry.absolute_path().as_str().to_owned(),
                entry.format(),
            ))
            .collect::<Vec<_>>();

        prop_assert_eq!(actual_supported, expected.0);
        prop_assert_eq!(actual_unavailable, expected.1);
    }
}
