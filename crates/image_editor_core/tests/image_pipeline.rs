use image_editor_core::{
    AbsolutePath, AdjustmentKind, CanonicalColorSpace, CanonicalImage, DecodedAlphaMode,
    DecodedImage, DraftAdjustments, EditOperation, ImageDocument, NormalizedOrientation,
    PreviewRequest, ReplayCache, Rgba16, SourceIdentity, SourceOrientation,
    normalize_decoded_image, render_current_editing_result,
};

fn image_id() -> SourceIdentity {
    SourceIdentity::new(AbsolutePath::new("/photos/source.png").unwrap(), None)
}

#[test]
fn decoded_images_become_top_left_straight_alpha_srgb_rgba16() {
    let decoded = DecodedImage::new(
        2,
        1,
        vec![
            Rgba16::new(32_768, 0, 16_384, 32_768),
            Rgba16::new(12_345, 54_321, 6_789, 0),
        ],
        DecodedAlphaMode::Premultiplied,
        SourceOrientation::TopRight,
    )
    .unwrap();

    let normalized = normalize_decoded_image(decoded).unwrap();

    assert_eq!((normalized.width(), normalized.height()), (2, 1));
    assert_eq!(normalized.color_space(), CanonicalColorSpace::Srgb);
    assert_eq!(normalized.orientation(), NormalizedOrientation::TopLeft);
    assert_eq!(
        normalized.pixels(),
        &[
            Rgba16::new(0, 0, 0, 0),
            Rgba16::new(u16::MAX, 0, 32_768, 32_768),
        ]
    );
}

#[test]
fn full_resolution_replay_applies_immutable_history_then_drafts() {
    let source = CanonicalImage::new(
        2,
        1,
        vec![
            Rgba16::new(0, 1, 2, 1_024),
            Rgba16::new(u16::MAX, 3, 4, u16::MAX),
        ],
    )
    .unwrap();
    let history = vec![EditOperation::FlipHorizontal];
    let mut draft = DraftAdjustments::new();
    draft.set(AdjustmentKind::Brightness, 1).unwrap();

    let result = render_current_editing_result(&source, &history, &draft).unwrap();

    assert_eq!((result.width(), result.height()), (2, 1));
    assert_eq!(
        result.pixels(),
        &[
            Rgba16::new(u16::MAX, 658, 659, u16::MAX),
            Rgba16::new(655, 656, 657, 1_024),
        ]
    );
    assert_eq!(
        source.pixels(),
        &[
            Rgba16::new(0, 1, 2, 1_024),
            Rgba16::new(u16::MAX, 3, 4, u16::MAX),
        ],
        "replay must never mutate the canonical base image"
    );
}

#[test]
fn revision_and_draft_keyed_cache_matches_uncached_replay() {
    let source = CanonicalImage::new(1, 1, vec![Rgba16::new(1_000, 2_000, 3_000, 4_000)]).unwrap();
    let document = ImageDocument::new(source.clone());
    let mut brighter = DraftAdjustments::new();
    brighter.set(AdjustmentKind::Brightness, 1).unwrap();
    let first = PreviewRequest {
        image_id: image_id(),
        revision: document.revision(),
        source: source.clone(),
        history: vec![EditOperation::FlipHorizontal],
        draft: brighter,
    };

    let mut changed_document = document.clone();
    changed_document.mark_changed();
    let mut darker = DraftAdjustments::new();
    darker.set(AdjustmentKind::Brightness, -1).unwrap();
    let second = PreviewRequest {
        image_id: image_id(),
        revision: changed_document.revision(),
        source,
        history: vec![EditOperation::FlipHorizontal],
        draft: darker,
    };

    let mut cache = ReplayCache::new();
    let first_cached = cache.evaluate_preview(&first).unwrap();
    assert_eq!(
        first_cached,
        render_current_editing_result(&first.source, &first.history, &first.draft).unwrap()
    );
    assert_eq!(cache.len(), 1);

    let second_cached = cache.evaluate_preview(&second).unwrap();
    assert_eq!(
        second_cached,
        render_current_editing_result(&second.source, &second.history, &second.draft).unwrap()
    );
    assert_ne!(first_cached, second_cached);
    assert_eq!(cache.len(), 2, "new revisions must not reuse stale results");

    assert_eq!(cache.evaluate_preview(&first).unwrap(), first_cached);
    assert_eq!(cache.len(), 2);
}

#[test]
fn fixed_point_adjustments_apply_brightness_then_contrast_clamp_and_preserve_alpha() {
    let source = CanonicalImage::new(
        3,
        1,
        vec![
            Rgba16::new(32_768, 32_769, 32_767, 101),
            Rgba16::new(0, 65_535, 32_768, 202),
            Rgba16::new(65_535, 0, 32_768, 303),
        ],
    )
    .unwrap();
    let mut draft = DraftAdjustments::new();
    draft.set(AdjustmentKind::Brightness, 1).unwrap();
    draft.set(AdjustmentKind::Contrast, 100).unwrap();

    let adjusted = render_current_editing_result(&source, &[], &draft).unwrap();

    assert_eq!(
        adjusted.pixels(),
        &[
            Rgba16::new(34_078, 34_080, 34_076, 101),
            Rgba16::new(0, 65_535, 34_078, 202),
            Rgba16::new(65_535, 0, 34_078, 303),
        ],
        "brightness is rounded before contrast, RGB is clamped, and alpha is unchanged"
    );
}
