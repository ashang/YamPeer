//! Platform integration boundary.
//!
//! Linux dialog backends are opt-in Cargo features. Their presence here does
//! not assert that a portal service or GTK runtime will be usable at startup;
//! later capability probes own that decision.

pub use image_editor_core::{ApplicationError, ErrorCategory, Result, SafeError};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CompiledPlatformFeatures {
    pub xdg_portal_backend: bool,
    pub gtk_backend: bool,
}

/// Compile-time linkage facts only; adapters must still probe runtime support.
pub const COMPILED_FEATURES: CompiledPlatformFeatures = CompiledPlatformFeatures {
    xdg_portal_backend: cfg!(feature = "xdg-portal"),
    gtk_backend: cfg!(feature = "gtk"),
};
