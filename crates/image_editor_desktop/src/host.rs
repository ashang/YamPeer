//! `eframe` host and effect boundary for the single Image Editor window.
//!
//! The reducer remains the sole owner of editor state. Native dialogs stay on
//! the UI thread, while every filesystem, codec, replay, and export operation
//! is represented by a typed completion sent back from a bounded worker pool.

use std::{
    fs,
    path::Path,
    sync::{
        Arc, Mutex,
        mpsc::{self, Receiver, Sender, TryRecvError},
    },
    thread::{self, JoinHandle},
    time::Duration,
};

use eframe::egui;
use image_editor_codecs::{
    CodecError, CodecRegistry, DecodeLimits, StartupPlatformCapabilities, complete_export_request,
};
use image_editor_core::{
    AbsolutePath, AdjustmentKind, ApplicationError, Availability, CapabilitySnapshot, CropDraft,
    DirectoryEntry, DirectoryEntryKind, DirectoryEntryLocation, EditorCommand, EditorState, Effect,
    ErrorCategory, FolderEnumerationInput, ImageFormat, InteractionMode, KeyModifiers,
    NoticeSeverity, NoticeSubject, PreviewState, RawKeyEvent, Revision, RuntimePlatform, SafeError,
    ShortcutKey, Utf8FileName, VisibleNotice, plan_folder_enumeration, reduce,
    render_current_editing_result, resolve_shortcut, shortcut_label,
};
use image_editor_platform::PlatformDialogs;
#[cfg(any(feature = "macos-dialogs", feature = "xdg-portal", feature = "gtk"))]
use image_editor_platform::RfdDialogBackend;
#[cfg(not(any(feature = "macos-dialogs", feature = "xdg-portal", feature = "gtk")))]
use image_editor_platform::{
    DialogFailure, FolderDialogRequest, PlatformDialogBackend, SaveDialogRequest,
};

const MAX_WORKER_THREADS: usize = 4;

/// Selects the dialog adapter linked for this platform, or an explicitly
/// unavailable adapter when packaging did not include one.
struct UiDialogs {
    #[cfg(any(feature = "macos-dialogs", feature = "xdg-portal", feature = "gtk"))]
    inner: PlatformDialogs<RfdDialogBackend>,
    #[cfg(not(any(feature = "macos-dialogs", feature = "xdg-portal", feature = "gtk")))]
    inner: PlatformDialogs<UnavailableDialogBackend>,
}

impl UiDialogs {
    fn detect() -> Self {
        #[cfg(any(feature = "macos-dialogs", feature = "xdg-portal", feature = "gtk"))]
        {
            Self {
                inner: PlatformDialogs::detect(RfdDialogBackend),
            }
        }
        #[cfg(not(any(feature = "macos-dialogs", feature = "xdg-portal", feature = "gtk")))]
        Self {
            inner: PlatformDialogs::detect(UnavailableDialogBackend),
        }
    }

    fn folder_picker_available(&self) -> image_editor_core::PlatformCapability {
        self.inner.folder_picker_available()
    }

    fn save_picker_available(&self) -> image_editor_core::PlatformCapability {
        self.inner.save_picker_available()
    }

    fn pick_folder(&self) -> image_editor_core::Result<Option<AbsolutePath>> {
        self.inner.pick_folder()
    }

    fn pick_export_target(
        &self,
        format: ImageFormat,
    ) -> image_editor_core::Result<Option<AbsolutePath>> {
        self.inner.pick_export_target(format)
    }
}

#[cfg(not(any(feature = "macos-dialogs", feature = "xdg-portal", feature = "gtk")))]
struct UnavailableDialogBackend;

#[cfg(not(any(feature = "macos-dialogs", feature = "xdg-portal", feature = "gtk")))]
impl PlatformDialogBackend for UnavailableDialogBackend {
    fn probe_folder_picker(&self) -> std::result::Result<String, DialogFailure> {
        Err(DialogFailure::new(
            "no native folder dialog backend was compiled for this platform",
        ))
    }

    fn probe_save_picker(&self) -> std::result::Result<String, DialogFailure> {
        Err(DialogFailure::new(
            "no native save dialog backend was compiled for this platform",
        ))
    }

    fn pick_folder(
        &self,
        _: FolderDialogRequest,
    ) -> std::result::Result<Option<std::path::PathBuf>, DialogFailure> {
        Err(DialogFailure::new("folder dialog is unavailable"))
    }

    fn pick_export_target(
        &self,
        _: SaveDialogRequest,
    ) -> std::result::Result<Option<std::path::PathBuf>, DialogFailure> {
        Err(DialogFailure::new("save dialog is unavailable"))
    }
}

/// Starts the only native `eframe` window owned by this process.
pub fn run() -> eframe::Result {
    let native_options = eframe::NativeOptions::default();
    eframe::run_native(
        "Image Editor",
        native_options,
        Box::new(|creation_context| {
            // This is the first creation-callback operation. Do not construct
            // the editable workspace until the packaged font is readable,
            // parseable, and registered on the eframe-provided context.
            let application: Box<dyn eframe::App> =
                match image_editor_desktop::font_bootstrap::FontBootstrapper::for_current_package()
                    .and_then(|bootstrapper| bootstrapper.install(&creation_context.egui_ctx))
                {
                    Ok(()) => Box::new(DesktopApp::new()),
                    Err(failure) => Box::new(StartupAvailabilityErrorApp { failure }),
                };
            Ok(application)
        }),
    )
}

/// A deliberately non-editable, ASCII-only safe error state.
///
/// It accepts no editor commands and renders no Required_Text, so a missing or
/// rejected CJK font can never degrade into missing-glyph boxes in the normal
/// workspace.
struct StartupAvailabilityErrorApp {
    failure: image_editor_desktop::font_bootstrap::FontBootstrapFailure,
}

impl eframe::App for StartupAvailabilityErrorApp {
    fn update(&mut self, context: &egui::Context, _: &mut eframe::Frame) {
        egui::CentralPanel::default().show(context, |ui| {
            ui.heading("Startup Availability Error");
            ui.label(self.failure.safe_message());
        });
    }
}

/// The stateful native application adapter. All methods are UI-thread confined.
pub struct DesktopApp {
    dialogs: UiDialogs,
    registry: Arc<CodecRegistry>,
    state: EditorState,
    workers: WorkerExecutor,
    completions: Receiver<EditorCommand>,
    preview_texture: PreviewTextureCache,
    crop_drag_start: Option<egui::Pos2>,
    /// Dialog probes are immutable in the core snapshot, but a service may
    /// disappear after startup. Keep those session-local downgrades visible
    /// and consult the live adapter before rendering either dependent control.
    runtime_dialog_notices: Vec<VisibleNotice>,
}

