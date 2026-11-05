//! The main application state and panel layout, porting the Python app's
//! dock layout: image list (left), image viewer (center), tag editor
//! (right), Auto-Captioner settings (right, below).

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread;

use egui::{ColorImage, Key, TextureHandle, TextureOptions};
use tg_caption::client::{caption_image, EndpointConfig, GenerationParams};
use tg_core::filter::Filter;
use tg_core::image_entry::ImageEntry;
use tg_core::settings::Settings;
use tg_core::sidecar;
use tg_core::tags::{add_caption_to_tags, CaptionPosition};

/// A captioning result arriving from a worker thread.
pub struct CaptionResult {
    pub index: usize,
    pub caption: Option<String>,
    pub error: Option<String>,
    pub elapsed_seconds: f64,
}

/// A thumbnail decoded on a worker thread.
struct LoadedThumbnail {
    index: usize,
    image: ColorImage,
}

/// A full-size image decoded on a worker thread.
struct LoadedViewerImage {
    index: usize,
    image: ColorImage,
}

pub struct TagGuiApp {
    pub settings: Settings,
    pub images: Vec<ImageEntry>,
    pub folder: Option<PathBuf>,
    pub selected: Option<usize>,
    /// Filter text as typed; `filter_parsed` is the parsed result.
    pub filter_text: String,
    pub filter_parsed: Option<Filter>,
    /// Tag text of the selected image as shown in the editor.
    pub tag_edit_text: String,
    /// Indices (into `images`) that pass the filter, in display order.
    pub visible_indices: Vec<usize>,
    /// Undo/redo stacks of full tag states (like the Python app's
    /// `undo_stack`/`redo_stack`).
    pub undo_stack: VecDeque<Vec<Vec<String>>>,
    pub redo_stack: Vec<Vec<Vec<String>>>,
    // Texture caches, keyed by image index.
    pub thumbnails: std::collections::HashMap<usize, TextureHandle>,
    pub viewer_image: Option<(usize, TextureHandle)>,
    // Background loading channels.
    thumbnail_sender: Option<mpsc::Sender<LoadedThumbnail>>,
    viewer_sender: Option<mpsc::Sender<LoadedViewerImage>>,
    pending_thumbnails: mpsc::Receiver<LoadedThumbnail>,
    pending_viewer: mpsc::Receiver<LoadedViewerImage>,
    viewer_requests: Arc<Mutex<VecDeque<(usize, PathBuf)>>>,
    thumbnail_requests: Arc<Mutex<VecDeque<(usize, PathBuf)>>>,
    // Viewer transform.
    pub zoom: f32,
    pub pan: egui::Vec2,
    // Captioning run state.
    pub api_base_url: String,
    pub api_key: String,
    pub api_model_name: String,
    pub caption_prompt: String,
    pub batch_size: usize,
    pub max_tokens: u32,
    pub do_sample: bool,
    pub temperature: f64,
    pub test_run_count: usize,
    pub caption_position: CaptionPosition,
    caption_results: mpsc::Receiver<CaptionResult>,
    caption_result_sender: Option<mpsc::Sender<CaptionResult>>,
    pub caption_cancel: Arc<AtomicBool>,
    pub caption_total: Arc<AtomicUsize>,
    pub caption_running: bool,
    pub caption_started_at: Option<std::time::Instant>,
    pub caption_failures: Vec<(String, String)>,
    pub caption_done_count: usize,
    pub caption_target_count: usize,
    pub log: VecDeque<String>,
}

