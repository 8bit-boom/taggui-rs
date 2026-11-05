//! The main application state and panel layout, porting the Python app's
//! dock layout: image list (left), image viewer (center), tag editor
//! (right), Auto-Captioner settings (right, below).

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread;

use egui::{ColorImage, Key, TextureHandle, TextureOptions};
use tg_caption::client::{caption_image, EndpointConfig, GenerationParams};
use tg_caption::hf;
use tg_caption::local::{self, LocalModelConfig, LocalProgress};
use tg_caption::wd_tagger::{WdTagger, WdTaggerError, WdTaggerSettings};
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptionBackend {
    RemoteApi,
    LocalGguf,
    WdTagger,
}

impl CaptionBackend {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::RemoteApi => "Remote API (vLLM / SGLang)",
            Self::LocalGguf => "Local GGUF model",
            Self::WdTagger => "WD Tagger (ONNX)",
        }
    }
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
    pub backend: CaptionBackend,
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
    // Local GGUF settings.
    pub local_model_path: String,
    pub local_mmproj_path: String,
    pub local_gpu_layers: i32,
    // WD tagger settings.
    pub wd_model_path: String,
    pub wd_min_probability: f32,
    pub wd_max_tags: usize,
    pub wd_tags_to_exclude: String,
    // Model manager downloads.
    pub download_repo: String,
    pub download_files: String,
    pub download_active: bool,
    pub download_progress: Vec<String>,
    pub extras: crate::extras::ExtrasState,
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
            backend: CaptionBackend::RemoteApi,
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
            local_model_path: String::new(),
            local_mmproj_path: String::new(),
            local_gpu_layers: 0,
            wd_model_path: String::new(),
            wd_min_probability: 0.35,
            wd_max_tags: 30,
            wd_tags_to_exclude: String::new(),
            download_repo: String::new(),
            download_files: String::new(),
            download_active: false,
            download_progress: Vec::new(),
            extras: crate::extras::ExtrasState::default(),
        }
    }

    /// Refresh the tag editor text from the selected image (after batch
    /// operations changed tags behind the editor).
    pub fn sync_tag_edit(&mut self) {
        if let Some(selected) = self.selected {
            self.tag_edit_text = self.images[selected]
                .tags
                .join(&self.settings.effective_tag_separator());
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

    /// Launch a background download of the Model Manager's repo/files.
    /// The worker locks the shared slot only briefly per progress update;
    /// the UI copies the slot every frame in `poll_background`.
    pub fn start_download(&mut self) {
        if self.download_active {
            return;
        }
        let repo = self.download_repo.trim().to_owned();
        let files: Vec<String> = self
            .download_files
            .split(',')
            .map(|file| file.trim().to_owned())
            .filter(|file| !file.is_empty())
            .collect();
        if repo.is_empty() || files.is_empty() {
            self.log("Enter a repository id and at least one file name.");
            return;
        }
        let models_dir = dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("taggui-rs")
            .join("models");
        self.download_active = true;
        self.download_progress.clear();
        let slot = download_progress_slot();
        let slot_for_worker = Arc::clone(&slot);
        let done = Arc::new(AtomicBool::new(false));
        download_done_flags()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(Arc::clone(&done));
        thread::spawn(move || {
            let slot = slot_for_worker;
            for file in &files {
                {
                    let mut slot_lock =
                        slot.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
                    slot_lock.push(format!("{file}: downloading..."));
                }
                let file_name = file.clone();
                let result = hf::download_file(
                    &repo,
                    file,
                    None,
                    &models_dir,
                    &mut |done_bytes, total| {
                        let mut slot_lock =
                            slot.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
                        if let Some(line) = slot_lock.last_mut() {
                            if total > 0 {
                                *line = format!(
                                    "{file_name}: {done_bytes} / {total}                                      bytes");
                            } else {
                                *line = format!("{file_name}: {done_bytes} bytes");
                            }
                        }
                    },
                );
                let mut slot_lock = slot.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
                match result {
                    Ok(path) => {
                        if let Some(line) = slot_lock.last_mut() {
                            *line = format!("{file}: saved to {}", path.display());
                        }
                    }
                    Err(error) => {
                        if let Some(line) = slot_lock.last_mut() {
                            *line = format!("{file}: FAILED - {error}");
                        }
                    }
                }
            }
            done.store(true, Ordering::Relaxed);
        });
        self.download_progress = slot
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
    }

    /// Start a captioning run with the selected backend over the visible
    /// images.
    pub fn start_captioning(&mut self) {
        if self.caption_running {
            return;
        }
        match self.backend {
            CaptionBackend::RemoteApi => self.start_remote_captioning(),
            CaptionBackend::LocalGguf => self.start_local_captioning(),
            CaptionBackend::WdTagger => self.start_wd_captioning(),
        }
    }

    /// Start a WD tagger run (local ONNX, sequential).
    fn start_wd_captioning(&mut self) {
        let model_path = self.wd_model_path.trim().to_owned();
        if model_path.is_empty() || !Path::new(&model_path).is_file() {
            self.log("Set a valid WD tagger model.onnx path first                 (download one from the Model Manager).");
            return;
        }
        let settings = WdTaggerSettings {
            min_probability: self.wd_min_probability,
            max_tags: self.wd_max_tags,
            tags_to_exclude: self
                .wd_tags_to_exclude
                .split(',')
                .map(|tag| tag.trim().replace('_', " "))
                .filter(|tag| !tag.is_empty())
                .collect(),
        };
        let separator = self.settings.effective_tag_separator();
        let targets: Vec<(usize, PathBuf)> = self
            .visible_indices
            .iter()
            .map(|&index| (index, self.images[index].path.clone()))
            .collect();
        self.caption_cancel = Arc::new(AtomicBool::new(false));
        self.caption_running = true;
        self.caption_started_at = Some(std::time::Instant::now());
        self.caption_failures.clear();
        self.caption_done_count = 0;
        self.caption_target_count = targets.len();
        let cancel = self.caption_cancel.clone();
        let caption_sender = self.caption_result_sender.clone();
        let position = self.caption_position;
        thread::spawn(move || {
            let mut tagger = match WdTagger::load(Path::new(&model_path)) {
                Ok(tagger) => tagger,
                Err(error) => {
                    if let Some(sender) = caption_sender {
                        let _ = sender.send(CaptionResult {
                            index: 0,
                            caption: None,
                            error: Some(error.to_string()),
                            elapsed_seconds: 0.0,
                        });
                    }
                    return;
                }
            };
            for (index, path) in targets {
                if cancel.load(Ordering::Relaxed) {
                    break;
                }
                let started = std::time::Instant::now();
                let (caption, error) = match tagger.generate_tags(&path, &settings) {
                    Ok(tags) => {
                        let caption = tags
                            .iter()
                            .map(|(tag, _)| tag.as_str())
                            .collect::<Vec<&str>>()
                            .join(&separator);
                        (Some(caption), None)
                    }
                    Err(error @ WdTaggerError::Image { .. }) => (None, Some(error.to_string())),
                    Err(error) => (None, Some(error.to_string())),
                };
                let _ = position;
                if let Some(sender) = &caption_sender {
                    let _ = sender.send(CaptionResult {
                        index,
                        caption,
                        error,
                        elapsed_seconds: started.elapsed().as_secs_f64(),
                    });
                }
            }
        });
    }

    /// Start a local GGUF captioning run (sequential on one thread).
    fn start_local_captioning(&mut self) {
        let model_path = self.local_model_path.trim().to_owned();
        if model_path.is_empty() || !Path::new(&model_path).is_file() {
            self.log("Set a valid GGUF model path first (download one via                 the Model Manager).");
            return;
        }
        let mmproj = self.local_mmproj_path.trim().to_owned();
        let config = LocalModelConfig {
            model_path: PathBuf::from(&model_path),
            mmproj_path: if mmproj.is_empty() {
                None
            } else {
                Some(PathBuf::from(mmproj))
            },
            n_gpu_layers: self.local_gpu_layers,
            n_ctx: 4096,
            n_threads: 0,
        };
        let params = self.generation_params();
        let prompt = self.caption_prompt.clone();
        let separator_for_results = self.settings.effective_tag_separator();
        let _ = separator_for_results;
        let targets: Vec<(usize, PathBuf)> = self
            .visible_indices
            .iter()
            .map(|&index| (index, self.images[index].path.clone()))
            .collect();
        self.caption_cancel = Arc::new(AtomicBool::new(false));
        self.caption_running = true;
        self.caption_started_at = Some(std::time::Instant::now());
        self.caption_failures.clear();
        self.caption_done_count = 0;
        self.caption_target_count = targets.len();
        let cancel = self.caption_cancel.clone();
        let caption_sender = self.caption_result_sender.clone();
        thread::spawn(move || {
            local::run_local_captioning(
                targets,
                &config,
                &prompt,
                &params,
                &cancel,
                |progress: LocalProgress| {
                    if let Some(sender) = &caption_sender {
                        let _ = sender.send(CaptionResult {
                            index: progress.index,
                            caption: progress.caption,
                            error: progress.error,
                            elapsed_seconds: progress.elapsed_seconds,
                        });
                    }
                },
            );
        });
    }

    fn generation_params(&self) -> GenerationParams {
        GenerationParams {
            max_tokens: self.max_tokens,
            do_sample: self.do_sample,
            temperature: self.temperature,
            top_k: 50,
            top_p: 1.0,
            repetition_penalty: 1.0,
        }
    }

    /// Start a remote endpoint run (concurrent requests).
    fn start_remote_captioning(&mut self) {
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
        // Model download progress.
        if self.download_active {
            self.download_progress = download_progress_slot()
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .clone();
            let all_done = download_done_flags()
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .iter()
                .all(|flag| flag.load(Ordering::Relaxed));
            if all_done {
                self.download_active = false;
            }
            ctx.request_repaint_after(std::time::Duration::from_millis(200));
        }
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
                    // EXIF orientation (the Python app rotates per the tag).
                    let orientation = crate::extras::exif_orientation(&path);
                    let mut rgba = image.to_rgba8();
                    crate::extras::apply_orientation(&mut rgba, orientation);
                    let rgba = image::imageops::thumbnail(&rgba, 200, 200);
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
                    let orientation = crate::extras::exif_orientation(&path);
                    let mut rgba = image.to_rgba8();
                    crate::extras::apply_orientation(&mut rgba, orientation);
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

/// Shared slot the download worker writes progress lines into; the UI
/// copies it every frame.
static DOWNLOAD_PROGRESS: std::sync::OnceLock<Arc<Mutex<Vec<String>>>> = std::sync::OnceLock::new();

fn download_progress_slot() -> Arc<Mutex<Vec<String>>> {
    DOWNLOAD_PROGRESS
        .get_or_init(|| Arc::new(Mutex::new(Vec::new())))
        .clone()
}

/// Completion flags for each download run.
static DOWNLOAD_DONE: std::sync::OnceLock<Mutex<Vec<Arc<AtomicBool>>>> = std::sync::OnceLock::new();

fn download_done_flags() -> &'static Mutex<Vec<Arc<AtomicBool>>> {
    DOWNLOAD_DONE.get_or_init(|| Mutex::new(Vec::new()))
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
        let mut extras = std::mem::take(&mut self.extras);
        super::extras::saved_filters_ui(ctx, self, &mut extras);
        super::panels::image_list(ctx, self);
        super::panels::viewer(ctx, self);
        super::panels::tag_editor(ctx, self);
        super::panels::captioner(ctx, self);
        super::panels::log_panel(ctx, self);
        // The extras panel owns the bottom slot; it borrows the app's
        // extras state separately from the app borrow itself.
        let mut extras = std::mem::take(&mut self.extras);
        super::extras::extras_panel(ctx, self, &mut extras);
        self.extras = extras;
    }

    fn on_exit(&mut self, _glow: Option<&eframe::glow::Context>) {
        // Persist settings.
        let _ = self
            .settings
            .save(&tg_core::settings::default_config_path());
    }
}