impl DesktopApp {
    pub fn new() -> Self {
        // Capability probing completes before the state is exposed to controls
        // that may request either native dialog.
        let dialogs = UiDialogs::detect();
        let registry = Arc::new(CodecRegistry::detect(StartupPlatformCapabilities::new(
            dialogs.folder_picker_available(),
            dialogs.save_picker_available(),
        )));
        let state = EditorState::new(registry.snapshot().clone());
        let (completion_sender, completions) = mpsc::channel();
        let workers = WorkerExecutor::new(Arc::clone(&registry), completion_sender);

        Self {
            dialogs,
            registry,
            state,
            workers,
            completions,
            preview_texture: PreviewTextureCache::default(),
            crop_drag_start: None,
            runtime_dialog_notices: Vec::new(),
        }
    }

    fn dispatch(&mut self, command: EditorCommand) {
        let reduction = reduce(&self.state, command);
        self.state = reduction.state;
        let capabilities = self.registry.snapshot().clone();
        for effect in reduction.effects {
            self.workers
                .submit(WorkerTask::from_effect(effect, capabilities.clone()));
        }
    }

    fn apply_completed_work(&mut self) {
        loop {
            match self.completions.try_recv() {
                Ok(completion) => self.dispatch(completion),
                Err(TryRecvError::Empty | TryRecvError::Disconnected) => return,
            }
        }
    }

    /// Runs the folder dialog directly in the UI callback, then queues only its
    /// filesystem enumeration completion boundary for a worker.
    fn choose_folder_on_ui_thread(&mut self) {
        if !self.dialogs.folder_picker_available().is_available() {
            return;
        }
        match self.dialogs.pick_folder() {
            Ok(Some(folder)) => self.dispatch(EditorCommand::BeginFolderEnumeration { folder }),
            Ok(None) => {}
            Err(error) => self.record_runtime_dialog_failure(error),
        }
    }

    /// Runs the save dialog directly in the UI callback. Format selection is
    /// rendered by the workspace task; this host accepts its selected format.
    fn choose_export_on_ui_thread(&mut self, format: ImageFormat) {
        if !self.dialogs.save_picker_available().is_available()
            || !self.state.capabilities().format(format).can_encode()
        {
            return;
        }
        match self.dialogs.pick_export_target(format) {
            Ok(Some(target)) => self.dispatch(EditorCommand::BeginExport { target, format }),
            Ok(None) => {}
            Err(error) => self.record_runtime_dialog_failure(error),
        }
    }

    fn record_runtime_dialog_failure(&mut self, error: ApplicationError) {
        let mut notice = error.to_notice();
        notice.severity = NoticeSeverity::Availability;
        self.runtime_dialog_notices
            .retain(|existing| existing.subject != notice.subject);
        self.runtime_dialog_notices.push(notice);
    }
}

/// One GPU texture keyed by the immutable preview identity and replay revision.
///
/// Immediate-mode UI construction may run every frame, but texture uploads only
/// occur after the worker installs a different rendered preview revision.
#[derive(Default)]
struct PreviewTextureCache {
    key: Option<PreviewTextureKey>,
    texture: Option<egui::TextureHandle>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct PreviewTextureKey {
    image_id: image_editor_core::ImageId,
    revision: Revision,
}

impl PreviewTextureCache {
    fn needs_upload(&self, image_id: &image_editor_core::ImageId, revision: Revision) -> bool {
        self.key.as_ref()
            != Some(&PreviewTextureKey {
                image_id: image_id.clone(),
                revision,
            })
    }

    fn synchronize(
        &mut self,
        context: &egui::Context,
        image_id: &image_editor_core::ImageId,
        revision: Revision,
        image: &image_editor_core::CanonicalImage,
    ) {
        if !self.needs_upload(image_id, revision) {
            return;
        }

        self.texture = Some(context.load_texture(
            "image-editor-preview",
            canonical_color_image(image),
            egui::TextureOptions::LINEAR,
        ));
        self.key = Some(PreviewTextureKey {
            image_id: image_id.clone(),
            revision,
        });
    }

    fn texture(&self) -> Option<&egui::TextureHandle> {
        self.texture.as_ref()
    }
}

fn canonical_color_image(image: &image_editor_core::CanonicalImage) -> egui::ColorImage {
    let mut pixels = Vec::with_capacity(image.pixels().len() * 4);
    for pixel in image.pixels() {
        // The UI preview is an 8-bit presentation of the core's canonical
        // RGBA16 data. Export and editing always retain the full precision.
        pixels.extend([
            (pixel.red >> 8) as u8,
            (pixel.green >> 8) as u8,
            (pixel.blue >> 8) as u8,
            (pixel.alpha >> 8) as u8,
        ]);
    }
    egui::ColorImage::from_rgba_unmultiplied(
        [image.width() as usize, image.height() as usize],
        &pixels,
    )
}

impl DesktopApp {
    fn runtime_platform() -> RuntimePlatform {
        if cfg!(target_os = "macos") {
            RuntimePlatform::MacOs
        } else {
            RuntimePlatform::Linux
        }
    }

    fn route_keyboard(&mut self, context: &egui::Context) {
        let platform = Self::runtime_platform();
        let consumed_by_text_control = context.wants_keyboard_input();
        let commands = context.input(|input| {
            input
                .events
                .iter()
                .filter_map(|event| raw_key_event(platform, event, consumed_by_text_control))
                .filter_map(|event| resolve_shortcut(platform, event))
                .collect::<Vec<_>>()
        });
        for command in commands {
            self.dispatch(command);
        }
    }

    fn synchronize_preview_texture(&mut self, context: &egui::Context) {
        let preview = self.state.browsing().preview().clone();
        if let PreviewState::Rendered {
            image_id,
            revision,
            image,
        } = preview
        {
            self.preview_texture
                .synchronize(context, &image_id, revision, &image);
        }
    }

    fn command_button(
        &mut self,
        ui: &mut egui::Ui,
        label: &str,
        command: EditorCommand,
        enabled: bool,
        disabled_reason: &str,
    ) {
        let title = command_title(Self::runtime_platform(), label, &command);
        let response = ui.add_enabled(enabled, egui::Button::new(title));
        if !enabled {
            response.on_disabled_hover_text(disabled_reason);
        } else if response.clicked() {
            self.dispatch(command);
        }
    }