impl TagGuiApp {
    pub fn new(settings: Settings) -> Self {
        let (thumbnail_tx, thumbnail_rx) = mpsc::channel();
        let (viewer_tx, viewer_rx) = mpsc::channel();
        let (_caption_tx, caption_rx) = mpsc::channel();
        Self {
            settings,
            images: Vec::new(),
            folder: None,
            selected: None,
            filter_text: String::new(),
            filter_parsed: None,
            tag_edit_text: String::new(),
            visible_indices: Vec::new(),
            undo_stack: VecDeque::new(),
            redo_stack: Vec::new(),
            thumbnails: std::collections::HashMap::new(),
            viewer_image: None,
            thumbnail_sender: Some(thumbnail_tx),
            viewer_sender: Some(viewer_tx),
            pending_thumbnails: thumbnail_rx,
            pending_viewer: viewer_rx,
            viewer_requests: Arc::new(Mutex::new(VecDeque::new())),
            thumbnail_requests: Arc::new(Mutex::new(VecDeque::new())),
            zoom: 1.0,
            pan: egui::Vec2::ZERO,
            api_base_url: String::from("http://127.0.0.1:8000/v1"),
            api_key: String::new(),
            api_model_name: String::new(),
            caption_prompt: String::from("Describe this image in detail."),
            batch_size: 4,
            max_tokens: 300,
            do_sample: false,
            temperature: 0.7,
            test_run_count: 0,
            caption_position: CaptionPosition::AfterLastTag,
            caption_results: caption_rx,
            caption_result_sender: None,
            caption_cancel: Arc::new(AtomicBool::new(false)),
            caption_total: Arc::new(AtomicUsize::new(0)),
            caption_running: false,
            caption_started_at: None,
            caption_failures: Vec::new(),
            caption_done_count: 0,
            caption_target_count: 0,
            log: VecDeque::new(),
        }
    }

    pub fn log(&mut self, message: impl Into<String>) {
        self.log.push_back(message.into());
        while self.log.len() > 500 {
            self.log.pop_front();
        }
    }

    /// Load a folder's images and sidecars (`ImageListModel.load_directory`).
    pub fn open_folder(&mut self, folder: PathBuf) {
        match tg_core::scan_directory(
            &folder,
            &self.settings.image_list_file_formats,
            &self.settings.effective_tag_separator(),
        ) {
            Ok(images) => {
                let count = images.len();
                self.images = images;
                self.folder = Some(folder.clone());
                self.thumbnails.clear();
                self.viewer_image = None;
                self.selected = None;
                self.undo_stack.clear();
                self.redo_stack.clear();
                self.apply_filter();
                self.log(format!("Opened {} images in {}", count, folder.display()));
                // Queue thumbnails for decoding, oldest selections first.
                let requests: Vec<(usize, PathBuf)> = (0..self.images.len())
                    .map(|index| (index, self.images[index].path.clone()))
                    .collect();
                *self.thumbnail_requests.lock().unwrap() = requests.into_iter().collect();
            }
            Err(error) => self.log(format!("Failed to open folder: {error}")),
        }
    }

    /// Recompute `visible_indices` from the filter text.
    pub fn apply_filter(&mut self) {
        self.filter_parsed = Filter::parse(&self.filter_text);
        let separator = self.settings.effective_tag_separator();
        self.visible_indices = (0..self.images.len())
            .filter(|&index| match &self.filter_parsed {
                Some(filter) => {
                    let image = &self.images[index];
                    let input = tg_core::filter::FilterInput {
                        path: &image.path,
                        dimensions: image.dimensions,
                        file_size: image.file_size,
                        tags: &image.tags,
                        tag_separator: &separator,
                        token_counter: None,
                    };
                    filter.matches(&input)
                }
                None => true,
            })
            .collect();
        // Keep the selection valid.
        if let Some(selected) = self.selected {
            if !self.visible_indices.contains(&selected) {
                let nearest = self
                    .visible_indices
                    .iter()
                    .copied()
                    .min_by_key(|&index| index.abs_diff(selected));
                self.selected = nearest;
            }
        } else if let Some(&first) = self.visible_indices.first() {
            self.selected = Some(first);
        }
    }

    pub fn select(&mut self, index: usize) {
        if self.selected == Some(index) {
            return;
        }
        // Save any pending edit for the outgoing selection first.
        self.commit_tag_edit();
        self.selected = Some(index);
        self.tag_edit_text = self.images[index]
            .tags
            .join(&self.settings.effective_tag_separator());
        self.viewer_image = None;
        self.zoom = 1.0;
        self.pan = egui::Vec2::ZERO;
        self.viewer_requests
            .lock()
            .unwrap()
            .push_back((index, self.images[index].path.clone()));
    }

