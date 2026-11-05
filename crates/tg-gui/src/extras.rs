//! The remaining Python-app parity surfaces: dataset statistics, the
//! All-Tags editor (rename/delete across the dataset), find & replace,
//! batch reorder, export, saved filters, tag presets, Danbooru
//! autocomplete, and EXIF orientation.

use std::path::PathBuf;

use tg_core::settings::Settings;
use tg_core::sidecar;
use tg_core::stats;
use tg_core::tags::{
    delete_tags_in_image, find_and_replace_in_caption_mut, rename_tags_in_image,
    reverse_tags_keeping_first, sort_tags_keeping_first,
};

/// A parsed Danbooru tags CSV for autocomplete: `(tag, count)`, sorted by
/// count descending.
pub struct AutocompleteList {
    pub entries: Vec<(String, u64)>,
}

impl AutocompleteList {
    /// Danbooru CSVs are `name,count,category,...`.
    pub fn load(path: &std::path::Path) -> Option<Self> {
        let mut reader = match csv::ReaderBuilder::new().has_headers(false).from_path(path) {
            Ok(reader) => reader,
            Err(_) => return None,
        };
        let mut entries = Vec::new();
        for record in reader.records().flatten() {
            let Some(tag) = record.get(0) else {
                continue;
            };
            let count = record
                .get(1)
                .and_then(|count| count.parse().ok())
                .unwrap_or(0);
            entries.push((tag.to_string(), count));
        }
        entries.sort_unstable_by_key(|entry| std::cmp::Reverse(entry.1));
        Some(Self { entries })
    }

    /// Up to `limit` suggestions for a prefix (case-insensitive).
    pub fn suggest(&self, prefix: &str, limit: usize) -> Vec<&str> {
        let prefix = prefix.to_lowercase();
        let mut matches = Vec::new();
        if prefix.is_empty() {
            return matches;
        }
        for (tag, _count) in &self.entries {
            if tag.to_lowercase().starts_with(&prefix) {
                matches.push(tag.as_str());
                if matches.len() >= limit {
                    break;
                }
            }
        }
        matches
    }
}

/// Read the EXIF orientation of an image file (1-8), defaulting to 1.
pub fn exif_orientation(path: &std::path::Path) -> u32 {
    let Ok(file) = std::fs::File::open(path) else {
        return 1;
    };
    let mut buffered = std::io::BufReader::new(file);
    let Ok(exif) = exif::Reader::new().read_from_container(&mut buffered) else {
        return 1;
    };
    exif.get_field(exif::Tag::Orientation, exif::In::PRIMARY)
        .and_then(|field| field.value.get_uint(0))
        .unwrap_or(1)
}

/// Apply an EXIF orientation (1-8) to an RGBA image in place.
pub fn apply_orientation(image: &mut image::RgbaImage, orientation: u32) {
    *image = match orientation {
        2 => image::imageops::flip_horizontal(image),
        3 => image::imageops::rotate180(image),
        4 => image::imageops::flip_vertical(image),
        5 => image::imageops::flip_horizontal(&image::imageops::rotate90(image)),
        6 => image::imageops::rotate90(image),
        7 => image::imageops::flip_horizontal(&image::imageops::rotate270(image)),
        8 => image::imageops::rotate270(image),
        _ => return,
    };
}

/// Panel state for the extras (inputs etc.), kept on the app.
#[derive(Default)]
pub struct ExtrasState {
    pub find_text: String,
    pub replace_text: String,
    pub find_whole_tags: bool,
    pub find_use_regex: bool,
    pub rename_old: String,
    pub rename_new: String,
    pub delete_tags: String,
    pub saved_filter_name: String,
    pub autocomplete: Option<AutocompleteList>,
    pub autocomplete_loaded: bool,
    pub export_message: String,
}

impl ExtrasState {
    /// Lazily load the Danbooru CSV for autocomplete when the path in
    /// settings is set.
    pub fn ensure_autocomplete(&mut self, settings: &Settings) {
        if self.autocomplete_loaded {
            return;
        }
        self.autocomplete_loaded = true;
        let path = &settings.danbooru_tags_csv_path;
        if !path.trim().is_empty() {
            self.autocomplete = AutocompleteList::load(std::path::Path::new(path.trim()));
        }
    }
}

