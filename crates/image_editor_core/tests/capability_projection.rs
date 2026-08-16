use std::collections::BTreeMap;

use image_editor_core::{
    AbsolutePath, Availability, AvailabilityReason, CapabilityName, CapabilitySnapshot,
    CodecProvider, DependentOperation, DirectoryEntry, DirectoryEntryKind, DirectoryEntryLocation,
    FolderEnumerationInput, FolderEnumerationPlan, FormatCapability, ImageFormat, NoticeSubject,
    PlatformCapability, Utf8FileName, plan_folder_enumeration, project_capabilities,
};
use proptest::prelude::*;

fn path(value: &str) -> AbsolutePath {
    AbsolutePath::new(value).unwrap()
}

fn unavailable(reason: &str) -> Availability {
    Availability::Unavailable {
        reason: AvailabilityReason::new(reason),
    }
}

fn snapshot(
    jpeg: (Availability, Availability),
    png: (Availability, Availability),
    tiff: (Availability, Availability),
    heic: (Availability, Availability),
    folder_picker: PlatformCapability,
    save_picker: PlatformCapability,
) -> CapabilitySnapshot {
    let mut formats = BTreeMap::new();
    for (format, (decode, encode), provider) in [
        (ImageFormat::Jpeg, jpeg, CodecProvider::PortableRust),
        (ImageFormat::Png, png, CodecProvider::PortableRust),
        (ImageFormat::Tiff, tiff, CodecProvider::PortableRust),
        (ImageFormat::Heic, heic, CodecProvider::Libheif),
    ] {
        formats.insert(
            format,
            FormatCapability::new(decode, encode, Some(provider)),
        );
    }
    CapabilitySnapshot::new(formats, folder_picker, save_picker)
}

fn direct_file(path_name: &str, name: &str) -> DirectoryEntry {
    DirectoryEntry::new(
        path(path_name),
        Utf8FileName::new(name).unwrap(),
        DirectoryEntryLocation::Direct,
        DirectoryEntryKind::RegularFile,
        None,
    )
}

const FORMATS: [ImageFormat; 4] = [
    ImageFormat::Jpeg,
    ImageFormat::Png,
    ImageFormat::Tiff,
    ImageFormat::Heic,
];

fn availability_from_truth_table(available: bool, capability: &str) -> Availability {
    if available {
        Availability::Available
    } else {
        unavailable(&format!("{capability} is unavailable"))
    }
}

fn snapshot_from_truth_table(truth_table: [bool; 10]) -> CapabilitySnapshot {
    snapshot(
        (
            availability_from_truth_table(truth_table[0], "JPEG decoding"),
            availability_from_truth_table(truth_table[4], "JPEG encoding"),
        ),
        (
            availability_from_truth_table(truth_table[1], "PNG decoding"),
            availability_from_truth_table(truth_table[5], "PNG encoding"),
        ),
        (
            availability_from_truth_table(truth_table[2], "TIFF decoding"),
            availability_from_truth_table(truth_table[6], "TIFF encoding"),
        ),
        (
            availability_from_truth_table(truth_table[3], "HEIC decoding"),
            availability_from_truth_table(truth_table[7], "HEIC encoding"),
        ),
        if truth_table[8] {
            PlatformCapability::available("test-folder-picker")
        } else {
            PlatformCapability::unavailable("folder picker is unavailable")
        },
        if truth_table[9] {
            PlatformCapability::available("test-save-picker")
        } else {
            PlatformCapability::unavailable("save picker is unavailable")
        },
    )
}

fn format_file_name(format: ImageFormat) -> &'static str {
    match format {
        ImageFormat::Jpeg => "unavailable.jpeg",
        ImageFormat::Png => "unavailable.png",
        ImageFormat::Tiff => "unavailable.tiff",
        ImageFormat::Heic => "unavailable.heic",
    }
}

fn folder_plan_for(capabilities: &CapabilitySnapshot) -> image_editor_core::FolderCollectionPlan {
    let entries = FORMATS
        .into_iter()
        .map(|format| {
            let name = format_file_name(format);
            direct_file(&format!("/photos/{name}"), name)
        })
        .collect();
    let FolderEnumerationPlan::Ready(plan) = plan_folder_enumeration(
        capabilities,
        FolderEnumerationInput::Succeeded {
            folder: path("/photos"),
            entries,
        },
    ) else {
        panic!("successful enumeration should produce a folder plan");
    };
    plan
}