    /// Move the selection within the filtered list (keyboard navigation).
    pub fn move_selection(&mut self, offset: isize) {
        let Some(selected) = self.selected else {
            return;
        };
        let position = self
            .visible_indices
            .iter()
            .position(|&index| index == selected);
        let Some(position) = position else {
            return;
        };
        let new_position =
            (position as isize + offset).clamp(0, self.visible_indices.len() as isize - 1) as usize;
        if let Some(&new_index) = self.visible_indices.get(new_position) {
            self.select(new_index);
        }
    }

    /// Push the current tag state onto the undo stack.
    pub fn push_undo(&mut self) {
        let state: Vec<Vec<String>> = self.images.iter().map(|image| image.tags.clone()).collect();
        self.undo_stack.push_back(state);
        if self.undo_stack.len() > 50 {
            self.undo_stack.pop_front();
        }
        self.redo_stack.clear();
    }

    pub fn undo(&mut self) {
        let Some(state) = self.undo_stack.pop_back() else {
            return;
        };
        let current: Vec<Vec<String>> =
            self.images.iter().map(|image| image.tags.clone()).collect();
        self.redo_stack.push(current);
        self.restore_tags(state);
    }

    pub fn redo(&mut self) {
        let Some(state) = self.redo_stack.pop() else {
            return;
        };
        let current: Vec<Vec<String>> =
            self.images.iter().map(|image| image.tags.clone()).collect();
        self.undo_stack.push_back(current);
        self.restore_tags(state);
    }

    fn restore_tags(&mut self, state: Vec<Vec<String>>) {
        for (image, tags) in self.images.iter_mut().zip(state) {
            if image.tags != tags {
                let separator = self.settings.effective_tag_separator();
                let _ = sidecar::write_tags(&image.path, &tags, &separator);
                image.tags = tags;
            }
        }
        if let Some(selected) = self.selected {
            self.tag_edit_text = self.images[selected]
                .tags
                .join(&self.settings.effective_tag_separator());
        }
        self.apply_filter();
    }

    /// Parse the tag editor's text and persist it (called on image switch
    /// and on every edit, like the Python app writes on change).
    pub fn commit_tag_edit(&mut self) {
        let Some(selected) = self.selected else {
            return;
        };
        let separator = self.settings.effective_tag_separator();
        let tags = sidecar::parse_tags(&self.tag_edit_text, &separator);
        if tags == self.images[selected].tags {
            return;
        }
        match sidecar::write_tags(&self.images[selected].path, &tags, &separator) {
            Ok(()) => {
                self.images[selected].tags = tags;
                self.apply_filter();
            }
            Err(error) => self.log(format!("Failed to save tags: {error}")),
        }
    }

    /// Apply a generated caption to the selected image
    /// (`add_caption_to_tags` semantics) and persist.
    pub fn apply_caption(&mut self, index: usize, caption: &str) {
        let separator = self.settings.effective_tag_separator();
        self.push_undo();
        let tags = add_caption_to_tags(
            &self.images[index].tags,
            caption,
            self.caption_position,
            &separator,
        );
        match sidecar::write_tags(&self.images[index].path, &tags, &separator) {
            Ok(()) => {
                self.images[index].tags = tags;
                if self.selected == Some(index) {
                    self.tag_edit_text = self.images[index].tags.join(&separator);
                }
                self.apply_filter();
            }
            Err(error) => self.log(format!("Failed to save tags: {error}")),
        }
    }

