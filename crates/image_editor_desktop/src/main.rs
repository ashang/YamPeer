//! Native single-window entry point.

#[cfg(feature = "native-window")]
mod host;

#[cfg(feature = "native-window")]
fn main() {
    let current_directory = match std::env::current_dir() {
        Ok(path) => path,
        Err(error) => {
            eprintln!("Image Editor could not resolve its working directory: {error}");
            return;
        }
    };
    let explicit_keybindings =
        match image_editor_desktop::keybindings::parse_explicit_keybindings_argument(
            std::env::args_os().skip(1),
            &current_directory,
        ) {
            Ok(path) => path,
            Err(error) => {
                eprintln!("Image Editor could not parse its arguments: {error}");
                return;
            }
        };

    if let Err(error) = host::run(explicit_keybindings) {
        eprintln!("Image Editor could not start: {error}");
    }
}

#[cfg(not(feature = "native-window"))]
fn main() {
    // Packaging may intentionally omit the native host. This binary remains a
    // no-op rather than treating optional runtime adapters as a startup error.
}
