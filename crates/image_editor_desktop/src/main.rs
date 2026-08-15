//! Future `eframe` host boundary.
//!
//! Feature selection only chooses which adapters are linked. Runtime codec and
//! dialog availability must be probed before the UI enables dependent actions.

use image_editor_codecs::COMPILED_FEATURES as CODEC_FEATURES;
use image_editor_platform::COMPILED_FEATURES as PLATFORM_FEATURES;

fn main() {
    let _compiled_adapters = (CODEC_FEATURES, PLATFORM_FEATURES);
    // The single-window eframe startup is implemented by task 8.3. This
    // boundary intentionally performs no optional codec or dialog startup.
}