    /// Start an endpoint captioning run over the currently visible images.
    pub fn start_captioning(&mut self) {
        if self.caption_running {
            return;
        }
        let config = EndpointConfig {
            base_url: self.api_base_url.clone(),
            api_key: Some(self.api_key.clone()),
            model_name: Some(self.api_model_name.clone()),
        };
        // Fail fast like the Python app: resolve the server-side model
        // name before spawning anything.
        let model = match config.resolve_model_name() {
            Ok(model) => model,
            Err(error) => {
                self.log(format!("Cannot start captioning: {error}"));
                return;
            }
        };
        self.log(format!(
            "Captioning with {model} on {} images",
            self.visible_indices.len()
        ));
        let (sender, _receiver) = mpsc::channel();
        // Keep a clone alive in the app (dropped when the run ends); the
        // workers get their own clones.
        self.caption_result_sender = Some(sender.clone());
        self.caption_cancel = Arc::new(AtomicBool::new(false));
        self.caption_total = Arc::new(AtomicUsize::new(0));
        self.caption_running = true;
        self.caption_started_at = Some(std::time::Instant::now());
        self.caption_failures.clear();
        self.caption_done_count = 0;
        let mut targets = self.visible_indices.clone();
        if self.test_run_count > 0 {
            targets.truncate(self.test_run_count);
            self.log(format!(
                "Test run: captioning the first {} images",
                targets.len()
            ));
        }
        self.caption_target_count = targets.len();
        let items: Vec<(usize, PathBuf)> = targets
            .into_iter()
            .map(|index| (index, self.images[index].path.clone()))
            .collect();
        let queue = Arc::new(Mutex::new(VecDeque::from(items)));
        let params = Arc::new(GenerationParams {
            max_tokens: self.max_tokens,
            do_sample: self.do_sample,
            temperature: self.temperature,
            top_k: 50,
            top_p: 1.0,
            repetition_penalty: 1.0,
        });
        let cancel = self.caption_cancel.clone();
        let total = self.caption_total.clone();
        let prompt = self.caption_prompt.clone();
        let batch_size = self.batch_size.clamp(1, 64);
        for _ in 0..batch_size {
            let queue = queue.clone();
            let cancel = cancel.clone();
            let total = total.clone();
            let sender = sender.clone();
            let config = config.clone();
            let params = params.clone();
            let prompt = prompt.clone();
            thread::spawn(move || loop {
                if cancel.load(Ordering::Relaxed) {
                    return;
                }
                let Some((index, path)) = queue.lock().unwrap().pop_front() else {
                    return;
                };
                let started = std::time::Instant::now();
                let result = caption_image(&config, &path, &prompt, &params);
                let _ = total.fetch_add(1, Ordering::Relaxed);
                let (caption, error) = match result {
                    Ok(caption) => (Some(caption), None),
                    Err(error) => (None, Some(error.to_string())),
                };
                let _ = sender.send(CaptionResult {
                    index,
                    caption,
                    error,
                    elapsed_seconds: started.elapsed().as_secs_f64(),
                });
            });
        }
    }

    pub fn stop_captioning(&mut self) {
        if self.caption_running {
            self.caption_cancel.store(true, Ordering::Relaxed);
            self.log("Canceling captioning...");
        }
    }

    /// Poll background channels once per frame.
    pub fn poll_background(&mut self, ctx: &egui::Context) {
        // Thumbnails.
        while let Ok(loaded) = self.pending_thumbnails.try_recv() {
            let handle = ctx.load_texture(
                format!("thumb{}", loaded.index),
                loaded.image,
                TextureOptions::default(),
            );
            self.thumbnails.insert(loaded.index, handle);
        }
        // Full viewer image.
        while let Ok(loaded) = self.pending_viewer.try_recv() {
            let handle = ctx.load_texture(
                format!("view{}", loaded.index),
                loaded.image,
                TextureOptions::default(),
            );
            self.viewer_image = Some((loaded.index, handle));
        }
        // Caption results.
        let mut results = Vec::new();
        while let Ok(result) = self.caption_results.try_recv() {
            results.push(result);
        }
        for result in results {
            self.caption_done_count += 1;
            match (result.caption, result.error) {
                (Some(caption), _) => {
                    let name = self.images[result.index].path.display().to_string();
                    let elapsed = result.elapsed_seconds;
                    self.log(format!(
                        "{} ({elapsed:.1} s): {caption}",
                        self.images[result.index]
                            .path
                            .file_name()
                            .unwrap_or_default()
                            .to_string_lossy()
                    ));
                    self.apply_caption(result.index, &caption);
                    let _ = name;
                }
                (None, Some(error)) => {
                    self.caption_failures.push((
                        self.images[result.index]
                            .path
                            .file_name()
                            .unwrap_or_default()
                            .to_string_lossy()
                            .into_owned(),
                        error,
                    ));
                }
                (None, None) => {}
            }
        }
        if self.caption_running && self.caption_done_count >= self.caption_target_count {
            self.caption_running = false;
            let elapsed = self
                .caption_started_at
                .map(|started| started.elapsed().as_secs_f64())
                .unwrap_or(0.0);
            let per_image = if self.caption_done_count > 0 {
                elapsed / self.caption_done_count as f64
            } else {
                0.0
            };
            self.log(format!(
                "Finished captioning {} images in {elapsed:.1} s \
                ({per_image:.1} s/image); {} failed",
                self.caption_done_count,
                self.caption_failures.len()
            ));
        }
        // Kick background decode jobs.
        self.pump_decode_queues();
    }

