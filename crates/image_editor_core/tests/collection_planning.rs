use std::collections::BTreeMap;

use image_editor_core::{
    AbsolutePath, Availability, AvailabilityReason, CapabilitySnapshot, CodecProvider,
    DirectoryEntry, DirectoryEntryKind, DirectoryEntryLocation, FolderEnumerationInput,
    FolderEnumerationPlan, FormatCapability, ImageFormat, NoticeSeverity, NoticeSubject,
    PlatformCapability, SafeError, Utf8FileName, plan_folder_enumeration,
};

fn path(value: &str) -> AbsolutePath {
    AbsolutePath::new(value).unwrap()
}

fn entry(
    absolute_path: &str,
    file_name: &str,
    location: DirectoryEntryLocation,
    kind: DirectoryEntryKind,
) -> DirectoryEntry {
    DirectoryEntry::new(
        path(absolute_path),
        Utf8FileName::new(file_name).unwrap(),
        location,
        kind,
        None,
    )
}

fn capabilities(heic_decode: Availability) -> CapabilitySnapshot {
    let available = || {
        FormatCapability::new(
            Availability::Available,
            Availability::Available,
            Some(CodecProvider::PortableRust),
        )
    };
    let mut formats = BTreeMap::new();
    formats.insert(ImageFormat::Jpeg, available());
    formats.insert(ImageFormat::Png, available());
    formats.insert(ImageFormat::Tiff, available());
    formats.insert(
        ImageFormat::Heic,
        FormatCapability::new(
            heic_decode,
            Availability::Available,
            Some(CodecProvider::Libheif),
        ),
    );
    CapabilitySnapshot::new(
        formats,
        PlatformCapability::available("folder-picker"),
        PlatformCapability::available("save-picker"),
    )
}

#[test]
fn planner_filters_to_direct_regular_candidates_and_orders_supported_entries_by_utf8_bytes() {
    let input = FolderEnumerationInput::Succeeded {
        folder: path("/photos"),
        entries: vec![
            entry(
                "/photos/child/inside.png",
                "inside.png",
                DirectoryEntryLocation::Descendant,
                DirectoryEntryKind::RegularFile,
            ),
            entry(
                "/photos/subdir.png",
                "subdir.png",
                DirectoryEntryLocation::Direct,
                DirectoryEntryKind::Directory,
            ),
            entry(
                "/photos/readme.txt",
                "readme.txt",
                DirectoryEntryLocation::Direct,
                DirectoryEntryKind::RegularFile,
            ),
            entry(
                "/photos/z.jpeg",
                "z.jpeg",
                DirectoryEntryLocation::Direct,
                DirectoryEntryKind::RegularFile,
            ),
            entry(
                "/photos/Alpha.PNG",
                "Alpha.PNG",
                DirectoryEntryLocation::Direct,
                DirectoryEntryKind::RegularFile,
            ),
            entry(
                "/photos/é.tif",
                "é.tif",
                DirectoryEntryLocation::Direct,
                DirectoryEntryKind::RegularFile,
            ),
        ],
    };

    let FolderEnumerationPlan::Ready(plan) =
        plan_folder_enumeration(&capabilities(Availability::Available), input)
    else {
        panic!("successful enumeration should produce a collection plan");
    };

    assert_eq!(plan.source_folder(), &path("/photos"));
    assert_eq!(
        plan.collection()
            .entries()
            .iter()
            .map(|entry| entry.file_name.as_str())
            .collect::<Vec<_>>(),
        vec!["Alpha.PNG", "z.jpeg", "é.tif"],
    );
    assert!(plan.unavailable().is_empty());
}

#[test]
fn planner_retains_undecodable_direct_candidates_as_filename_availability_notices() {
    let heic_unavailable = Availability::Unavailable {
        reason: AvailabilityReason::new("libheif decoder plugin is missing"),
    };
    let input = FolderEnumerationInput::Succeeded {
        folder: path("/photos"),
        entries: vec![
            entry(
                "/photos/available.png",
                "available.png",
                DirectoryEntryLocation::Direct,
                DirectoryEntryKind::RegularFile,
            ),
            entry(
                "/photos/unavailable.heic",
                "unavailable.heic",
                DirectoryEntryLocation::Direct,
                DirectoryEntryKind::RegularFile,
            ),
        ],
    };

    let FolderEnumerationPlan::Ready(plan) =
        plan_folder_enumeration(&capabilities(heic_unavailable), input)
    else {
        panic!("successful enumeration should produce a collection plan");
    };

    assert_eq!(plan.collection().entries().len(), 1);
    assert_eq!(plan.unavailable().len(), 1);
    assert_eq!(
        plan.unavailable()[0].file_name().as_str(),
        "unavailable.heic"
    );
    assert_eq!(plan.unavailable()[0].format(), ImageFormat::Heic);

    let notices = plan.availability_notices();
    assert_eq!(notices.len(), 1);
    assert_eq!(notices[0].severity, NoticeSeverity::Availability);
    assert_eq!(
        notices[0].subject,
        NoticeSubject::FileName(Utf8FileName::new("unavailable.heic").unwrap())
    );
    assert!(
        notices[0]
            .message
            .summary()
            .contains("HEIC decoding is unavailable")
    );
}

#[test]
fn planner_exposes_enumeration_failure_without_a_collection_to_install() {
    let folder = path("/unreadable");
    let cause = SafeError::new(
        image_editor_core::ErrorCategory::FileSystem,
        "permission denied",
    );
    let outcome = plan_folder_enumeration(
        &capabilities(Availability::Available),
        FolderEnumerationInput::Failed {
            folder: folder.clone(),
            cause: cause.clone(),
        },
    );

    assert_eq!(
        outcome,
        FolderEnumerationPlan::Failed(image_editor_core::ApplicationError::FolderEnumeration {
            folder,
            cause,
        })
    );
}
