use image_editor_core::{
    AbsolutePath, ApplicationError, ExportPlan, ExportTargetResolution, FileIdentity, ImageFormat,
    Revision, SourceIdentity, TargetConflict,
};

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