/// The All-Tags editor + statistics + find/replace + reorder + export,
/// as a bottom panel. Port of the All Tags pane, Dataset Statistics
/// dock, Find and Replace dialog, batch reorder, and export dialogs'
/// core workflows.
pub fn extras_panel(
    ctx: &egui::Context,
    app: &mut crate::app::TagGuiApp,
    extras: &mut ExtrasState,
) {
    egui::TopBottomPanel::bottom("extras")
        .default_height(180.0)
        .resizable(true)
        .show(ctx, |ui| {
            let stats = stats::compute(&app.images, 8);
            ui.horizontal(|ui| {
                ui.heading("Dataset");
                ui.label(format!(
                    "{} images - {} tagged, {} untagged - {} tags total \
                     ({:.1}/image), {} unique",
                    stats.total_images,
                    stats.tagged_images,
                    stats.untagged_images,
                    stats.total_tags,
                    stats.average_tags_per_image,
                    stats.unique_tags
                ));
                if stats.total_images > 0 {
                    ui.label(format!(
                        "landscape/square/portrait: {}/{} /{}/{} /{}/{}",
                        stats.ratio_summary.0,
                        stats.total_images,
                        stats.ratio_summary.1,
                        stats.total_images,
                        stats.ratio_summary.2,
                        stats.total_images
                    ));
                }
            });
            ui.separator();
            let stats_clone = stats.clone();
            ui.columns(3, |columns| {
                let stats = &stats_clone;
                // Column 0: statistics summary + top tags.
                columns[0].vertical(|ui| {
                    ui.strong("Tag frequency (top)");
                    for (tag, count) in &stats.top_tags {
                        ui.horizontal(|ui| {
                            if ui
                                .selectable_label(false, format!("{tag} ({count})"))
                                .clicked()
                            {
                                app.filter_text = format!("tag:\"{tag}\"");
                                app.apply_filter();
                            }
                        });
                    }
                });
                // Column 1: all-tags rename/delete.
                columns[1].vertical(|ui| {
                    ui.strong("All tags");
                    egui::Grid::new("all_tags_grid").show(ui, |ui| {
                        ui.label("Rename");
                        ui.add(egui::TextEdit::singleline(&mut extras.rename_old));
                        ui.label("to");
                        ui.add(egui::TextEdit::singleline(&mut extras.rename_new));
                        if ui.button("Apply").clicked() && !extras.rename_old.trim().is_empty() {
                            rename_everywhere(app, &extras.rename_old, &extras.rename_new);
                        }
                        ui.end_row();
                        ui.label("Delete");
                        ui.add(egui::TextEdit::singleline(&mut extras.delete_tags));
                        if ui.button("Delete everywhere").clicked()
                            && !extras.delete_tags.trim().is_empty()
                        {
                            delete_everywhere(app, &extras.delete_tags);
                        }
                        ui.end_row();
                    });
                    ui.strong("Batch reorder (visible images)");
                    ui.horizontal(|ui| {
                        if ui.button("Sort A-Z (keep first)").clicked() {
                            reorder_visible(app, false);
                        }
                        if ui.button("Reverse (keep first)").clicked() {
                            reorder_visible(app, true);
                        }
                    });
                });
                // Column 2: find & replace + export + presets.
                columns[2].vertical(|ui| {
                    ui.strong("Find and replace (visible images)");
                    egui::Grid::new("find_replace_grid").show(ui, |ui| {
                        ui.label("Find");
                        ui.add(egui::TextEdit::singleline(&mut extras.find_text));
                        ui.label("Replace with");
                        ui.add(egui::TextEdit::singleline(&mut extras.replace_text));
                        ui.end_row();
                        ui.checkbox(&mut extras.find_whole_tags, "Whole tags only");
                        ui.checkbox(&mut extras.find_use_regex, "Use regex");
                        if ui.button("Replace").clicked() && !extras.find_text.trim().is_empty() {
                            find_replace_visible(app, extras);
                        }
                        ui.end_row();
                    });
                    ui.strong("Export");
                    ui.horizontal(|ui| {
                        if ui.button("tags.csv").clicked() {
                            export_csv(app, "tags.csv");
                            extras.export_message = String::from("tags.csv exported");
                        }
                        if ui.button("captions.txt").clicked() {
                            export_captions(app);
                            extras.export_message = String::from("captions.txt exported");
                        }
                        if ui.button("tag_frequency.csv").clicked() {
                            export_frequency(app, "tag_frequency.csv");
                            extras.export_message = String::from("tag_frequency.csv exported");
                        }
                    });
                    if !extras.export_message.is_empty() {
                        ui.label(&extras.export_message);
                    }
                });
            });
        });
}

