# Implementation Plan: macOS Image Editor

## Overview

Implement the application as a Rust workspace with a platform-independent `image_editor_core`, codec and platform adapters, and an `eframe`/`egui` desktop host. Build and test the deterministic core first, then add guarded I/O, the single-window workspace, and target-specific package metadata. Each task is an incremental prompt for a code-generation agent; later tasks extend and wire the previous working code rather than creating disconnected components.

## Tasks

- [x] 1. Establish the Rust workspace and shared domain contracts
  - [x] 1.1 Create the pinned Cargo workspace and crate boundaries
    - Create `image_editor_core`, `image_editor_codecs`, `image_editor_platform`, and `image_editor_desktop` crates with exact dependency versions and feature gates for portable codecs, HEIC, XDG Portal, and GTK.
    - Add shared error/result conventions and compile-time feature wiring without making optional runtime dependencies startup prerequisites.
    - _Requirements: 10.1, 10.5, 11.1, 11.2_
  - [x] 1.2 Implement core value types and invariant-enforcing constructors
    - Define image formats/capabilities, UTF-8 paths and filenames, source identities, collection entries, canonical RGBA16 images, crop rectangles, edit operations, drafts, notices, and application errors.
    - Enforce checked image-buffer sizing, nonzero dimensions, half-open crop invariants, and `[-100, 100]` adjustment ranges at the pure-core boundary.
    - _Requirements: 3.1-3.4, 4.3-4.6, 5.3-5.11, 9.1, 9.5-9.8_
  - [x] 1.3 Write core model and invariant unit tests
    - Exercise overflow rejection, malformed/unsupported values, crop-bound validation, adjustment-range validation, and safe visible-error construction.
    - _Requirements: 4.4-4.6, 5.3-5.8, 9.3, 9.6_

- [x] 2. Implement capability detection, collection discovery, and conservative projections
  - [x] 2.1 Implement the codec registry and startup capability snapshot
    - Add portable JPEG/PNG/TIFF registration and decode/encode self-check interfaces; model optional HEIC decode/encode availability independently with human-readable diagnostics.
    - Return a complete immutable `CapabilitySnapshot` before open-folder or export commands can be accepted.
    - _Requirements: 9.1, 9.2, 9.5-9.8, 11.2, 11.3_
  - [x] 2.2 Implement pure direct-folder collection planning
    - Filter only direct regular `.jpg`, `.jpeg`, `.png`, `.tif`, `.tiff`, and `.heic` candidates; retain undecodable candidates as availability notices and order supported entries by UTF-8 filename then full path bytes.
    - Expose folder-enumeration success/failure inputs without mutating prior browsing state.
    - _Requirements: 1.2, 1.3, 1.4, 9.3, 9.4_
  - [x] 2.3 Write property test for capability-filtered collection planning
    - **Property 1: Capability-filtered collection is complete and ordered.**
    - Generate direct/descendant/directory entries, valid UTF-8 paths, supported extensions, and capability combinations; compare against a reference ordering oracle.
    - **Validates: Requirements 1.2, 1.4, 9.3, 9.4.**
  - [x] 2.4 Implement pure capability-to-view/export projection
    - Derive selectable image entries, export format choices, disabled dependent operations, and non-blocking availability messages from independent format and dialog capabilities.
    - Ensure JPEG/PNG/TIFF operations remain usable when HEIC capability is unavailable.
    - _Requirements: 7.1, 8.1, 9.2-9.8, 11.2, 11.3_
  - [x] 2.5 Write property test for conservative capability projection
    - **Property 9: Capability projection is conservative and format-specific.**
    - Generate decode/encode/folder-picker/save-picker truth tables and verify enabled controls, notices, and format choices against the requirement matrix.
    - **Validates: Requirements 7.1, 8.1, 9.1-9.8, 11.2, 11.3.**

