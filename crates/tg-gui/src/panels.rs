//! Panel layout, mirroring the Python app's docks: image list (left),
//! image viewer (center), tag editor + Auto-Captioner (right).

use egui::{Color32, Vec2};

use crate::app::TagGuiApp;

/// Top bar: folder opening, the filter input, and counters.
pub fn top_bar(ctx: &egui::Context, app: &mut TagGuiApp) {
    egui::TopBottomPanel::top("top_bar").show(ctx, |ui| {
        ui.horizontal(|ui| {
            if ui.button("Open Folder...").clicked() {
                if let Some(folder) = rfd::FileDialog::new().pick_folder() {
                    app.open_folder(folder);
                }
            }
            let filter_response = ui.add(
                egui::TextEdit::singleline(&mut app.filter_text)
                    .hint_text("Filter images (e.g. tags:>3 AND ext:png)")
                    .desired_width(280.0),
            );
            if filter_response.changed() {
                app.apply_filter();
            }
            if !app.filter_text.is_empty() && ui.small_button("Clear").clicked() {
                app.filter_text.clear();
                app.apply_filter();
            }
            ui.separator();
            let total = app.images.len();
            let visible = app.visible_indices.len();
            ui.label(format!("{visible} / {total} images"));
            if let Some(folder) = &app.folder {
                ui.label(folder.display().to_string());
            }
        });
    });
}

/// Left panel: the image list with thumbnails, like the Python app's
/// image list dock.
pub fn image_list(ctx: &egui::Context, app: &mut TagGuiApp) {
    egui::SidePanel::left("image_list")
        .resizable(true)
        .default_width(230.0)
        .show(ctx, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                let visible = app.visible_indices.clone();
                for index in visible {
                    let name = app.images[index]
                        .path
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .into_owned();
                    let is_selected = app.selected == Some(index);
                    let row = ui.horizontal(|ui| {
                        if let Some(handle) = app.thumbnails.get(&index) {
                            let size = handle.size_vec2();
                            let scale = if size.x > 0.0 {
                                (160.0 / size.x).min(1.0)
                            } else {
                                1.0
                            };
                            ui.add(
                                egui::Image::new(handle)
                                    .fit_to_exact_size(Vec2::new(size.x * scale, size.y * scale)),
                            );
                        } else {
                            let (rect, _) =
                                ui.allocate_exact_size(Vec2::splat(48.0), egui::Sense::hover());
                            ui.painter().rect_filled(rect, 2.0, Color32::from_gray(40));
                        }
                        ui.add(egui::Label::new(name).wrap());
                    });
                    let response = ui.allocate_rect(row.response.rect, egui::Sense::click());
                    if response.clicked() {
                        app.select(index);
                    }
                    if is_selected {
                        ui.painter().rect_stroke(
                            row.response.rect,
                            2.0,
                            egui::Stroke::new(2.0_f32, Color32::from_rgb(0, 120, 215)),
                            egui::StrokeKind::Inside,
                        );
                    }
                }
            });
        });
}

/// Center panel: the image viewer with scroll-zoom and drag-pan.
pub fn viewer(ctx: &egui::Context, app: &mut TagGuiApp) {
    egui::CentralPanel::default().show(ctx, |ui| {
        let Some(selected) = app.selected else {
            ui.centered_and_justified(|ui| {
                ui.label("Open a folder to start");
            });
            return;
        };
        let available = ui.available_size();
        let Some((loaded_index, handle)) = &app.viewer_image else {
            ui.centered_and_justified(|ui| {
                ui.spinner();
                ui.label("Loading image...");
            });
            return;
        };
        if *loaded_index != selected {
            ui.centered_and_justified(|ui| {
                ui.spinner();
                ui.label("Loading image...");
            });
            return;
        }
        let image_size = handle.size_vec2();
        let fit_scale = (available.x / image_size.x.max(1.0))
            .min(available.y / image_size.y.max(1.0))
            .min(1.0);
        let scale = fit_scale * app.zoom;
        let target_size = Vec2::new(image_size.x * scale, image_size.y * scale);
        let (rect, response) =
            ui.allocate_exact_size(ui.available_size(), egui::Sense::click_and_drag());
        // Zoom with the scroll wheel.
        let scroll = ctx.input(|input| input.raw_scroll_delta.y);
        if response.hovered() && scroll != 0.0 {
            app.zoom = (app.zoom * (1.0 + scroll / 600.0)).clamp(0.05, 20.0);
        }
        // Pan with dragging.
        if response.dragged() {
            app.pan += response.drag_delta();
        }
        let center = rect.center() + app.pan;
        let draw_rect = egui::Rect::from_center_size(center, target_size);
        let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
        ui.painter()
            .image(handle.id(), draw_rect, uv, Color32::WHITE);
    });
}