    fn render_collection_pane(&mut self, ui: &mut egui::Ui) {
        ui.heading("图像集合");
        let live_folder_capability = self.dialogs.folder_picker_available();
        let folder_available = self.state.capabilities().folder_picker().is_available()
            && live_folder_capability.is_available();
        let folder_reason = unavailable_reason(
            live_folder_capability.availability(),
            &unavailable_reason(
                self.state.capabilities().folder_picker().availability(),
                "当前平台没有可用的文件夹选择器",
            ),
        );
        let response = ui.add_enabled(folder_available, egui::Button::new("打开文件夹"));
        if !folder_available {
            response.on_disabled_hover_text(folder_reason);
        } else if response.clicked() {
            self.choose_folder_on_ui_thread();
        }

        ui.separator();
        let active = self.state.browsing().active().cloned();
        let entries = self.state.browsing().collection().entries().to_vec();
        if entries.is_empty() {
            ui.weak("当前集合没有可显示的受支持图像。");
        }
        egui::ScrollArea::vertical().show(ui, |ui| {
            for entry in entries {
                let selected = active.as_ref() == Some(&entry.id);
                if ui
                    .selectable_label(selected, entry.file_name.as_str())
                    .clicked()
                {
                    self.dispatch(EditorCommand::SelectImage { image_id: entry.id });
                }
            }
        });

        let file_notices = self
            .state
            .notices()
            .iter()
            .filter(|notice| matches!(notice.subject, NoticeSubject::FileName(_)))
            .cloned()
            .collect::<Vec<_>>();
        if !file_notices.is_empty() {
            ui.separator();
            ui.label("可用性说明");
            for notice in &file_notices {
                render_notice(ui, notice);
            }
        }
    }

    fn render_preview_pane(&mut self, ui: &mut egui::Ui) {
        let active_name = self.state.browsing().active().and_then(|active| {
            self.state
                .browsing()
                .collection()
                .entries()
                .iter()
                .find(|entry| &entry.id == active)
                .map(|entry| entry.file_name.as_str().to_owned())
        });
        ui.heading(active_name.as_deref().unwrap_or("预览"));
        ui.separator();

        let preview = self.state.browsing().preview().clone();
        match preview {
            PreviewState::EmptyCollection => {
                ui.centered_and_justified(|ui| ui.weak("打开文件夹以浏览图像。"));
            }
            PreviewState::NoActiveImage => {
                ui.centered_and_justified(|ui| ui.weak("从集合中选择一张图像以开始编辑。"));
            }
            PreviewState::Pending { .. } => {
                ui.centered_and_justified(|ui| {
                    ui.spinner();
                    ui.label("正在准备预览…");
                });
            }
            PreviewState::Rendered { image, .. } => {
                let Some(texture) = self.preview_texture.texture() else {
                    ui.centered_and_justified(|ui| ui.weak("正在上传预览纹理…"));
                    return;
                };
                let texture_id = texture.id();
                let source_size = egui::vec2(image.width() as f32, image.height() as f32);
                let available = ui.available_size();
                let scale = (available.x / source_size.x)
                    .min(available.y / source_size.y)
                    .clamp(0.01, 1.0);
                let displayed_size = source_size * scale;
                let response = ui.add(
                    egui::Image::new((texture_id, displayed_size))
                        .sense(egui::Sense::click_and_drag()),
                );

                if let InteractionMode::Crop(draft) = self.state.mode() {
                    draw_crop_overlay(ui, response.rect, draft, image.width(), image.height());
                    self.update_crop_from_drag(&response, image.width(), image.height());
                }
            }
        }
    }

    fn update_crop_from_drag(&mut self, response: &egui::Response, width: u32, height: u32) {
        if response.drag_started() {
            self.crop_drag_start = response.interact_pointer_pos();
        }
        if response.dragged() {
            if let (Some(start), Some(end)) =
                (self.crop_drag_start, response.interact_pointer_pos())
            {
                let (start_x, start_y) =
                    source_pixel_coordinate(start, response.rect, width, height);
                let (end_x, end_y) = source_pixel_coordinate(end, response.rect, width, height);
                self.dispatch(EditorCommand::UpdateCropDraft {
                    draft: CropDraft::new(
                        start_x.min(end_x),
                        start_y.min(end_y),
                        start_x.max(end_x),
                        start_y.max(end_y),
                    ),
                });
            }
        }
        if response.drag_stopped() {
            self.crop_drag_start = None;
        }
    }

