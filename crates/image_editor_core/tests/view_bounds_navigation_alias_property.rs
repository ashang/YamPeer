use std::collections::BTreeMap;

use image_editor_core::{
    EditorCommand, EffectiveKeybindingMap, KeyModifiers, KeybindingAction, KeybindingGesture,
    LogicalSize, NavigationDirection, PanDirection, RationalScale, RawKeyEvent, ShortcutKey,
    ShortcutResolver, ViewState, ZoomDirection,
};
use proptest::prelude::*;

fn scale_is_bounded(scale: RationalScale) -> bool {
    let numerator = u64::from(scale.numerator());
    let denominator = u64::from(scale.denominator());
    let minimum = RationalScale::MIN;
    let maximum = RationalScale::MAX;

    numerator * u64::from(minimum.denominator()) >= u64::from(minimum.numerator()) * denominator
        && numerator * u64::from(maximum.denominator())
            <= u64::from(maximum.numerator()) * denominator
}

fn offset_limit(image_extent: u32, preview_extent: u32, scale: RationalScale) -> i64 {
    let scaled_extent = (u64::from(image_extent) * u64::from(scale.numerator()))
        .div_ceil(u64::from(scale.denominator()));
    let overflow = scaled_extent.saturating_sub(u64::from(preview_extent));
    i64::try_from(overflow.div_ceil(2)).expect("generated logical extents fit in i64")
}

fn assert_view_is_bounded(view: &ViewState, image_size: LogicalSize) {
    let scale = view.effective_scale(image_size);
    assert!(scale_is_bounded(scale));
    assert!(scale_is_bounded(view.manual_scale));

    let maximum_x = offset_limit(image_size.width, view.preview_size.width, scale);
    let maximum_y = offset_limit(image_size.height, view.preview_size.height, scale);
    assert!((-maximum_x..=maximum_x).contains(&view.canvas_offset.x));
    assert!((-maximum_y..=maximum_y).contains(&view.canvas_offset.y));
}

fn pan_direction(value: u8) -> PanDirection {
    match value % 4 {
        0 => PanDirection::Left,
        1 => PanDirection::Down,
        2 => PanDirection::Up,
        _ => PanDirection::Right,
    }
}

fn configured_navigation_resolver(
    previous_alias: ShortcutKey,
    next_alias: ShortcutKey,
) -> ShortcutResolver {
    let plain = KeyModifiers::default();
    let mut bindings = BTreeMap::new();
    bindings.insert(
        KeybindingAction::PreviousImage,
        vec![KeybindingGesture::new(previous_alias, plain)],
    );
    bindings.insert(
        KeybindingAction::NextImage,
        vec![KeybindingGesture::new(next_alias, plain)],
    );

    ShortcutResolver::new(
        EffectiveKeybindingMap::try_from_bindings(bindings)
            .expect("the generated previous and next aliases do not overlap"),
    )
}

// Feature: macos-image-editor, Property 15: View transforms remain bounded and navigation aliases are semantically equivalent
// Validates: Requirements 12.6, 12.7, 12.8, 12.9
proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    #[test]
    fn view_transforms_stay_bounded_and_configured_navigation_aliases_match_navigation_intents(
        image_width in 1_u32..4097,
        image_height in 1_u32..4097,
        preview_width in 1_u32..4097,
        preview_height in 1_u32..4097,
        operations in prop::collection::vec((0_u8..6, any::<u16>(), 1_u32..4097, 1_u32..4097), 0..64),
        previous_alias_index in 0_usize..3,
        next_alias_index in 0_usize..4,
    ) {
        let image_size = LogicalSize::new(image_width, image_height);
        let mut view = ViewState::for_preview_size(LogicalSize::new(preview_width, preview_height));
        assert_view_is_bounded(&view, image_size);

        for (operation, value, resized_width, resized_height) in operations {
            view = match operation {
                0 => view.fit_to_window(image_size),
                1 => view.set_manual_zoom(value, image_size),
                2 => view.zoom_by_step(ZoomDirection::In, image_size),
                3 => view.zoom_by_step(ZoomDirection::Out, image_size),
                4 => view.pan(pan_direction(value as u8), image_size),
                _ => view.with_preview_size(LogicalSize::new(resized_width, resized_height), image_size),
            };
            assert_view_is_bounded(&view, image_size);
        }

        let previous_aliases = [ShortcutKey::ArrowLeft, ShortcutKey::ArrowUp, ShortcutKey::PageUp];
        let next_aliases = [
            ShortcutKey::ArrowRight,
            ShortcutKey::ArrowDown,
            ShortcutKey::PageDown,
            ShortcutKey::Space,
        ];
        let previous_alias = previous_aliases[previous_alias_index];
        let next_alias = next_aliases[next_alias_index];
        let resolver = configured_navigation_resolver(previous_alias, next_alias);
        let plain = KeyModifiers::default();

        prop_assert_eq!(
            resolver.resolve(RawKeyEvent::press(previous_alias, plain)),
            Some(EditorCommand::Navigate {
                direction: NavigationDirection::Left,
            }),
        );
        prop_assert_eq!(
            resolver.resolve(RawKeyEvent::press(next_alias, plain)),
            Some(EditorCommand::Navigate {
                direction: NavigationDirection::Right,
            }),
        );
    }
}
