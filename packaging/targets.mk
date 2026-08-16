CARGO ?= cargo
DIST_DIR ?= dist

# Release packages opt in to the exact host, portable codec, and platform-dialog
# surface they ship instead of relying on the desktop crate's default features.
COMMON_PACKAGE_FEATURES := native-window,portable-codecs
MACOS_PACKAGE_FEATURES := $(COMMON_PACKAGE_FEATURES),macos-dialogs
LINUX_PACKAGE_FEATURES := $(COMMON_PACKAGE_FEATURES),xdg-portal

.PHONY: package-macos package-linux

# Produces a macOS application layout whose executable-relative font path is
# `Contents/Resources/resources/fonts/...`, plus its metadata inventories.
package-macos:
	$(CARGO) build --locked --release --target aarch64-apple-darwin --package image_editor_desktop --no-default-features --features $(MACOS_PACKAGE_FEATURES)
	$(CARGO) run --locked --package image_editor_desktop --no-default-features --features $(MACOS_PACKAGE_FEATURES) --bin generate-capabilities -- --profile macos-aarch64 --output $(DIST_DIR)/macos-aarch64/capabilities.json
	mkdir -p "$(DIST_DIR)/macos-aarch64/Image Editor.app/Contents/MacOS"
	cp "target/aarch64-apple-darwin/release/image-editor" "$(DIST_DIR)/macos-aarch64/Image Editor.app/Contents/MacOS/image-editor"

# Produces a Linux executable beside its `resources` directory and package metadata.
package-linux:
	$(CARGO) build --locked --release --target x86_64-unknown-linux-gnu --package image_editor_desktop --no-default-features --features $(LINUX_PACKAGE_FEATURES)
	$(CARGO) run --locked --package image_editor_desktop --no-default-features --features $(LINUX_PACKAGE_FEATURES) --bin generate-capabilities -- --profile linux-x86_64-portal --output $(DIST_DIR)/linux-x86_64-portal/capabilities.json
	cp "target/x86_64-unknown-linux-gnu/release/image-editor" "$(DIST_DIR)/linux-x86_64-portal/image-editor"
