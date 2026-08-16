use image_editor_core::{CanonicalImage, EditOperation, Rgba16, apply_edit_operation};
use proptest::prelude::*;

fn asymmetric_image(width: u32, height: u32) -> CanonicalImage {
    let pixels = (0..width * height)
        .map(|index| {
            let sample = index as u16;
            Rgba16::new(sample, !sample, sample.wrapping_mul(257), u16::MAX - sample)
        })
        .collect();
    CanonicalImage::new(width, height, pixels)
        .expect("generated asymmetric dimensions and pixels form a canonical image")
}

fn pixel_at(image: &CanonicalImage, x: u32, y: u32) -> Rgba16 {
    image.pixels()[(y * image.width() + x) as usize]
}

// Feature: macos-image-editor, Property 4: Geometric operations preserve their specified pixel mapping
// Validates: Requirements 3.1-3.5
proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    #[test]
    fn geometric_operations_preserve_specified_pixel_mappings(
        (width, height) in (1u32..=16, 1u32..=16).prop_filter(
            "images must have asymmetric dimensions",
            |(width, height)| width != height,
        ),
    ) {
        let source = asymmetric_image(width, height);
        let horizontal = apply_edit_operation(&source, &EditOperation::FlipHorizontal)
            .expect("horizontal flip accepts a canonical image");
        let vertical = apply_edit_operation(&source, &EditOperation::FlipVertical)
            .expect("vertical flip accepts a canonical image");
        let clockwise = apply_edit_operation(&source, &EditOperation::RotateClockwise90)
            .expect("clockwise rotation accepts a canonical image");
        let counterclockwise = apply_edit_operation(&source, &EditOperation::RotateCounterclockwise90)
            .expect("counterclockwise rotation accepts a canonical image");

        prop_assert_eq!((horizontal.width(), horizontal.height()), (width, height));
        prop_assert_eq!((vertical.width(), vertical.height()), (width, height));
        prop_assert_eq!((clockwise.width(), clockwise.height()), (height, width));
        prop_assert_eq!((counterclockwise.width(), counterclockwise.height()), (height, width));

        for y in 0..height {
            for x in 0..width {
                let source_pixel = pixel_at(&source, x, y);
                prop_assert_eq!(pixel_at(&horizontal, width - 1 - x, y), source_pixel);
                prop_assert_eq!(pixel_at(&vertical, x, height - 1 - y), source_pixel);
                prop_assert_eq!(pixel_at(&clockwise, height - 1 - y, x), source_pixel);
                prop_assert_eq!(pixel_at(&counterclockwise, y, width - 1 - x), source_pixel);
            }
        }

        let after_four_clockwise_rotations = (0..4).fold(source.clone(), |image, _| {
            apply_edit_operation(&image, &EditOperation::RotateClockwise90)
                .expect("clockwise rotation accepts each intermediate canonical image")
        });
        prop_assert_eq!(after_four_clockwise_rotations, source);
    }
}
