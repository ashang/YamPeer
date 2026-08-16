use image_editor_core::{
    AbsolutePath, ApplicationError, ExportPlan, ExportTargetResolution, FileIdentity, ImageFormat,
    Revision, SourceIdentity, TargetConflict,
};
use proptest::prelude::*;

fn path(value: &str) -> AbsolutePath {
    AbsolutePath::new(value).expect("test paths are absolute")
}

fn source(path_value: &str, identity: Option<&str>) -> SourceIdentity {
    SourceIdentity::new(
        path(path_value),
        identity.map(|value| FileIdentity::new(value).expect("identity is valid")),
    )
}

#[test]
fn export_plan_rejects_the_source_path_even_without_platform_metadata() {
    let source = source("/photos/source.png", None);

    let error = ExportPlan::validate(
        source,
        Revision::INITIAL,
        path("/photos/source.png"),
        ImageFormat::Png,
        ExportTargetResolution::missing(),
    )
    .expect_err("the source path must never reach a writer");

    assert_eq!(
        error,
        ApplicationError::ExportTargetConflict {
            path: path("/photos/source.png"),
            kind: TargetConflict::SourceImage,
        }
    );
}

#[test]
fn export_plan_rejects_source_identity_aliases_and_other_existing_regular_files() {
    let source = source("/photos/source.png", Some("device:42"));
    let source_alias = ExportPlan::validate(
        source.clone(),
        Revision::INITIAL,
        path("/exports/source-alias.png"),
        ImageFormat::Png,
        ExportTargetResolution::existing_regular(Some(FileIdentity::new("device:42").unwrap())),
    )
    .expect_err("a hard-link or alias to the source must be rejected");
    assert_eq!(
        source_alias,
        ApplicationError::ExportTargetConflict {
            path: path("/exports/source-alias.png"),
            kind: TargetConflict::SourceImage,
        }
    );

    let existing_target = ExportPlan::validate(
        source,
        Revision::INITIAL,
        path("/exports/existing.png"),
        ImageFormat::Png,
        ExportTargetResolution::existing_regular(Some(FileIdentity::new("device:99").unwrap())),
    )
    .expect_err("an unrelated existing regular file must not be replaced");
    assert_eq!(
        existing_target,
        ApplicationError::ExportTargetConflict {
            path: path("/exports/existing.png"),
            kind: TargetConflict::ExistingLocalFile,
        }
    );
}

#[test]
fn valid_export_plan_retains_immutable_source_identity_and_document_revision() {
    let source = source("/photos/source.png", Some("device:42"));
    let plan = ExportPlan::validate(
        source.clone(),
        Revision::INITIAL,
        path("/exports/new.png"),
        ImageFormat::Png,
        ExportTargetResolution::missing(),
    )
    .expect("a missing target may be planned for exclusive creation");

    assert_eq!(plan.source_identity(), &source);
    assert_eq!(plan.document_revision(), Revision::INITIAL);
    assert_eq!(plan.target(), &path("/exports/new.png"));
    assert_eq!(plan.format(), ImageFormat::Png);
}

// Feature: macos-image-editor, Property 11: Export planning never permits replacement
// Validates: Requirements 7.2-7.6
proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    #[test]
    fn export_plan_rejects_every_replacement_conflict_and_preserves_valid_snapshots(
        case_id in any::<u32>(),
        source_has_identity in any::<bool>(),
        target_is_source_path in any::<bool>(),
        target_resolution_kind in 0u8..3,
        target_identity_kind in 0u8..3,
        format_index in 0usize..4,
    ) {
        let source_path = path(&format!("/photos/source-{case_id}.png"));
        let source = SourceIdentity::new(
            source_path.clone(),
            source_has_identity.then(|| {
                FileIdentity::new(format!("source-device-{case_id}"))
                    .expect("generated source identity is valid")
            }),
        );
        let target = if target_is_source_path {
            source_path
        } else {
            path(&format!("/exports/export-{case_id}.png"))
        };
        let target_identity = match target_identity_kind {
            0 => None,
            1 => source.file_identity().cloned().or_else(|| {
                Some(
                    FileIdentity::new(format!("target-device-{case_id}"))
                        .expect("generated target identity is valid"),
                )
            }),
            _ => Some(
                FileIdentity::new(format!("different-device-{case_id}"))
                    .expect("generated target identity is valid"),
            ),
        };
        let resolution = match target_resolution_kind {
            0 => ExportTargetResolution::missing(),
            1 => ExportTargetResolution::existing_regular(target_identity),
            _ => ExportTargetResolution::existing_other(),
        };
        let format = [
            ImageFormat::Jpeg,
            ImageFormat::Png,
            ImageFormat::Tiff,
            ImageFormat::Heic,
        ][format_index];
        let identifies_source_by_identity = matches!(
            &resolution,
            ExportTargetResolution::ExistingRegular {
                identity: Some(target_identity),
            } if source.file_identity() == Some(target_identity)
        );
        let expected_conflict = if target_is_source_path || identifies_source_by_identity {
            Some(TargetConflict::SourceImage)
        } else if matches!(resolution, ExportTargetResolution::ExistingRegular { .. }) {
            Some(TargetConflict::ExistingLocalFile)
        } else {
            None
        };

        let result = ExportPlan::validate(
            source.clone(),
            Revision::INITIAL,
            target.clone(),
            format,
            resolution,
        );

        match expected_conflict {
            Some(kind) => prop_assert_eq!(
                result,
                Err(ApplicationError::ExportTargetConflict { path: target, kind }),
            ),
            None => {
                let plan = result.expect("only missing or non-regular targets are plannable");
                prop_assert_eq!(plan.source_identity(), &source);
                prop_assert_eq!(plan.document_revision(), Revision::INITIAL);
                prop_assert_eq!(plan.target(), &target);
                prop_assert_eq!(plan.format(), format);
            }
        }
    }
}