- [x] 3. Add the reducer, asynchronous-effect protocol, and image navigation
  - [x] 3.1 Implement immutable editor state, reducer, and revision-token effects
    - Add browsing state, per-image documents, interaction mode, pending effects, and typed completion commands.
    - Make every effect completion conditional on its request/revision token so stale enumeration, decode, preview, and export work cannot overwrite newer state.
    - _Requirements: 1.3, 1.7, 3.6, 4.2, 5.12, 6.6_
  - [x] 3.2 Implement selection and navigation planning with atomic decode completion
    - Add selection plus Left, Right, Home, and End commands with no wrapping; require a successful decode completion before replacing the active image and preview.
    - Preserve prior browsing state and emit filename-specific errors for decode failures; expose the required empty/no-active navigation states.
    - _Requirements: 1.5-1.7, 2.1-2.13_
  - [x] 3.3 Write property test for atomic candidate activation
    - **Property 2: Candidate activation is atomic.**
    - Generate prior browsing states and successful/failed selection or navigation completions; assert complete state retention on error and exact activation on success.
    - **Validates: Requirements 1.5-1.7, 2.1-2.9.**
  - [x] 3.4 Write property test for navigation ordering and boundaries
    - **Property 3: Navigation targets obey collection order and boundaries.**
    - Generate ordered, empty, and no-active collections to check targets, boundary no-ops, and retained state.
    - **Validates: Requirements 2.10-2.13.**

- [x] 4. Implement deterministic canonical image editing and crop interaction
  - [x] 4.1 Implement canonical decode normalization and history replay pipeline
    - Convert decoded images once to explicit straight-alpha sRGB RGBA16 after source-orientation normalization; render full-resolution results from immutable base pixels, ordered history, and drafts.
    - Keep cache keys revision-based while routing cached and uncached evaluation through the same core operations.
    - _Requirements: 7.7, 10.1, 10.2_
  - [x] 4.2 Implement fixed-integer flip and rotation operations
    - Implement horizontal/vertical flip and clockwise/counterclockwise 90-degree mappings with required dimensions, append semantics, redo clearing, preview revision effects, and no-active errors.
    - _Requirements: 3.1-3.6, 6.3_
  - [x] 4.3 Write property test for geometric mappings
    - **Property 4: Geometric operations preserve their specified pixel mapping.**
    - Generate asymmetric nonempty RGBA16 images with distinguishable pixels; verify every mapping, output dimensions, and four-clockwise-rotation identity.
    - **Validates: Requirements 3.1-3.5.**
  - [x] 4.4 Implement source-coordinate crop draft, validation, confirmation, and cancellation
    - Add crop entry, source-pixel draft clamping, nonempty in-bounds confirmation that appends exactly one crop, invalid-confirmation state retention, cancellation, and no-active errors.
    - _Requirements: 4.1-4.7, 6.3_
  - [x] 4.5 Write property test for crop transaction behavior
    - **Property 5: Crop is bounded, exact, and transactional.**
    - Generate valid and invalid untrusted bounds to verify exact half-open copied pixels, retained crop state on failure, and cancellation behavior.
    - **Validates: Requirements 4.3-4.7.**

- [ ] 5. Implement adjustment, history, and cross-platform shortcut behavior
  - [x] 5.1 Implement brightness and contrast draft interaction and deterministic arithmetic
    - Add focus, one-step clamped increase/decrease, preview application order, and `Return` commit behavior including zero-value operations and focused-draft reset.
    - Use specified fixed-point rounding/clamping while leaving alpha unchanged and preserving preview/state for no-active commands.
    - _Requirements: 5.1-5.12, 6.3_
  - [x] 5.2 Write property test for adjustment clamping and commits
    - **Property 6: Adjustment commands are clamped and commit exactly their draft.**
    - Generate focused adjustment command sequences and values to verify bounds, identity at zero, exact single commit, reset, and matching pixels.
    - **Validates: Requirements 5.1-5.11.**
  - [x] 5.3 Implement per-image undo and redo reducer transitions
    - Move operations LIFO between each active document's history and redo stacks; preserve empty-stack/no-active state and clear only the edited document's redo stack after a new operation.
    - _Requirements: 6.1-6.6_
  - [x] 5.4 Write property test for reversible, branch-safe per-image history
    - **Property 7: Per-image history is reversible and branch-safe.**
    - Generate multi-document edit histories and compare undo/redo behavior to a reference stack model.
    - **Validates: Requirements 6.1-6.6.**
  - [x] 5.5 Implement pure macOS/Linux shortcut resolution
    - Normalize raw pressed/released/repeated key events and map Command/Control, Option/Alt, navigation, edit, and adjustment inputs to one semantic command per accepted non-repeat press.
    - Supply runtime-correct shortcut labels and ignore events consumed by text-capable controls.
    - _Requirements: 8.3, 8.5, 8.6, 10.3, 10.4_
  - [x] 5.6 Write property test for platform-invariant shortcut semantics
    - **Property 8: Shortcut resolution has platform-invariant semantics.**
    - Generate defined shortcut intents and raw event variants; compare macOS/Linux semantic commands and reject releases/repeats.
    - **Validates: Requirements 8.3, 8.5, 8.6, 10.3, 10.4.**

