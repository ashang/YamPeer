//! Native single-window entry point.

#[cfg(feature = "native-window")]
mod host;

#[cfg(feature = "native-window")]
fn main() {
    if let Err(error) = host::run() {
        eprintln!("Image Editor could not start: {error}");
    }
}

#[cfg(not(feature = "native-window"))]
fn main() {
    // Packaging may intentionally omit the native host. This binary remains a
    // no-op rather than treating optional runtime adapters as a startup error.
}