    fn pump_decode_queues(&mut self) {
        let thumbnail_sender = self.thumbnail_sender.clone();
        let thumbnail_requests = self.thumbnail_requests.clone();
        thread::spawn(move || {
            while let Some((index, path)) = thumbnail_requests.lock().unwrap().pop_front() {
                if let Ok(image) = image::open(&path) {
                    let rgba = image.thumbnail(200, 200).to_rgba8();
                    let color_image = ColorImage::from_rgba_unmultiplied(
                        [rgba.width() as usize, rgba.height() as usize],
                        rgba.as_raw(),
                    );
                    let Some(sender) = thumbnail_sender.as_ref() else {
                        return; // app closed
                    };
                    if sender
                        .send(LoadedThumbnail {
                            index,
                            image: color_image,
                        })
                        .is_err()
                    {
                        return; // app closed
                    }
                }
            }
        });
        let viewer_sender = self.viewer_sender.clone();
        let viewer_requests = self.viewer_requests.clone();
        thread::spawn(move || {
            while let Some((index, path)) = viewer_requests.lock().unwrap().pop_front() {
                if let Ok(image) = image::open(&path) {
                    let rgba = image.to_rgba8();
                    let color_image = ColorImage::from_rgba_unmultiplied(
                        [rgba.width() as usize, rgba.height() as usize],
                        rgba.as_raw(),
                    );
                    let Some(sender) = viewer_sender.as_ref() else {
                        return; // app closed
                    };
                    if sender
                        .send(LoadedViewerImage {
                            index,
                            image: color_image,
                        })
                        .is_err()
                    {
                        return; // app closed
                    }
                }
            }
        });
    }

    /// Global keyboard shortcuts, mirroring the Python app's core flow.
    pub fn handle_shortcuts(&mut self, ctx: &egui::Context) {
        ctx.input(|input| {
            if input.key_pressed(Key::ArrowDown) && !input.modifiers.ctrl {
                self.move_selection(1);
            }
            if input.key_pressed(Key::ArrowUp) && !input.modifiers.ctrl {
                self.move_selection(-1);
            }
            if input.modifiers.ctrl && input.key_pressed(Key::Z) {
                if input.modifiers.shift {
                    self.redo();
                } else {
                    self.undo();
                }
            }
            if input.modifiers.ctrl && input.key_pressed(Key::Y) {
                self.redo();
            }
        });
    }
}

impl eframe::App for TagGuiApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.handle_shortcuts(ctx);
        self.poll_background(ctx);
        if self.caption_running {
            // Keep repainting while a run is in flight.
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
        super::panels::top_bar(ctx, self);
        super::panels::image_list(ctx, self);
        super::panels::viewer(ctx, self);
        super::panels::tag_editor(ctx, self);
        super::panels::captioner(ctx, self);
        super::panels::log_panel(ctx, self);
    }

    fn on_exit(&mut self, _glow: Option<&eframe::glow::Context>) {
        // Persist settings.
        let _ = self
            .settings
            .save(&tg_core::settings::default_config_path());
    }
}