    fn render_command_pane(&mut self, ui: &mut egui::Ui) {
        ui.heading("编辑命令");
        let active = self.state.browsing().active().cloned();
        let has_active = active.is_some();
        let no_active_reason = "请先从图像集合中选择一张图像";
        let (has_undo, has_redo, brightness, contrast) = active
            .as_ref()
            .and_then(|id| self.state.browsing().document(id))
            .map(|document| {
                (
                    !document.history().is_empty(),
                    !document.redo().is_empty(),
                    document.draft().brightness().get(),
                    document.draft().contrast().get(),
                )
            })
            .unwrap_or((false, false, 0, 0));
        let in_crop = matches!(self.state.mode(), InteractionMode::Crop(_));
        let in_adjust = matches!(self.state.mode(), InteractionMode::Adjust(_));

        ui.label("几何变换");
        self.command_button(
            ui,
            "水平翻转",
            EditorCommand::FlipHorizontal,
            has_active,
            no_active_reason,
        );
        self.command_button(
            ui,
            "垂直翻转",
            EditorCommand::FlipVertical,
            has_active,
            no_active_reason,
        );
        self.command_button(
            ui,
            "顺时针旋转",
            EditorCommand::RotateClockwise90,
            has_active,
            no_active_reason,
        );
        self.command_button(
            ui,
            "逆时针旋转",
            EditorCommand::RotateCounterclockwise90,
            has_active,
            no_active_reason,
        );
        self.command_button(
            ui,
            "开始裁剪",
            EditorCommand::EnterCrop,
            has_active && !in_crop,
            if has_active {
                "当前已处于裁剪模式"
            } else {
                no_active_reason
            },
        );
        self.command_button(
            ui,
            "确认裁剪",
            EditorCommand::ConfirmCrop,
            has_active && in_crop,
            "开始裁剪后才能确认；无效选择会保留当前裁剪状态",
        );
        self.command_button(
            ui,
            "取消裁剪",
            EditorCommand::CancelCrop,
            has_active && in_crop,
            "开始裁剪后才能取消",
        );

        ui.separator();
        ui.label(format!("调整：亮度 {brightness}，对比度 {contrast}"));
        self.command_button(
            ui,
            "调整亮度",
            EditorCommand::FocusAdjustment(AdjustmentKind::Brightness),
            has_active,
            no_active_reason,
        );
        self.command_button(
            ui,
            "调整对比度",
            EditorCommand::FocusAdjustment(AdjustmentKind::Contrast),
            has_active,
            no_active_reason,
        );
        self.command_button(
            ui,
            "增加调整",
            EditorCommand::IncreaseAdjustment,
            has_active && in_adjust,
            "先选择亮度或对比度调整",
        );
        self.command_button(
            ui,
            "减少调整",
            EditorCommand::DecreaseAdjustment,
            has_active && in_adjust,
            "先选择亮度或对比度调整",
        );
        self.command_button(
            ui,
            "提交调整",
            EditorCommand::CommitAdjustment,
            has_active && in_adjust,
            "先选择亮度或对比度调整",
        );

        ui.separator();
        self.command_button(
            ui,
            "撤销",
            EditorCommand::Undo,
            has_active && has_undo,
            if has_active {
                "没有可撤销的编辑"
            } else {
                no_active_reason
            },
        );
        self.command_button(
            ui,
            "重做",
            EditorCommand::Redo,
            has_active && has_redo,
            if has_active {
                "没有可重做的编辑"
            } else {
                no_active_reason
            },
        );

        ui.separator();
        ui.label("导出");
        let live_save_capability = self.dialogs.save_picker_available();
        let save_available = self.state.capabilities().save_picker().is_available()
            && live_save_capability.is_available();
        let export_formats = [
            ImageFormat::Jpeg,
            ImageFormat::Png,
            ImageFormat::Tiff,
            ImageFormat::Heic,
        ]
        .into_iter()
        .filter(|format| self.state.capabilities().format(*format).can_encode())
        .collect::<Vec<_>>();
        let export_reason = if !has_active {
            no_active_reason.to_owned()
        } else {
            unavailable_reason(
                live_save_capability.availability(),
                &unavailable_reason(
                    self.state.capabilities().save_picker().availability(),
                    "当前没有可用的导出文件选择器",
                ),
            )
        };
        if export_formats.is_empty() {
            let response = ui.add_enabled(false, egui::Button::new("导出（无可用格式）"));
            response.on_disabled_hover_text("当前环境没有可用的图像编码器");
        }
        for format in export_formats {
            let enabled = has_active && save_available;
            let response = ui.add_enabled(
                enabled,
                egui::Button::new(format!("导出 {}", format.display_name())),
            );
            if !enabled {
                response.on_disabled_hover_text(&export_reason);
            } else if response.clicked() {
                self.choose_export_on_ui_thread(format);
            }
        }

        ui.separator();
        ui.label(format!("{} 个请求正在处理", self.state.pending().len()));
        for notice in self
            .state
            .notices()
            .iter()
            .chain(self.runtime_dialog_notices.iter())
        {
            render_notice(ui, notice);
        }
    }
}

impl eframe::App for DesktopApp {
    fn update(&mut self, context: &egui::Context, _: &mut eframe::Frame) {
        self.apply_completed_work();
        self.route_keyboard(context);
        self.synchronize_preview_texture(context);

        egui::SidePanel::left("image-collection")
            .resizable(true)
            .default_width(220.0)
            .show(context, |ui| self.render_collection_pane(ui));
        egui::SidePanel::right("editor-commands")
            .resizable(true)
            .default_width(210.0)
            .show(context, |ui| self.render_command_pane(ui));
        egui::CentralPanel::default().show(context, |ui| self.render_preview_pane(ui));

        if !self.state.pending().is_empty() {
            context.request_repaint_after(Duration::from_millis(16));
        }
    }
}

fn raw_key_event(
    platform: RuntimePlatform,
    event: &egui::Event,
    consumed_by_text_control: bool,
) -> Option<RawKeyEvent> {
    let egui::Event::Key {
        key,
        pressed,
        repeat,
        modifiers,
        ..
    } = event
    else {
        return None;
    };
    let key = match key {
        egui::Key::ArrowUp => ShortcutKey::ArrowUp,
        egui::Key::ArrowDown => ShortcutKey::ArrowDown,
        egui::Key::ArrowLeft => ShortcutKey::ArrowLeft,
        egui::Key::ArrowRight => ShortcutKey::ArrowRight,
        egui::Key::Home => ShortcutKey::Home,
        egui::Key::End => ShortcutKey::End,
        egui::Key::Enter => ShortcutKey::Enter,
        egui::Key::B => ShortcutKey::Character('b'),
        egui::Key::C => ShortcutKey::Character('c'),
        egui::Key::D => ShortcutKey::Character('d'),
        egui::Key::F => ShortcutKey::Character('f'),
        egui::Key::R => ShortcutKey::Character('r'),
        egui::Key::Z => ShortcutKey::Character('z'),
        _ => return None,
    };
    let modifiers = KeyModifiers {
        command: matches!(platform, RuntimePlatform::MacOs) && modifiers.mac_cmd,
        control: matches!(platform, RuntimePlatform::Linux) && modifiers.ctrl,
        option: matches!(platform, RuntimePlatform::MacOs) && modifiers.alt,
        alt: matches!(platform, RuntimePlatform::Linux) && modifiers.alt,
        shift: modifiers.shift,
    };
    Some(RawKeyEvent {
        key,
        modifiers,
        pressed: *pressed,
        repeat: *repeat,
        consumed_by_text_control,
    })
}

fn command_title(platform: RuntimePlatform, label: &str, command: &EditorCommand) -> String {
    shortcut_label(platform, command)
        .map(|shortcut| format!("{label} ({shortcut})"))
        .unwrap_or_else(|| label.to_owned())
}

fn source_pixel_coordinate(
    point: egui::Pos2,
    rect: egui::Rect,
    width: u32,
    height: u32,
) -> (u32, u32) {
    let x = (((point.x - rect.left()) / rect.width()) * width as f32)
        .round()
        .clamp(0.0, width as f32) as u32;
    let y = (((point.y - rect.top()) / rect.height()) * height as f32)
        .round()
        .clamp(0.0, height as f32) as u32;
    (x, y)
}

fn draw_crop_overlay(ui: &egui::Ui, rect: egui::Rect, draft: CropDraft, width: u32, height: u32) {
    let map = |x: u32, y: u32| {
        egui::pos2(
            rect.left() + rect.width() * x as f32 / width as f32,
            rect.top() + rect.height() * y as f32 / height as f32,
        )
    };
    let crop = egui::Rect::from_min_max(map(draft.left, draft.top), map(draft.right, draft.bottom));
    let stroke = egui::Stroke::new(2.0, egui::Color32::from_rgb(98, 189, 255));
    let painter = ui.painter();
    painter.line_segment([crop.left_top(), crop.right_top()], stroke);
    painter.line_segment([crop.right_top(), crop.right_bottom()], stroke);
    painter.line_segment([crop.right_bottom(), crop.left_bottom()], stroke);
    painter.line_segment([crop.left_bottom(), crop.left_top()], stroke);
    painter.text(
        crop.left_top(),
        egui::Align2::LEFT_BOTTOM,
        format!(
            "{}, {} → {}, {} px",
            draft.left, draft.top, draft.right, draft.bottom
        ),
        egui::FontId::proportional(12.0),
        egui::Color32::WHITE,
    );
}

fn unavailable_reason(availability: &Availability, fallback: &str) -> String {
    match availability {
        Availability::Available => fallback.to_owned(),
        Availability::Unavailable { reason } => reason.summary().to_owned(),
    }
}

fn render_notice(ui: &mut egui::Ui, notice: &image_editor_core::VisibleNotice) {
    let subject = match &notice.subject {
        NoticeSubject::FileName(name) => name.as_str().to_owned(),
        NoticeSubject::Path(path) => path.as_str().to_owned(),
        NoticeSubject::Capability(capability) => capability.to_string(),
        NoticeSubject::Command(command) => command.to_string(),
    };
    let color = match notice.severity {
        NoticeSeverity::Availability => egui::Color32::YELLOW,
        NoticeSeverity::Error => egui::Color32::LIGHT_RED,
        NoticeSeverity::Info => egui::Color32::LIGHT_GREEN,
    };
    ui.colored_label(color, format!("{subject}: {}", notice.message.summary()));
}

enum WorkerTask {
    EnumerateFolder {
        token: image_editor_core::RequestToken,
        folder: AbsolutePath,
        capabilities: CapabilitySnapshot,
    },
    DecodeImage {
        token: image_editor_core::RequestToken,
        candidate: image_editor_core::CollectionEntry,
    },
    RenderPreview {
        token: image_editor_core::RequestToken,
        request: image_editor_core::PreviewRequest,
    },
    WriteExport {
        token: image_editor_core::RequestToken,
        request: image_editor_core::ExportRequest,
    },
}

impl WorkerTask {
    fn from_effect(effect: Effect, capabilities: CapabilitySnapshot) -> Self {
        match effect {
            Effect::EnumerateFolder { token, folder } => Self::EnumerateFolder {
                token,
                folder,
                capabilities,
            },
            Effect::DecodeImage { token, candidate } => Self::DecodeImage { token, candidate },
            Effect::RenderPreview { token, request } => Self::RenderPreview { token, request },
            Effect::WriteExport { token, request } => Self::WriteExport { token, request },
        }
    }