- [x] 6. Checkpoint - Ensure core tests pass
  - Ensure all tests pass, ask the user if questions arise.

- [ ] 7. Add guarded codecs, filesystem operations, and non-destructive export
  - [x] 7.1 Implement bounded codec decode/encode adapters
    - Implement `ImageCodec` dispatch, resource limits, malformed-file failures, portable lossless PNG/TIFF conversion, JPEG encoding, and runtime-probed optional HEIC registration.
    - Keep unavailable codec capability distinct from a content decode failure.
    - _Requirements: 1.5-1.7, 7.7, 9.1, 9.5-9.8_
  - [x] 7.2 Write codec integration tests with real fixtures
    - Test JPEG/PNG/TIFF decode/encode, malformed supported content, resource-limit failures, PNG/TIFF sample equivalence, JPEG tolerance, and HEIC cases only when the detected adapter is available.
    - _Requirements: 1.5-1.7, 7.7, 9.5, 9.6_
  - [x] 7.3 Implement file identity checks and export planning
    - Add source/target identity resolution and `ExportPlan` validation that rejects source targets and pre-existing regular targets before opening a writer, retaining immutable source identity and document revision.
    - _Requirements: 7.2-7.6_
  - [x] 7.4 Write property test for non-replacement export planning
    - **Property 11: Export planning never permits replacement.**
    - Generate source and destination identity/existence combinations to verify all conflicts are rejected and valid plans retain source identity and revision.
    - **Validates: Requirements 7.2-7.6.**
  - [x] 7.5 Implement create-new export execution and typed completion handling
    - Render the planned full-resolution result, encode through a sibling temporary/new destination, flush, publish only without replacement, clean up only attempt-created failures, and report path-specific errors without state mutation.
    - _Requirements: 7.2-7.7_
  - [-] 7.6 Write filesystem/export integration tests
    - Use temporary directories to verify source and existing target bytes survive conflicts/failures, create-new races fail safely, and reopened portable exports meet format-appropriate equivalence.
    - _Requirements: 7.2-7.7_

- [ ] 8. Implement platform adapters and the single-window desktop workspace
  - [-] 8.1 Implement macOS/Linux platform capability, dialog, and filesystem adapters
    - Probe native folder/save dialog availability before requests; use `rfd` backends with macOS and XDG Portal/GTK Linux detection, map runtime loss to availability notices, and provide platform file identities.
    - Keep unavailable platform integrations non-fatal and disable only their dependent operations.
    - _Requirements: 1.1, 7.1, 9.1, 9.2, 9.7, 11.2-11.5_
  - [x] 8.2 Write platform adapter integration tests
    - On hosted macOS/Linux runners, test capability probe ordering, single-folder/single-path dialog configuration, runtime dialog failure downgrade, and filesystem identity behavior; gate non-automatable native interactions.
    - _Requirements: 1.1, 7.1, 9.1, 9.2, 9.7, 11.4, 11.5_
  - [x] 8.3 Implement the eframe application host and worker orchestration
    - Start exactly one primary window, construct capabilities before enabling requests, execute dialogs on the UI thread and I/O/render/export effects on a bounded worker executor, then return typed reducer completions.
    - _Requirements: 8.4, 9.1, 10.5, 11.2_
  - [x] 8.4 Implement the capability-aware egui workspace and preview texture cache
    - Render collection, complete active filename, preview/empty/pending/error states, source-pixel crop overlay conversion, and all applicable visible command controls in the sole window.
    - Bind controls and normalized keys to the shared command router; disable unavailable/inapplicable operations with explanatory notices and upload GPU textures only for new preview revisions.
    - _Requirements: 1.4, 4.3-4.4, 8.1-8.6, 9.3-9.7, 11.2_
  - [x] 8.5 Write desktop integration and visual regression tests
    - Verify one-window startup, complete control visibility with an active image, complete filename display, one reducer command per accepted key press, source-pixel crop overlay conversion, disabled-capability notices, and platform-specific shortcut labels.
    - _Requirements: 4.3-4.4, 8.1-8.6, 9.3-9.7, 11.2_