#[test]
fn projection_keeps_portable_browsing_and_export_enabled_when_heic_is_unavailable() {
    let capabilities = snapshot(
        (Availability::Available, Availability::Available),
        (Availability::Available, Availability::Available),
        (Availability::Available, Availability::Available),
        (
            unavailable("HEIC decoder is missing"),
            unavailable("HEIC encoder is missing"),
        ),
        PlatformCapability::available("native-folder"),
        PlatformCapability::available("native-save"),
    );
    let FolderEnumerationPlan::Ready(folder_plan) = plan_folder_enumeration(
        &capabilities,
        FolderEnumerationInput::Succeeded {
            folder: path("/photos"),
            entries: vec![
                direct_file("/photos/portable.png", "portable.png"),
                direct_file("/photos/unavailable.heic", "unavailable.heic"),
            ],
        },
    ) else {
        panic!("successful enumeration should produce a folder plan");
    };

    let projection = project_capabilities(&capabilities, Some(&folder_plan));

    assert_eq!(
        projection
            .selectable_images()
            .iter()
            .map(|entry| entry.file_name.as_str())
            .collect::<Vec<_>>(),
        vec!["portable.png"]
    );
    assert_eq!(
        projection.export_formats(),
        &[ImageFormat::Jpeg, ImageFormat::Png, ImageFormat::Tiff]
    );
    assert!(projection.is_operation_enabled(DependentOperation::OpenFolder));
    assert!(projection.is_operation_enabled(DependentOperation::Export));
    assert!(projection.disabled_operations().is_empty());
    assert!(projection.availability_messages().iter().any(|message| {
        message.subject
            == image_editor_core::NoticeSubject::FileName(
                Utf8FileName::new("unavailable.heic").unwrap(),
            )
            && message
                .message
                .summary()
                .contains("HEIC decoding is unavailable")
    }));
}

#[test]
fn projection_keeps_decode_and_encode_capabilities_independent() {
    let capabilities = snapshot(
        (
            unavailable("JPEG decoder is missing"),
            Availability::Available,
        ),
        (
            Availability::Available,
            unavailable("PNG encoder is missing"),
        ),
        (Availability::Available, Availability::Available),
        (
            Availability::Available,
            unavailable("HEIC encoder is missing"),
        ),
        PlatformCapability::available("native-folder"),
        PlatformCapability::available("native-save"),
    );
    let FolderEnumerationPlan::Ready(folder_plan) = plan_folder_enumeration(
        &capabilities,
        FolderEnumerationInput::Succeeded {
            folder: path("/photos"),
            entries: vec![
                direct_file("/photos/unavailable.jpg", "unavailable.jpg"),
                direct_file("/photos/decodable.heic", "decodable.heic"),
            ],
        },
    ) else {
        panic!("successful enumeration should produce a folder plan");
    };

    let projection = project_capabilities(&capabilities, Some(&folder_plan));

    assert_eq!(
        projection
            .selectable_images()
            .iter()
            .map(|entry| entry.format)
            .collect::<Vec<_>>(),
        vec![ImageFormat::Heic]
    );
    assert_eq!(
        projection.export_formats(),
        &[ImageFormat::Jpeg, ImageFormat::Tiff]
    );
    assert!(projection.is_operation_enabled(DependentOperation::Export));
}

#[test]
fn projection_disables_only_operations_that_require_unavailable_dialogs() {
    let capabilities = snapshot(
        (Availability::Available, Availability::Available),
        (Availability::Available, Availability::Available),
        (Availability::Available, Availability::Available),
        (
            unavailable("HEIC decoder is missing"),
            unavailable("HEIC encoder is missing"),
        ),
        PlatformCapability::unavailable("folder dialog backend is missing"),
        PlatformCapability::unavailable("save dialog backend is missing"),
    );

    let projection = project_capabilities(&capabilities, None);

    assert!(!projection.is_operation_enabled(DependentOperation::OpenFolder));
    assert!(!projection.is_operation_enabled(DependentOperation::Export));
    assert_eq!(
        projection.disabled_operations(),
        &[
            image_editor_core::DisabledOperation {
                operation: DependentOperation::OpenFolder,
                unavailable_capabilities: vec![CapabilityName::FolderPicker],
            },
            image_editor_core::DisabledOperation {
                operation: DependentOperation::Export,
                unavailable_capabilities: vec![CapabilityName::SavePicker],
            },
        ]
    );
    assert_eq!(
        projection.export_formats(),
        &[ImageFormat::Jpeg, ImageFormat::Png, ImageFormat::Tiff]
    );
    assert!(projection.availability_messages().iter().any(|message| {
        message.subject
            == image_editor_core::NoticeSubject::Capability(CapabilityName::FolderPicker)
    }));
    assert!(projection.availability_messages().iter().any(|message| {
        message.subject == image_editor_core::NoticeSubject::Capability(CapabilityName::SavePicker)
    }));
}