    fn complete(self, registry: &CodecRegistry) -> EditorCommand {
        match self {
            Self::EnumerateFolder {
                token,
                folder,
                capabilities,
            } => EditorCommand::FolderEnumerated {
                token,
                result: enumerate_folder(&folder, &capabilities),
            },
            Self::DecodeImage { token, candidate } => match registry.decode(
                candidate.format,
                &candidate.absolute_path,
                DecodeLimits::DEFAULT,
            ) {
                Ok(image) => EditorCommand::ImageDecoded { token, image },
                Err(error) => EditorCommand::OperationFailed {
                    token,
                    error: decode_error(candidate.file_name, error),
                },
            },
            Self::RenderPreview { token, request } => {
                match render_current_editing_result(
                    &request.source,
                    &request.history,
                    &request.draft,
                ) {
                    Ok(image) => EditorCommand::PreviewRendered { token, image },
                    Err(_) => EditorCommand::OperationFailed {
                        token,
                        error: ApplicationError::boundary(
                            "preview rendering",
                            SafeError::new(
                                ErrorCategory::Invariant,
                                "could not render image preview",
                            ),
                        ),
                    },
                }
            }
            Self::WriteExport { token, request } => {
                complete_export_request(registry, token, request)
            }
        }
    }
}

/// A fixed-size executor. Worker threads own no editor state and can only send
/// typed reducer commands; request tokens make stale results harmless.
struct WorkerExecutor {
    sender: Option<Sender<WorkerTask>>,
    workers: Vec<JoinHandle<()>>,
}

impl WorkerExecutor {
    fn new(registry: Arc<CodecRegistry>, completion_sender: Sender<EditorCommand>) -> Self {
        let (sender, receiver) = mpsc::channel();
        let receiver = Arc::new(Mutex::new(receiver));
        let worker_count = worker_count();
        let mut workers = Vec::with_capacity(worker_count);
        for index in 0..worker_count {
            let receiver = Arc::clone(&receiver);
            let registry = Arc::clone(&registry);
            let completion_sender = completion_sender.clone();
            workers.push(
                thread::Builder::new()
                    .name(format!("image-editor-worker-{index}"))
                    .spawn(move || worker_loop(receiver, registry, completion_sender))
                    .expect("could not start bounded image editor worker"),
            );
        }
        Self {
            sender: Some(sender),
            workers,
        }
    }