fn write_export(app: &mut crate::app::TagGuiApp, name: &str, content: &str) {
    let folder = app.folder.clone().unwrap_or_else(|| PathBuf::from("."));
    let path = folder.join(name);
    match std::fs::write(&path, content) {
        Ok(()) => app.log(format!("Exported {}", path.display())),
        Err(error) => app.log(format!("Export failed: {error}")),
    }
}

fn export_csv(app: &mut crate::app::TagGuiApp, name: &str) {
    let separator = app.settings.effective_tag_separator();
    let mut content = String::from("image name,caption\n");
    for image in &app.images {
        let caption = image.tags.join(&separator);
        content.push_str(&format!(
            "{},\"{}\"\n",
            image.path.file_name().unwrap_or_default().to_string_lossy(),
            caption.replace('"', "\"\"")
        ));
    }
    write_export(app, name, &content);
}

fn export_captions(app: &mut crate::app::TagGuiApp) {
    let separator = app.settings.effective_tag_separator();
    let mut content = String::new();
    for image in &app.images {
        content.push_str(&image.tags.join(&separator));
        content.push('\n');
    }
    write_export(app, "captions.txt", &content);
}

fn export_frequency(app: &mut crate::app::TagGuiApp, name: &str) {
    let stats = stats::compute(&app.images, usize::MAX);
    let mut content = String::from("tag,count\n");
    for (tag, count) in &stats.top_tags {
        content.push_str(&format!("{tag},{count}\n"));
    }
    write_export(app, name, &content);
}

fn rename_everywhere(app: &mut crate::app::TagGuiApp, old: &str, new: &str) {
    let use_regex = old.contains('*') || old.contains('(');
    app.push_undo();
    let old_tags = vec![old.to_owned()];
    let separator = app.settings.effective_tag_separator();
    let mut changed = 0;
    for image in &mut app.images {
        let mut tags = image.tags.clone();
        if rename_tags_in_image(&mut tags, &old_tags, new, use_regex)
            && sidecar::write_tags(&image.path, &tags, &separator).is_ok()
        {
            image.tags = tags;
            changed += 1;
        }
    }
    app.log(format!("Renamed tags in {changed} images"));
    app.apply_filter();
    app.sync_tag_edit();
}

fn delete_everywhere(app: &mut crate::app::TagGuiApp, tags_text: &str) {
    let use_regex = tags_text.contains('*') || tags_text.contains('(');
    app.push_undo();
    let old_tags = vec![tags_text.trim().to_owned()];
    let separator = app.settings.effective_tag_separator();
    let mut changed = 0;
    for image in &mut app.images {
        let mut tags = image.tags.clone();
        if delete_tags_in_image(&mut tags, &old_tags, use_regex)
            && sidecar::write_tags(&image.path, &tags, &separator).is_ok()
        {
            image.tags = tags;
            changed += 1;
        }
    }
    app.log(format!("Deleted tags in {changed} images"));
    app.apply_filter();
    app.sync_tag_edit();
}

fn find_replace_visible(app: &mut crate::app::TagGuiApp, extras: &ExtrasState) {
    app.push_undo();
    let separator = app.settings.effective_tag_separator();
    let visible: Vec<usize> = app.visible_indices.clone();
    let mut changed = 0;
    for index in visible {
        let mut tags = app.images[index].tags.clone();
        let modified = if extras.find_whole_tags {
            let old = vec![extras.find_text.clone()];
            let was = tags.clone();
            if !extras.replace_text.trim().is_empty() {
                rename_tags_in_image(&mut tags, &old, &extras.replace_text, extras.find_use_regex);
            } else {
                delete_tags_in_image(&mut tags, &old, extras.find_use_regex);
            }
            tags != was
        } else {
            find_and_replace_in_caption_mut(
                &mut tags,
                &extras.find_text,
                &extras.replace_text,
                extras.find_use_regex,
                &separator,
            )
        };
        if modified && sidecar::write_tags(&app.images[index].path, &tags, &separator).is_ok() {
            app.images[index].tags = tags;
            changed += 1;
        }
    }
    app.log(format!("Find and replace changed {changed} images"));
    app.apply_filter();
    app.sync_tag_edit();
}

