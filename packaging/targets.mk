CARGO ?= cargo
DIST_DIR ?= dist

.PHONY: package-macos package-linux

# Produces the macOS binary and the capability manifest shipped beside its package artifact.
package-macos:
	$(CARGO) build --locked --release --target aarch64-apple-darwin --package image_editor_desktop --features macos-dialogs
	$(CARGO) run --locked --package image_editor_desktop --bin generate-capabilities -- --profile macos-aarch64 --output $(DIST_DIR)/macos-aarch64/capabilities.json

# Produces the Linux binary and the capability manifest shipped beside its package artifact.
package-linux:
	$(CARGO) build --locked --release --target x86_64-unknown-linux-gnu --package image_editor_desktop --features xdg-portal
	$(CARGO) run --locked --package image_editor_desktop --bin generate-capabilities -- --profile linux-x86_64-portal --output $(DIST_DIR)/linux-x86_64-portal/capabilities.json