// Feature: macos-image-editor, Property 9: Capability projection is conservative and format-specific
// Validates: Requirements 7.1, 8.1, 9.1-9.8, 11.2, 11.3
proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    #[test]
    fn capability_projection_matches_the_independent_requirement_matrix(
        truth_table in prop::array::uniform10(any::<bool>()),
    ) {
        let capabilities = snapshot_from_truth_table(truth_table);
        let fully_available = snapshot_from_truth_table([true; 10]);
        let stale_folder_plan = folder_plan_for(&fully_available);
        let current_folder_plan = folder_plan_for(&capabilities);

        // A plan produced before a capability downgrade must be re-filtered by
        // the current snapshot, so no unavailable decoder leaves an image selectable.
        let stale_projection = project_capabilities(&capabilities, Some(&stale_folder_plan));
        let mut expected_decodable_formats = FORMATS
            .into_iter()
            .enumerate()
            .filter_map(|(index, format)| truth_table[index].then_some(format))
            .collect::<Vec<_>>();
        expected_decodable_formats.sort_by_key(|format| format_file_name(*format));
        let stale_selectable_formats = stale_projection
            .selectable_images()
            .iter()
            .map(|entry| entry.format)
            .collect::<Vec<_>>();
        prop_assert_eq!(
            stale_selectable_formats,
            expected_decodable_formats.clone(),
        );

        let projection = project_capabilities(&capabilities, Some(&current_folder_plan));
        let selectable_formats = projection
            .selectable_images()
            .iter()
            .map(|entry| entry.format)
            .collect::<Vec<_>>();
        prop_assert_eq!(selectable_formats, expected_decodable_formats);

        let expected_export_formats = FORMATS
            .into_iter()
            .enumerate()
            .filter_map(|(index, format)| truth_table[index + 4].then_some(format))
            .collect::<Vec<_>>();
        prop_assert_eq!(projection.export_formats(), expected_export_formats.as_slice());

        prop_assert_eq!(
            projection.is_operation_enabled(DependentOperation::OpenFolder),
            truth_table[8],
        );
        prop_assert_eq!(
            projection.is_operation_enabled(DependentOperation::Export),
            truth_table[9] && !expected_export_formats.is_empty(),
        );

        let mut expected_disabled = Vec::new();
        if !truth_table[8] {
            expected_disabled.push(image_editor_core::DisabledOperation {
                operation: DependentOperation::OpenFolder,
                unavailable_capabilities: vec![CapabilityName::FolderPicker],
            });
        }
        if !truth_table[9] || expected_export_formats.is_empty() {
            let mut unavailable_capabilities = Vec::new();
            if !truth_table[9] {
                unavailable_capabilities.push(CapabilityName::SavePicker);
            }
            if expected_export_formats.is_empty() {
                unavailable_capabilities.extend(
                    FORMATS
                        .into_iter()
                        .map(CapabilityName::FormatEncode),
                );
            }
            expected_disabled.push(image_editor_core::DisabledOperation {
                operation: DependentOperation::Export,
                unavailable_capabilities,
            });
        }
        prop_assert_eq!(projection.disabled_operations(), expected_disabled.as_slice());

        let mut expected_notice_subjects = Vec::new();
        for (index, format) in FORMATS.into_iter().enumerate() {
            if !truth_table[index] {
                expected_notice_subjects.push(NoticeSubject::Capability(
                    CapabilityName::FormatDecode(format),
                ));
            }
            if !truth_table[index + 4] {
                expected_notice_subjects.push(NoticeSubject::Capability(
                    CapabilityName::FormatEncode(format),
                ));
            }
        }
        if !truth_table[8] {
            expected_notice_subjects.push(NoticeSubject::Capability(CapabilityName::FolderPicker));
        }
        if !truth_table[9] {
            expected_notice_subjects.push(NoticeSubject::Capability(CapabilityName::SavePicker));
        }
        for (index, format) in FORMATS.into_iter().enumerate() {
            if !truth_table[index] {
                expected_notice_subjects.push(NoticeSubject::FileName(
                    Utf8FileName::new(format_file_name(format)).expect("generated filename is UTF-8"),
                ));
            }
        }
        let actual_notice_subjects = projection
            .availability_messages()
            .iter()
            .map(|notice| notice.subject.clone())
            .collect::<Vec<_>>();
        prop_assert_eq!(actual_notice_subjects, expected_notice_subjects);
    }
}