fn reorder_visible(app: &mut crate::app::TagGuiApp, descending: bool) {
    app.push_undo();
    let separator = app.settings.effective_tag_separator();
    let visible: Vec<usize> = app.visible_indices.clone();
    let mut changed = 0;
    for index in visible {
        let tags = std::mem::take(&mut app.images[index].tags);
        let reordered = if descending {
            reverse_tags_keeping_first(&tags)
        } else {
            sort_tags_keeping_first(&tags, false)
        };
        if reordered != tags {
            if sidecar::write_tags(&app.images[index].path, &reordered, &separator).is_ok() {
                app.images[index].tags = reordered;
                changed += 1;
            } else {
                app.images[index].tags = tags;
            }
        } else {
            app.images[index].tags = tags;
        }
    }
    app.log(format!("Reordered tags in {changed} images"));
    app.sync_tag_edit();
}

/// Saved filters + presets UI, shown in the top bar.
pub fn saved_filters_ui(
    ctx: &egui::Context,
    app: &mut crate::app::TagGuiApp,
    extras: &mut ExtrasState,
) {
    egui::TopBottomPanel::top("saved_filters_bar").show(ctx, |ui| {
        ui.horizontal(|ui| {
            ui.label("Saved filters:");
            let names: Vec<String> = app.settings.saved_filters.keys().cloned().collect();
            egui::ComboBox::from_id_salt("saved_filter")
                .selected_text(extras.saved_filter_name.as_str())
                .show_ui(ui, |ui| {
                    for name in &names {
                        let _ = ui.selectable_label(*name == extras.saved_filter_name, name);
                        if ui.selectable_label(false, name).clicked() {
                            extras.saved_filter_name = name.clone();
                            if let Some(text) = app.settings.saved_filters.get(name) {
                                app.filter_text = text.clone();
                                app.apply_filter();
                            }
                        }
                    }
                });
            if !names.is_empty()
                && ui.small_button("Delete").clicked()
                && !extras.saved_filter_name.is_empty()
            {
                app.settings.saved_filters.remove(&extras.saved_filter_name);
                extras.saved_filter_name.clear();
                let _ = app.settings.save(&tg_core::settings::default_config_path());
            }
        });
    });
}

/// Autocomplete suggestions for the tag editor's current last token;
/// `None` when there is nothing to suggest.
pub fn autocomplete_suggestions(
    app: &mut crate::app::TagGuiApp,
    extras: &mut ExtrasState,
) -> Option<Vec<String>> {
    if !app.settings.autocomplete_tags {
        return None;
    }
    extras.ensure_autocomplete(&app.settings);
    let list = extras.autocomplete.as_ref()?;
    let separator = &app.settings.effective_tag_separator();
    let text = &app.tag_edit_text;
    let prefix = text
        .rsplit(separator.as_str())
        .next()
        .unwrap_or("")
        .trim()
        .to_string();
    if prefix.len() < 2 {
        return None;
    }
    Some(
        list.suggest(&prefix, 8)
            .into_iter()
            .map(str::to_owned)
            .collect(),
    )
}

fn apply_suggestion(app: &mut crate::app::TagGuiApp, extras: &ExtrasState, suggestion: &str) {
    let separator = app.settings.effective_tag_separator();
    let text = app.tag_edit_text.clone();
    let Some(last_start) = text.rfind(separator.as_str()) else {
        app.tag_edit_text = suggestion.to_string();
        return;
    };
    let head = &text[..last_start + separator.len()];
    app.tag_edit_text = format!("{head}{suggestion}");
    let _ = extras;
}

/// Render the autocomplete popup; returns true if a suggestion was
/// clicked (the caller should commit the tag edit).
pub fn autocomplete_popup(
    ui: &mut egui::Ui,
    app: &mut crate::app::TagGuiApp,
    extras: &mut ExtrasState,
) -> bool {
    let Some(suggestions) = autocomplete_suggestions(app, extras) else {
        return false;
    };
    let mut clicked = false;
    ui.horizontal_wrapped(|ui| {
        for suggestion in &suggestions {
            let button = egui::Button::new(suggestion.clone()).small();
            if ui.add(button).clicked() {
                apply_suggestion(app, extras, suggestion);
                clicked = true;
            }
        }
    });
    clicked
}