    fn submit(&self, task: WorkerTask) {
        // An executor only disconnects during application shutdown, after UI
        // callbacks have stopped accepting commands.
        let _ = self
            .sender
            .as_ref()
            .and_then(|sender| sender.send(task).ok());
    }
}

impl Drop for WorkerExecutor {
    fn drop(&mut self) {
        self.sender.take();
        for worker in self.workers.drain(..) {
            let _ = worker.join();
        }
    }
}

fn worker_count() -> usize {
    thread::available_parallelism()
        .map(usize::from)
        .unwrap_or(1)
        .clamp(1, MAX_WORKER_THREADS)
}

fn worker_loop(
    receiver: Arc<Mutex<Receiver<WorkerTask>>>,
    registry: Arc<CodecRegistry>,
    completion_sender: Sender<EditorCommand>,
) {
    loop {
        let task = match receiver
            .lock()
            .expect("worker receiver mutex poisoned")
            .recv()
        {
            Ok(task) => task,
            Err(_) => return,
        };
        if completion_sender.send(task.complete(&registry)).is_err() {
            return;
        }
    }
}

fn enumerate_folder(
    folder: &AbsolutePath,
    capabilities: &CapabilitySnapshot,
) -> image_editor_core::FolderEnumerationPlan {
    let entries = match read_direct_entries(folder) {
        Ok(entries) => FolderEnumerationInput::Succeeded {
            folder: folder.clone(),
            entries,
        },
        Err(cause) => FolderEnumerationInput::Failed {
            folder: folder.clone(),
            cause,
        },
    };
    plan_folder_enumeration(capabilities, entries)
}

fn read_direct_entries(folder: &AbsolutePath) -> Result<Vec<DirectoryEntry>, SafeError> {
    let mut entries = Vec::new();
    let directory = fs::read_dir(Path::new(folder.as_str())).map_err(|error| {
        SafeError::new(
            ErrorCategory::FileSystem,
            format!("could not enumerate selected folder: {}", error.kind()),
        )
    })?;
    for entry in directory {
        let entry = entry.map_err(|error| {
            SafeError::new(
                ErrorCategory::FileSystem,
                format!("could not read a folder entry: {}", error.kind()),
            )
        })?;
        let path = entry.path();
        let Some(path) = path.to_str() else {
            continue;
        };
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        let file_type = entry.file_type().map_err(|error| {
            SafeError::new(
                ErrorCategory::FileSystem,
                format!("could not inspect a folder entry: {}", error.kind()),
            )
        })?;
        let kind = if file_type.is_file() {
            DirectoryEntryKind::RegularFile
        } else if file_type.is_dir() {
            DirectoryEntryKind::Directory
        } else {
            DirectoryEntryKind::Other
        };
        let (Ok(path), Ok(name)) = (AbsolutePath::new(path.to_owned()), Utf8FileName::new(name))
        else {
            continue;
        };
        entries.push(DirectoryEntry::new(
            path,
            name,
            DirectoryEntryLocation::Direct,
            kind,
            None,
        ));
    }
    Ok(entries)
}

fn decode_error(file_name: Utf8FileName, error: CodecError) -> ApplicationError {
    match error {
        CodecError::ResourceLimit(limit) => ApplicationError::ResourceLimit {
            subject: file_name,
            limit,
        },
        CodecError::Unavailable { .. } => ApplicationError::Decode {
            file_name,
            cause: SafeError::new(
                ErrorCategory::OptionalDependency,
                "the required image decoder is unavailable",
            ),
        },
        CodecError::Content { .. } | CodecError::Input { .. } | CodecError::Output { .. } => {
            ApplicationError::Decode {
                file_name,
                cause: SafeError::new(ErrorCategory::PortableCodec, "could not decode image"),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        sync::{Arc, mpsc},
        time::{Duration, SystemTime, UNIX_EPOCH},
    };

    use image_editor_codecs::{CodecRegistry, StartupPlatformCapabilities};
    use image_editor_core::{
        AbsolutePath, EditorCommand, EditorState, PlatformCapability, Revision, SourceIdentity,
        reduce,
    };

    use super::{
        PreviewTextureCache, PreviewTextureKey, WorkerExecutor, WorkerTask,
        source_pixel_coordinate, worker_count,
    };

    #[test]
    fn worker_count_is_bounded() {
        assert!((1..=super::MAX_WORKER_THREADS).contains(&worker_count()));
    }

    #[test]
    fn worker_returns_typed_folder_enumeration_completion() {
        let folder = std::env::temp_dir().join(format!(
            "image-editor-desktop-host-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system clock is after Unix epoch")
                .as_nanos()
        ));
        fs::create_dir(&folder).expect("temporary folder is created");
        let folder = AbsolutePath::new(folder.to_string_lossy().into_owned())
            .expect("temporary folder path is absolute UTF-8");
        let registry = Arc::new(CodecRegistry::detect(StartupPlatformCapabilities::new(
            PlatformCapability::unavailable("test"),
            PlatformCapability::unavailable("test"),
        )));
        let state = EditorState::new(registry.snapshot().clone());
        let reduction = reduce(
            &state,
            EditorCommand::BeginFolderEnumeration {
                folder: folder.clone(),
            },
        );
        let effect = reduction
            .effects
            .into_iter()
            .next()
            .expect("enumeration effect");
        let (sender, receiver) = mpsc::channel();
        let workers = WorkerExecutor::new(Arc::clone(&registry), sender);
        workers.submit(WorkerTask::from_effect(effect, registry.snapshot().clone()));

        let completion = receiver
            .recv_timeout(Duration::from_secs(2))
            .expect("worker returns completion");
        assert!(matches!(completion, EditorCommand::FolderEnumerated { .. }));

        drop(workers);
        fs::remove_dir(folder.as_str()).expect("temporary folder is removed");
    }

    #[test]
    fn preview_texture_cache_reuses_only_the_same_image_revision() {
        let first = SourceIdentity::new(AbsolutePath::new("/photos/first.png").unwrap(), None);
        let second = SourceIdentity::new(AbsolutePath::new("/photos/second.png").unwrap(), None);
        let cache = PreviewTextureCache {
            key: Some(PreviewTextureKey {
                image_id: first.clone(),
                revision: Revision::INITIAL,
            }),
            ..Default::default()
        };

        assert!(!cache.needs_upload(&first, Revision::INITIAL));
        assert!(cache.needs_upload(&second, Revision::INITIAL));
    }

    #[test]
    fn crop_pointer_coordinates_are_converted_and_clamped_to_source_pixels() {
        let rect = eframe::egui::Rect::from_min_max(
            eframe::egui::pos2(10.0, 20.0),
            eframe::egui::pos2(110.0, 70.0),
        );
        assert_eq!(
            source_pixel_coordinate(eframe::egui::pos2(60.0, 45.0), rect, 200, 100),
            (100, 50)
        );
        assert_eq!(
            source_pixel_coordinate(eframe::egui::pos2(-1.0, 90.0), rect, 200, 100),
            (0, 100)
        );
    }
}

#[cfg(test)]
mod desktop_workspace_tests {
    use std::collections::BTreeMap;

    use eframe::egui;
    use image_editor_core::{
        AbsolutePath, Availability, CanonicalImage, CapabilitySnapshot, CodecProvider,
        DirectoryEntry, DirectoryEntryKind, DirectoryEntryLocation, EditorCommand, EditorState,
        FolderEnumerationInput, FormatCapability, ImageFormat, PlatformCapability, Rgba16,
        RuntimePlatform, Utf8FileName, plan_folder_enumeration, reduce,
    };

    use super::{
        DesktopApp, command_title, draw_crop_overlay, raw_key_event, source_pixel_coordinate,
    };

    fn capabilities() -> CapabilitySnapshot {
        let available = || {
            FormatCapability::new(
                Availability::Available,
                Availability::Available,
                Some(CodecProvider::PortableRust),
            )
        };
        let unavailable = || {
            FormatCapability::new(
                Availability::Unavailable {
                    reason: image_editor_core::AvailabilityReason::new("HEIC codec unavailable"),
                },
                Availability::Unavailable {
                    reason: image_editor_core::AvailabilityReason::new("HEIC codec unavailable"),
                },
                None,
            )
        };
        let mut formats = BTreeMap::new();
        formats.insert(ImageFormat::Jpeg, available());
        formats.insert(ImageFormat::Png, available());
        formats.insert(ImageFormat::Tiff, available());
        formats.insert(ImageFormat::Heic, unavailable());
        CapabilitySnapshot::new(
            formats,
            PlatformCapability::unavailable("folder picker unavailable"),
            PlatformCapability::unavailable("save picker unavailable"),
        )
    }

    fn package_smoke_capabilities(optional_dependencies_available: bool) -> CapabilitySnapshot {
        let portable = || {
            FormatCapability::new(
                Availability::Available,
                Availability::Available,
                Some(CodecProvider::PortableRust),
            )
        };
        let heic = || {
            if optional_dependencies_available {
                FormatCapability::new(
                    Availability::Available,
                    Availability::Available,
                    Some(CodecProvider::Libheif),
                )
            } else {
                FormatCapability::new(
                    Availability::Unavailable {
                        reason: image_editor_core::AvailabilityReason::new(
                            "HEIC runtime dependency is unavailable",
                        ),
                    },
                    Availability::Unavailable {
                        reason: image_editor_core::AvailabilityReason::new(
                            "HEIC runtime dependency is unavailable",
                        ),
                    },
                    None,
                )
            }
        };
        let dialog = |name| {
            if optional_dependencies_available {
                PlatformCapability::available(name)
            } else {
                PlatformCapability::unavailable(format!("{name} runtime dependency is unavailable"))
            }
        };
        let mut formats = BTreeMap::new();
        formats.insert(ImageFormat::Jpeg, portable());
        formats.insert(ImageFormat::Png, portable());
        formats.insert(ImageFormat::Tiff, portable());
        formats.insert(ImageFormat::Heic, heic());
        CapabilitySnapshot::new(formats, dialog("folder picker"), dialog("save picker"))
    }

    fn active_state(capabilities: CapabilitySnapshot) -> EditorState {
        let folder = AbsolutePath::new("/photos").expect("test folder is absolute");
        let png_path =
            AbsolutePath::new("/photos/full.image.name.png").expect("test path is absolute");
        let heic_path =
            AbsolutePath::new("/photos/unavailable.heic").expect("test path is absolute");
        let entries = vec![
            DirectoryEntry::new(
                png_path,
                Utf8FileName::new("full.image.name.png").expect("test filename is valid"),
                DirectoryEntryLocation::Direct,
                DirectoryEntryKind::RegularFile,
                None,
            ),
            DirectoryEntry::new(
                heic_path,
                Utf8FileName::new("unavailable.heic").expect("test filename is valid"),
                DirectoryEntryLocation::Direct,
                DirectoryEntryKind::RegularFile,
                None,
            ),
        ];
        let initial = EditorState::new(capabilities.clone());
        let enumerating = reduce(
            &initial,
            EditorCommand::BeginFolderEnumeration {
                folder: folder.clone(),
            },
        );
        let plan = plan_folder_enumeration(
            &capabilities,
            FolderEnumerationInput::Succeeded { folder, entries },
        );
        let collected = reduce(
            &enumerating.state,
            EditorCommand::FolderEnumerated {
                token: enumerating.effects[0].token(),
                result: plan,
            },
        );
        let candidate = collected.state.browsing().collection().entries()[0].clone();
        let decoding = reduce(&collected.state, EditorCommand::BeginDecode { candidate });
        reduce(
            &decoding.state,
            EditorCommand::ImageDecoded {
                token: decoding.effects[0].token(),
                image: CanonicalImage::new(2, 2, vec![Rgba16::new(1, 2, 3, u16::MAX); 4])
                    .expect("test image is valid"),
            },
        )
        .state
    }

    fn collect_text(shape: &egui::epaint::Shape, texts: &mut Vec<String>) {
        match shape {
            egui::epaint::Shape::Text(text) => texts.push(text.galley.job.text.clone()),
            egui::epaint::Shape::Vec(shapes) => {
                for shape in shapes {
                    collect_text(shape, texts);
                }
            }
            _ => {}
        }
    }

    fn workspace_text(app: &mut DesktopApp) -> Vec<String> {
        let context = egui::Context::default();
        let output = context.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1280.0, 800.0),
                )),
                ..Default::default()
            },
            |context| {
                egui::SidePanel::left("image-collection")
                    .resizable(true)
                    .default_width(220.0)
                    .show(context, |ui| app.render_collection_pane(ui));
                egui::SidePanel::right("editor-commands")
                    .resizable(true)
                    .default_width(210.0)
                    .show(context, |ui| app.render_command_pane(ui));
                egui::CentralPanel::default().show(context, |ui| app.render_preview_pane(ui));
            },
        );
        let mut texts = Vec::new();
        for shape in &output.shapes {
            collect_text(&shape.shape, &mut texts);
        }
        texts
    }

    #[test]
    fn package_startup_smoke_uses_one_workspace_with_optional_dependencies_present_or_missing() {
        for (scenario, optional_dependencies_available) in [
            ("optional dependencies present", true),
            ("optional dependencies missing", false),
        ] {
            let mut app = DesktopApp::new();
            app.state =
                EditorState::new(package_smoke_capabilities(optional_dependencies_available));

            // Rendering all three panes through one context is the headless
            // representation of the sole primary workspace created by `run`.
            let texts = workspace_text(&mut app);
            for expected in ["图像集合", "预览", "编辑命令"] {
                assert!(
                    texts.iter().any(|text| text.contains(expected)),
                    "{scenario} must start its one workspace with {expected:?}: {texts:#?}"
                );
            }
            for portable_format in ["导出 JPEG", "导出 PNG", "导出 TIFF"] {
                assert!(
                    texts.iter().any(|text| text.contains(portable_format)),
                    "{scenario} must retain portable export control {portable_format:?}"
                );
            }

            if optional_dependencies_available {
                assert!(
                    texts.iter().any(|text| text.contains("导出 HEIC")),
                    "{scenario} must enable the detected HEIC export choice"
                );
                assert!(
                    !texts.iter().any(|text| text.contains("HEIC decoding:")),
                    "{scenario} must not report an unavailable HEIC dependency"
                );
            } else {
                for expected in ["HEIC decoding:", "folder picker:", "save picker:"] {
                    assert!(
                        texts.iter().any(|text| text.contains(expected)),
                        "{scenario} must show the unavailable capability notice {expected:?}"
                    );
                }
                assert!(
                    !texts.iter().any(|text| text.contains("导出 HEIC")),
                    "{scenario} must omit HEIC export when its runtime dependency is absent"
                );
            }
        }
    }

    #[test]
    fn single_workspace_visual_regression_shows_active_image_controls_filename_and_notices() {
        let mut app = DesktopApp::new();
        app.state = active_state(capabilities());

        let texts = workspace_text(&mut app);
        let (increase, decrease, undo, redo) = if cfg!(target_os = "macos") {
            (
                "增加调整 (Option+Up)",
                "减少调整 (Option+Down)",
                "撤销 (Command+Z)",
                "重做 (Command+Shift+Z)",
            )
        } else {
            (
                "增加调整 (Alt+Up)",
                "减少调整 (Alt+Down)",
                "撤销 (Control+Z)",
                "重做 (Control+Shift+Z)",
            )
        };
        for expected in [
            "图像集合",
            "编辑命令",
            "full.image.name.png",
            "水平翻转 (F)",
            "垂直翻转 (Shift+F)",
            "顺时针旋转 (R)",
            "逆时针旋转 (Shift+R)",
            "开始裁剪 (C)",
            "确认裁剪",
            "取消裁剪",
            "调整亮度 (B)",
            "调整对比度 (D)",
            increase,
            decrease,
            "提交调整 (Return)",
            undo,
            redo,
            "导出 JPEG",
            "导出 PNG",
            "导出 TIFF",
            "unavailable.heic:",
            "folder picker:",
            "save picker:",
        ] {
            assert!(
                texts.iter().any(|text| text.contains(expected)),
                "desktop workspace snapshot is missing {expected:?}; rendered text: {texts:#?}"
            );
        }
        assert!(
            !texts.iter().any(|text| text.contains("导出 HEIC")),
            "the unavailable HEIC encoder must not be rendered as an export control"
        );
    }

    #[test]
    fn accepted_key_press_routes_one_reducer_command_once() {
        let event = egui::Event::Key {
            key: egui::Key::F,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        };
        let raw = raw_key_event(RuntimePlatform::Linux, &event, false)
            .expect("defined shortcut becomes a raw key event");
        let command = image_editor_core::resolve_shortcut(RuntimePlatform::Linux, raw)
            .expect("accepted F press resolves to a command");
        assert_eq!(command, EditorCommand::FlipHorizontal);

        let state = active_state(capabilities());
        let reduced = reduce(&state, command);
        let active = reduced
            .state
            .browsing()
            .active()
            .expect("decoded test image is active");
        let document = reduced
            .state
            .browsing()
            .document(active)
            .expect("active document exists");
        assert_eq!(document.history().len(), 1);
        assert_eq!(
            document.history()[0],
            image_editor_core::EditOperation::FlipHorizontal
        );
    }

    #[test]
    fn crop_overlay_visual_coordinates_round_trip_to_source_pixels() {
        let rect = egui::Rect::from_min_max(egui::pos2(10.0, 20.0), egui::pos2(210.0, 120.0));
        let draft = image_editor_core::CropDraft::new(20, 10, 180, 90);
        let context = egui::Context::default();
        let output = context.run(egui::RawInput::default(), |context| {
            egui::CentralPanel::default().show(context, |ui| {
                draw_crop_overlay(ui, rect, draft, 200, 100);
            });
        });
        let lines = output
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::epaint::Shape::LineSegment { points, .. } => Some(*points),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert!(lines.contains(&[egui::pos2(30.0, 30.0), egui::pos2(190.0, 30.0)]));
        assert_eq!(
            source_pixel_coordinate(egui::pos2(190.0, 110.0), rect, 200, 100),
            (180, 90)
        );
    }

    #[test]
    fn runtime_dialog_failure_becomes_an_availability_notice() {
        let mut app = DesktopApp::new();
        app.record_runtime_dialog_failure(image_editor_core::ApplicationError::PlatformOperation {
            capability: image_editor_core::CapabilityName::SavePicker,
            cause: image_editor_core::SafeError::new(
                image_editor_core::ErrorCategory::PlatformIntegration,
                "XDG Desktop Portal service disappeared",
            ),
        });

        assert_eq!(app.runtime_dialog_notices.len(), 1);
        let notice = &app.runtime_dialog_notices[0];
        assert_eq!(
            notice.severity,
            image_editor_core::NoticeSeverity::Availability
        );
        assert!(matches!(
            &notice.subject,
            image_editor_core::NoticeSubject::Capability(
                image_editor_core::CapabilityName::SavePicker
            )
        ));
        assert_eq!(
            notice.message.summary(),
            "XDG Desktop Portal service disappeared"
        );
    }

    #[test]
    fn platform_specific_shortcut_labels_match_the_rendered_button_convention() {
        assert_eq!(
            command_title(
                RuntimePlatform::MacOs,
                "增加调整",
                &EditorCommand::IncreaseAdjustment,
            ),
            "增加调整 (Option+Up)"
        );
        assert_eq!(
            command_title(RuntimePlatform::MacOs, "撤销", &EditorCommand::Undo,),
            "撤销 (Command+Z)"
        );
        assert_eq!(
            command_title(
                RuntimePlatform::Linux,
                "增加调整",
                &EditorCommand::IncreaseAdjustment,
            ),
            "增加调整 (Alt+Up)"
        );
        assert_eq!(
            command_title(RuntimePlatform::Linux, "撤销", &EditorCommand::Undo),
            "撤销 (Control+Z)"
        );
    }
}