/// Right panel: the tag editor for the selected image.
pub fn tag_editor(ctx: &egui::Context, app: &mut TagGuiApp) {
    egui::SidePanel::right("tag_editor")
        .resizable(true)
        .default_width(340.0)
        .show(ctx, |ui| {
            ui.heading("Tags");
            if app.selected.is_none() {
                ui.label("No image selected.");
                return;
            }
            // Commit on change (the Python app writes on every change);
            // the parse handles separator splitting and trimming.
            let response =
                ui.add(egui::TextEdit::multiline(&mut app.tag_edit_text).desired_rows(6));
            if response.changed() {
                app.commit_tag_edit();
            }
            ui.separator();
            let tag_count = app
                .selected
                .and_then(|index| app.images.get(index))
                .map(|image| image.tags.len())
                .unwrap_or(0);
            ui.label(format!(
                "{tag_count} tags - saved to the .txt sidecar \
                on every change. Up/Down switches images, Ctrl+Z undoes."
            ));
        });
}

/// Right panel (below tags): the Auto-Captioner, ported from the Python
/// dock's endpoint mode.
pub fn captioner(ctx: &egui::Context, app: &mut TagGuiApp) {
    egui::SidePanel::right("auto_captioner")
        .resizable(true)
        .default_width(340.0)
        .show(ctx, |ui| {
            ui.heading("Auto-Captioner (remote API)");
            egui::Grid::new("captioner_settings").show(ui, |ui| {
                ui.label("Server URL");
                ui.add(egui::TextEdit::singleline(&mut app.api_base_url));
                ui.end_row();
                ui.label("API key");
                ui.add(egui::TextEdit::singleline(&mut app.api_key).password(true));
                ui.end_row();
                ui.label("Model (blank = detect)");
                ui.add(egui::TextEdit::singleline(&mut app.api_model_name));
                ui.end_row();
                ui.label("Prompt");
                ui.add(egui::TextEdit::multiline(&mut app.caption_prompt).desired_rows(3));
                ui.end_row();
                ui.label("Batch size");
                ui.add(egui::DragValue::new(&mut app.batch_size).range(1..=64));
                ui.end_row();
                ui.label("Max tokens");
                ui.add(egui::DragValue::new(&mut app.max_tokens).range(1..=2000));
                ui.end_row();
                ui.label("Sampling");
                ui.checkbox(&mut app.do_sample, "");
                ui.end_row();
                ui.label("Temperature");
                ui.add(
                    egui::DragValue::new(&mut app.temperature)
                        .range(0.0..=2.0)
                        .speed(0.01),
                );
                ui.end_row();
                ui.label("Test run (0 = off)");
                ui.add(egui::DragValue::new(&mut app.test_run_count).range(0..=1000));
                ui.end_row();
                ui.label("Caption position");
                egui::ComboBox::from_id_salt("caption_position")
                    .selected_text(app.caption_position.as_str())
                    .show_ui(ui, |ui| {
                        for position in [
                            tg_core::tags::CaptionPosition::AfterLastTag,
                            tg_core::tags::CaptionPosition::BeforeFirstTag,
                            tg_core::tags::CaptionPosition::OverwriteFirstTag,
                            tg_core::tags::CaptionPosition::OverwriteAllTags,
                            tg_core::tags::CaptionPosition::DoNotAdd,
                        ] {
                            ui.selectable_value(
                                &mut app.caption_position,
                                position,
                                position.as_str(),
                            );
                        }
                    });
                ui.end_row();
            });
            ui.horizontal(|ui| {
                if app.caption_running {
                    if ui.button("Stop Captioning").clicked() {
                        app.stop_captioning();
                    }
                } else if ui.button("Start Captioning").clicked() {
                    app.start_captioning();
                }
                if app.caption_running {
                    let done = app.caption_done_count;
                    let total = app.caption_target_count;
                    let eta = app
                        .caption_started_at
                        .map(|started| {
                            let elapsed = started.elapsed().as_secs_f64();
                            if done > 0 {
                                elapsed / done as f64 * (total - done) as f64
                            } else {
                                0.0
                            }
                        })
                        .unwrap_or(0.0);
                    ui.label(format!("{done} / {total} - ETA {eta:.0} s"));
                }
            });
            if !app.caption_failures.is_empty() {
                ui.separator();
                ui.label(format!("{} failed:", app.caption_failures.len()));
                for (name, error) in app.caption_failures.iter().take(10) {
                    ui.label(format!("{name}: {error}"));
                }
            }
        });
}

/// Bottom panel: the captioning log (the Python app's console dock).
pub fn log_panel(ctx: &egui::Context, app: &mut TagGuiApp) {
    egui::TopBottomPanel::bottom("log")
        .default_height(120.0)
        .resizable(true)
        .show(ctx, |ui| {
            ui.heading("Console");
            egui::ScrollArea::vertical()
                .stick_to_bottom(true)
                .show(ui, |ui| {
                    for line in &app.log {
                        ui.label(line);
                    }
                });
        });
}