- [ ] 9. Wire cross-platform conformance and distributable capability metadata
  - [x] 9.1 Implement conformance fixtures, deterministic result serialization, and cross-platform pipeline harness
    - Add fixed lossless PNG fixtures and execute shared operation/draft sequences through both runtime command tables; serialize normalized dimensions, crop state, and RGBA16 samples for comparison.
    - _Requirements: 7.7, 10.1-10.5_
  - [x] 9.2 Write property test for shared pipeline equivalence
    - **Property 10: Shared pipeline is platform-equivalent.**
    - Generate valid operation/draft sequences over conformance images and verify platform-table equivalence plus PNG/TIFF lossless encode/decode samples.
    - **Validates: Requirements 7.7, 10.1, 10.2, 10.5.**
  - [x] 9.3 Add target-specific package manifests and capability manifest generation
    - Create macOS and Linux packaging/build configuration that records target, locked toolchain, portable codecs, optional HEIC provider, and selected dialog backend in machine-readable `capabilities.json`.
    - Preserve feature/runtime separation so package metadata never becomes startup truth or a hidden optional-dependency requirement.
    - _Requirements: 11.1-11.3_
  - [x] 9.4 Write packaging smoke and cross-platform integration tests
    - On macOS and Linux CI runners, verify package startup opens one window with optional HEIC/dialog dependencies both present and absent, and compare conformance artifacts plus PNG/TIFF export-reopen results.
    - _Requirements: 8.4, 10.1, 10.2, 10.5, 11.1-11.5_
  - [~] 9.5 Wire the final application composition and CI quality gates
    - Connect workspace crates into the shipping binary and add deterministic commands for formatting, clippy, core tests, property tests with at least 100 cases/property, target builds, and hosted integration suites.
    - Ensure all implemented code paths are reachable from the primary window and all optional feature combinations compile.
    - _Requirements: 1.1-11.5_

- [x] 10. Final checkpoint - Ensure all tests pass
  - Ensure all tests pass, ask the user if questions arise.

## Notes

- Tasks marked with `*` are optional test tasks and can be skipped for an MVP; all non-optional tasks are implementation work.
- Property tests are included because the design defines eleven correctness properties. Each test task must use `proptest`, run at least 100 cases, and begin with the specified feature/property traceability comment.
- Native dialog, package, and cross-platform tests require appropriate hosted macOS/Linux CI capabilities; gated tests must not change the behavior of the pure-core test suite.
- Checkpoints are intentionally excluded from the dependency graph because they do not write, modify, or test a discrete code component.

## Task Dependency Graph

```json
{
  "waves": [
    { "id": 0, "tasks": ["1.1"] },
    { "id": 1, "tasks": ["1.2"] },
    { "id": 2, "tasks": ["1.3", "2.1"] },
    { "id": 3, "tasks": ["2.2"] },
    { "id": 4, "tasks": ["2.3", "2.4"] },
    { "id": 5, "tasks": ["2.5", "3.1"] },
    { "id": 6, "tasks": ["3.2"] },
    { "id": 7, "tasks": ["3.3", "3.4", "4.1"] },
    { "id": 8, "tasks": ["4.2"] },
    { "id": 9, "tasks": ["4.3", "4.4"] },
    { "id": 10, "tasks": ["4.5", "5.1"] },
    { "id": 11, "tasks": ["5.2", "5.3"] },
    { "id": 12, "tasks": ["5.4", "5.5"] },
    { "id": 13, "tasks": ["5.6", "7.1"] },
    { "id": 14, "tasks": ["7.2", "7.3"] },
    { "id": 15, "tasks": ["7.4", "7.5"] },
    { "id": 16, "tasks": ["7.6", "8.1"] },
    { "id": 17, "tasks": ["8.2", "8.3"] },
    { "id": 18, "tasks": ["8.4"] },
    { "id": 19, "tasks": ["8.5", "9.1"] },
    { "id": 20, "tasks": ["9.2", "9.3"] },
    { "id": 21, "tasks": ["9.4", "9.5"] }
  ]
}
```
