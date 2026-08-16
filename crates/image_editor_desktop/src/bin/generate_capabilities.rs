use std::{env, path::PathBuf, process::ExitCode};

use image_editor_desktop::PackageProfile;

fn main() -> ExitCode {
    match arguments().and_then(|(profile, output)| {
        profile
            .write_capabilities_json(&output)
            .map_err(|error| error.to_string())
    }) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("generate-capabilities: {error}");
            eprintln!("usage: generate-capabilities --profile <name> --output <path>");
            ExitCode::from(2)
        }
    }
}

fn arguments() -> Result<(PackageProfile, PathBuf), String> {
    let mut arguments = env::args().skip(1);
    let mut profile = None;
    let mut output = None;

    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "--profile" => {
                let value = arguments
                    .next()
                    .ok_or_else(|| "--profile requires a value".to_owned())?;
                profile = Some(PackageProfile::parse(&value).map_err(|error| error.to_string())?);
            }
            "--output" => {
                output = Some(PathBuf::from(
                    arguments
                        .next()
                        .ok_or_else(|| "--output requires a value".to_owned())?,
                ));
            }
            _ => return Err(format!("unrecognized argument: {argument}")),
        }
    }

    match (profile, output) {
        (Some(profile), Some(output)) => Ok((profile, output)),
        _ => Err("--profile and --output are both required".to_owned()),
    }
}
