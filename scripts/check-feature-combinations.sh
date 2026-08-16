#!/usr/bin/env bash
# Compile every desktop feature subset so optional capability wiring cannot rot.
set -euo pipefail

cargo_command=${CARGO:-cargo}
features=(native-window portable-codecs heic macos-dialogs xdg-portal gtk)
combination_count=$((1 << ${#features[@]}))

for ((mask = 0; mask < combination_count; mask++)); do
    enabled=()
    for ((index = 0; index < ${#features[@]}; index++)); do
        if ((mask & (1 << index))); then
            enabled+=("${features[index]}")
        fi
    done

    if ((${#enabled[@]} == 0)); then
        echo 'Checking image_editor_desktop without optional features'
        "$cargo_command" check --locked --package image_editor_desktop --no-default-features
    else
        feature_list=$(IFS=,; echo "${enabled[*]}")
        echo "Checking image_editor_desktop features: ${feature_list}"
        "$cargo_command" check --locked --package image_editor_desktop --no-default-features --features "$feature_list"
    fi
done
