use crate::{
    db::Database,
    geometry::{Annotation, Point, Shape},
    image_data::{load_image, ImageTransform, LoadedImage},
    schema::{self, AppKeybinds, KeybindConfig, LabelType, SchemaDefinition, SchemaLabel},
};
use anyhow::{bail, Context, Result};
use eframe::egui::{
    self, pos2, vec2, CollapsingHeader, Color32, Context as EguiContext, Key, Modifiers, Pos2,
    Rect, Sense, Shape as EguiShape, Stroke, TextureHandle, TextureOptions, TopBottomPanel, Vec2,
};
use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
};

const DB_PATH: &str = "labels.sqlite3";
const HANDLE_RADIUS: f32 = 5.0;
const HANDLE_HIT_RADIUS: f32 = 10.0;

#[derive(Clone)]
struct LabelBinding {
    id: i64,
    name: String,
    color_rgb: [u8; 3],
    label_type: LabelType,
    keybind: String,
}

#[derive(Clone)]
struct AnnotationEditState {
    annotation_id: i64,
    handle_index: usize,
    original_shape: Shape,
}

#[derive(Clone, Copy)]
struct ParsedKeybind {
    key: Key,
    modifiers: Modifiers,
}

pub struct LabelerApp {
    db: Database,
    labels: Vec<LabelBinding>,
    selected_label_id: Option<i64>,
    image_classifications: HashSet<i64>,
    annotations: Vec<Annotation>,
    image: Option<LoadedImage>,
    texture: Option<TextureHandle>,
    browser_dir: PathBuf,
    image_path_input: String,
    transform_save_path_input: String,
    status: String,
    rect_start: Option<Point>,
    rect_current: Option<Point>,
    polygon_points: Vec<Point>,
    selected_annotation_id: Option<i64>,
    annotation_edit: Option<AnnotationEditState>,
    zoom: f32,
    pan: Vec2,
    brightness: f32,
    contrast: f32,
    image_transform: ImageTransform,
    schema_names: Vec<String>,
    selected_schema_name: String,
    schema_name_input: String,
    schema_import_path_input: String,
    schema_definition: SchemaDefinition,
    app_keybinds: AppKeybinds,
}

impl LabelerApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Result<Self> {
        let db = Database::open(&PathBuf::from(DB_PATH))?;
        let schema_names = schema::list_schema_names()?;
        let selected_schema_name = schema_names
            .iter()
            .find(|name| name.as_str() == "default")
            .cloned()
            .or_else(|| schema_names.first().cloned())
            .context("no schema files found")?;
        let schema_definition = schema::load_schema(&selected_schema_name)?;
        let app_keybinds = schema::load_app_keybinds()?;
        let browser_dir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));

        cc.egui_ctx.set_pixels_per_point(1.25);

        let mut app = Self {
            db,
            labels: Vec::new(),
            selected_label_id: None,
            image_classifications: HashSet::new(),
            annotations: Vec::new(),
            image: None,
            texture: None,
            browser_dir,
            image_path_input: String::new(),
            transform_save_path_input: String::new(),
            status: format!(
                "Database: {} | Schemas: {}",
                DB_PATH,
                schema::config_dir()?.display()
            ),
            rect_start: None,
            rect_current: None,
            polygon_points: Vec::new(),
            selected_annotation_id: None,
            annotation_edit: None,
            zoom: 1.0,
            pan: Vec2::ZERO,
            brightness: 0.0,
            contrast: 1.0,
            image_transform: ImageTransform::default(),
            schema_names,
            selected_schema_name: selected_schema_name.clone(),
            schema_name_input: selected_schema_name,
            schema_import_path_input: String::new(),
            schema_definition,
            app_keybinds,
        };
        app.apply_schema(
            app.selected_schema_name.clone(),
            app.schema_definition.clone(),
        )?;
        Ok(app)
    }

    fn apply_schema(
        &mut self,
        schema_name: String,
        schema_definition: SchemaDefinition,
    ) -> Result<()> {
        let previous_selection = self
            .selected_annotation_label()
            .map(|label| label.name.clone());
        let labels = self.sync_schema_labels(&schema_definition)?;

        self.labels = labels;
        self.selected_label_id = previous_selection
            .as_deref()
            .and_then(|name| {
                self.labels
                    .iter()
                    .find(|label| label.name == name && label.label_type != LabelType::Global)
                    .map(|label| label.id)
            })
            .or_else(|| {
                self.labels
                    .iter()
                    .find(|label| label.label_type != LabelType::Global)
                    .map(|label| label.id)
            });
        self.selected_schema_name = schema_name.clone();
        self.schema_name_input = schema_name;
        self.schema_definition = schema_definition;
        Ok(())
    }

    fn sync_schema_labels(
        &self,
        schema_definition: &SchemaDefinition,
    ) -> Result<Vec<LabelBinding>> {
        if schema_definition.labels.is_empty() {
            bail!("schema must contain at least one label");
        }

        let mut seen = HashSet::new();
        let mut labels = Vec::with_capacity(schema_definition.labels.len());

        for label in &schema_definition.labels {
            let name = label.name.trim();
            if name.is_empty() {
                bail!("labels cannot have an empty name");
            }
            if !seen.insert(name.to_ascii_lowercase()) {
                bail!("schema label `{name}` is duplicated");
            }

            let class_id = self.db.upsert_class(name, label.color_rgb)?;
            labels.push(LabelBinding {
                id: class_id,
                name: name.to_string(),
                color_rgb: label.color_rgb,
                label_type: label.label_type,
                keybind: label.keybind.chord.trim().to_string(),
            });
        }

        labels.sort_by(|left, right| left.name.cmp(&right.name));
        Ok(labels)
    }

    fn selected_annotation_label(&self) -> Option<&LabelBinding> {
        let selected_id = self.selected_label_id?;
        self.labels
            .iter()
            .find(|label| label.id == selected_id && label.label_type != LabelType::Global)
    }

    fn global_labels(&self) -> impl Iterator<Item = &LabelBinding> {
        self.labels
            .iter()
            .filter(|label| label.label_type == LabelType::Global)
    }

    fn clear_transient_state(&mut self) {
        self.rect_start = None;
        self.rect_current = None;
        self.polygon_points.clear();
        self.annotation_edit = None;
    }

    fn load_schema_by_name(&mut self, name: &str) {
        match schema::load_schema(name)
            .and_then(|definition| self.apply_schema(name.to_string(), definition))
        {
            Ok(()) => {
                self.status = format!("Loaded schema `{name}`");
                self.clear_transient_state();
            }
            Err(error) => {
                self.status = format!("Failed to load schema `{name}`: {error:#}");
            }
        }
    }

    fn save_schema(&mut self) {
        let schema_name = self.schema_name_input.trim().to_string();
        let schema_definition = self.schema_definition.clone();
        match schema::save_schema(&schema_name, &schema_definition) {
            Ok(path) => {
                let name = path
                    .file_stem()
                    .and_then(|value| value.to_str())
                    .unwrap_or("default")
                    .to_string();
                match schema::list_schema_names().and_then(|names| {
                    self.schema_names = names;
                    self.apply_schema(name.clone(), schema_definition)?;
                    Ok(name)
                }) {
                    Ok(name) => {
                        self.status = format!("Saved schema to {}", path.display());
                        self.selected_schema_name = name;
                    }
                    Err(error) => {
                        self.status = format!("Saved schema, but refresh failed: {error:#}");
                    }
                }
            }
            Err(error) => {
                self.status = format!("Failed to save schema: {error:#}");
            }
        }
    }

    fn add_label(&mut self) {
        let next_index = self.schema_definition.labels.len() + 1;
        self.schema_definition.labels.push(SchemaLabel {
            name: format!("label-{next_index}"),
            label_type: LabelType::Rectangle,
            color_rgb: deterministic_color(&format!("label-{next_index}")),
            keybind: KeybindConfig::default(),
        });
    }

    fn new_schema(&mut self) {
        let base_name = self.schema_name_input.trim();
        self.schema_name_input = if base_name.is_empty() {
            "new-schema".to_string()
        } else {
            format!("{base_name}-copy")
        };
        self.selected_schema_name.clear();
        self.schema_definition = SchemaDefinition { labels: Vec::new() };
        self.add_label();
        self.clear_transient_state();
        self.status = "Started a new schema".to_string();
    }

    fn import_schema(&mut self) {
        let import_path = PathBuf::from(self.schema_import_path_input.trim());
        match schema::import_schema_from_file(&import_path).and_then(
            |(name, saved_path, schema_definition)| {
                self.schema_names = schema::list_schema_names()?;
                self.apply_schema(name.clone(), schema_definition)?;
                Ok((name, saved_path))
            },
        ) {
            Ok((name, saved_path)) => {
                self.selected_schema_name = name;
                self.status = format!("Imported schema to {}", saved_path.display());
            }
            Err(error) => {
                self.status = format!("Failed to import schema: {error:#}");
            }
        }
    }

    fn save_app_keybinds(&mut self) {
        match schema::save_app_keybinds(&self.app_keybinds) {
            Ok(path) => self.status = format!("Saved app keybinds to {}", path.display()),
            Err(error) => self.status = format!("Failed to save app keybinds: {error:#}"),
        }
    }

    fn load_image(&mut self, ctx: &EguiContext) {
        match self.try_load_image(ctx) {
            Ok(()) => {}
            Err(error) => self.status = format!("Load failed: {error:#}"),
        }
    }

    fn try_load_image(&mut self, ctx: &EguiContext) -> Result<()> {
        let path = PathBuf::from(self.image_path_input.trim());
        let image = load_image(&path)?;
        self.db.upsert_image(
            &image.hash,
            image.width,
            image.height,
            image.bit_depth,
            &image.path,
        )?;
        let annotations = self.db.list_annotations(&image.hash)?;
        let classifications = self.db.list_image_classifications(&image.hash)?;
        self.image_transform = ImageTransform::default();
        let display = image.transformed_adjusted_display(
            self.image_transform,
            self.brightness,
            self.contrast,
        );
        let texture = ctx.load_texture(
            format!("image:{}", image.hash),
            display,
            TextureOptions::NEAREST,
        );

        self.status = format!(
            "Loaded {} ({}x{}, {}-bit, sha256 {})",
            image.path, image.width, image.height, image.bit_depth, image.hash
        );
        self.transform_save_path_input = default_rotated_output_path(Path::new(&image.path))
            .display()
            .to_string();
        self.image = Some(image);
        self.texture = Some(texture);
        self.annotations = annotations;
        self.image_classifications = classifications.into_iter().collect();
        self.clear_transient_state();
        self.selected_annotation_id = None;
        self.reset_view(ctx);
        Ok(())
    }

    fn refresh_texture(&mut self) {
        if let (Some(image), Some(texture)) = (&self.image, &mut self.texture) {
            texture.set(
                image.transformed_adjusted_display(
                    self.image_transform,
                    self.brightness,
                    self.contrast,
                ),
                TextureOptions::NEAREST,
            );
        }
    }

    fn reset_view(&mut self, ctx: &EguiContext) {
        self.zoom = 1.0;
        self.pan = Vec2::ZERO;
        self.brightness = 0.0;
        self.contrast = 1.0;
        self.refresh_texture();
        ctx.request_repaint();
    }

    fn save_shape(&mut self, shape: Shape) {
        let Some(image) = &self.image else {
            self.status = "Load an image first".to_string();
            return;
        };
        let Some(class_id) = self.selected_label_id else {
            self.status = "Select a schema label before creating annotations".to_string();
            return;
        };
        let Some(expected_type) = self
            .selected_annotation_label()
            .map(|label| label.label_type)
        else {
            self.status = "Selected schema label is invalid".to_string();
            return;
        };
        if !shape_matches_annotation_type(&shape, expected_type) {
            self.status = format!("Selected label only accepts {}", expected_type.as_str());
            return;
        }
        let Some(shape) = shape.normalized() else {
            self.status = "Shape was too small or incomplete".to_string();
            return;
        };

        match self
            .db
            .insert_annotation(&image.hash, class_id, &shape)
            .and_then(|_| self.db.list_annotations(&image.hash))
        {
            Ok(annotations) => {
                self.annotations = annotations;
                self.status = "Annotation saved".to_string();
                self.clear_transient_state();
            }
            Err(error) => self.status = format!("Failed to save annotation: {error:#}"),
        }
    }

    fn delete_selected_annotation(&mut self) {
        let Some(annotation_id) = self.selected_annotation_id else {
            self.status = "No annotation selected".to_string();
            return;
        };

        match self.db.delete_annotation(annotation_id).and_then(|_| {
            self.image
                .as_ref()
                .context("image not loaded")
                .and_then(|image| self.db.list_annotations(&image.hash))
        }) {
            Ok(annotations) => {
                self.annotations = annotations;
                self.selected_annotation_id = None;
                self.annotation_edit = None;
                self.status = format!("Deleted annotation {annotation_id}");
            }
            Err(error) => self.status = format!("Delete failed: {error:#}"),
        }
    }

    fn set_image_classification(&mut self, class_id: i64, present: bool) {
        let Some(image) = &self.image else {
            self.status = "Load an image first".to_string();
            return;
        };

        match self
            .db
            .set_image_classification(&image.hash, class_id, present)
        {
            Ok(()) => {
                if present {
                    self.image_classifications.insert(class_id);
                } else {
                    self.image_classifications.remove(&class_id);
                }
            }
            Err(error) => {
                self.status = format!("Failed to update image classification: {error:#}");
            }
        }
    }

    fn toggle_image_classification(&mut self, class_id: i64) {
        let next = !self.image_classifications.contains(&class_id);
        self.set_image_classification(class_id, next);
    }

    fn annotation_mut(&mut self, annotation_id: i64) -> Option<&mut Annotation> {
        self.annotations
            .iter_mut()
            .find(|annotation| annotation.id == annotation_id)
    }

    fn begin_annotation_edit(&mut self, annotation_id: i64, handle_index: usize) {
        let original_shape = self
            .annotations
            .iter()
            .find(|annotation| annotation.id == annotation_id)
            .map(|annotation| annotation.shape.clone());
        if let Some(original_shape) = original_shape {
            self.clear_transient_state();
            self.selected_annotation_id = Some(annotation_id);
            self.annotation_edit = Some(AnnotationEditState {
                annotation_id,
                handle_index,
                original_shape,
            });
        }
    }

    fn update_annotation_edit(&mut self, point: Point) {
        let Some(edit) = self.annotation_edit.clone() else {
            return;
        };
        let Some(annotation) = self.annotation_mut(edit.annotation_id) else {
            return;
        };
        if let Some(updated_shape) =
            updated_shape_for_handle(&annotation.shape, edit.handle_index, point)
        {
            annotation.shape = updated_shape;
        }
    }

    fn finish_annotation_edit(&mut self) {
        let Some(edit) = self.annotation_edit.take() else {
            return;
        };

        let Some(annotation) = self
            .annotations
            .iter()
            .find(|annotation| annotation.id == edit.annotation_id)
        else {
            return;
        };
        let shape = annotation.shape.clone();
        if let Err(error) = self.db.update_annotation(edit.annotation_id, &shape) {
            if let Some(annotation) = self.annotation_mut(edit.annotation_id) {
                annotation.shape = edit.original_shape;
            }
            self.status = format!("Failed to update annotation: {error:#}");
            return;
        }
        self.status = format!("Updated annotation {}", edit.annotation_id);
    }

    fn select_annotation(&mut self, annotation_id: i64) {
        self.selected_annotation_id = Some(annotation_id);
        self.rect_start = None;
        self.rect_current = None;
        self.polygon_points.clear();
    }

    fn save_transformed_image(&mut self) {
        let Some(image) = &self.image else {
            self.status = "Load an image first".to_string();
            return;
        };

        let output_path = PathBuf::from(self.transform_save_path_input.trim());
        if output_path.as_os_str().is_empty() {
            self.status = "Output path cannot be empty".to_string();
            return;
        }
        if output_path == PathBuf::from(&image.path) {
            self.status = "Refusing to overwrite the original image".to_string();
            return;
        }
        if output_path
            .extension()
            .and_then(|value| value.to_str())
            .map(|value| value.to_ascii_lowercase())
            .as_deref()
            != Some("png")
        {
            self.status = "Transformed images must be saved as PNG".to_string();
            return;
        }
        if let Some(parent) = output_path.parent() {
            if let Err(error) = fs::create_dir_all(parent) {
                self.status = format!("Failed to create output directory: {error:#}");
                return;
            }
        }
        if !image.source_format.allows_transform_export() {
            self.status =
                "Rotation and mirroring are disabled for DICOM/DICONDE images".to_string();
            return;
        }

        match image.export_transformed_png(self.image_transform, &output_path) {
            Ok(()) => {
                self.status = format!("Saved transformed image to {}", output_path.display());
            }
            Err(error) => {
                self.status = format!("Failed to save transformed image: {error:#}");
            }
        }
    }

    fn load_adjacent_image(&mut self, ctx: &EguiContext, offset: isize) {
        let Some(current_path) = (!self.image_path_input.trim().is_empty())
            .then(|| PathBuf::from(self.image_path_input.trim()))
        else {
            self.status = "Load an image first".to_string();
            return;
        };

        let root = navigation_root(&self.browser_dir, &current_path);
        let images = collect_supported_images(&root);
        let Some(current_index) = images.iter().position(|path| path == &current_path) else {
            self.status = format!("Current image is not inside {}", root.display());
            return;
        };
        let next_index = current_index as isize + offset;
        if next_index < 0 || next_index >= images.len() as isize {
            self.status = "No more images in that direction".to_string();
            return;
        }

        self.image_path_input = images[next_index as usize].display().to_string();
        self.load_image(ctx);
    }

    fn rotate_left(&mut self) {
        let Some(image) = &self.image else {
            self.status = "Load an image first".to_string();
            return;
        };
        if !image.source_format.allows_transform_export() {
            self.status = "Rotation is disabled for DICOM/DICONDE images".to_string();
            return;
        }
        self.image_transform.rotate_left();
        self.refresh_texture();
        self.status = "Rotated left".to_string();
    }

    fn rotate_right(&mut self) {
        let Some(image) = &self.image else {
            self.status = "Load an image first".to_string();
            return;
        };
        if !image.source_format.allows_transform_export() {
            self.status = "Rotation is disabled for DICOM/DICONDE images".to_string();
            return;
        }
        self.image_transform.rotate_right();
        self.refresh_texture();
        self.status = "Rotated right".to_string();
    }

    fn toggle_mirror_horizontal(&mut self) {
        let Some(image) = &self.image else {
            self.status = "Load an image first".to_string();
            return;
        };
        if !image.source_format.allows_transform_export() {
            self.status = "Mirroring is disabled for DICOM/DICONDE images".to_string();
            return;
        }
        self.image_transform.toggle_mirror_horizontal();
        self.refresh_texture();
        self.status = "Toggled horizontal mirror".to_string();
    }

    fn toggle_mirror_vertical(&mut self) {
        let Some(image) = &self.image else {
            self.status = "Load an image first".to_string();
            return;
        };
        if !image.source_format.allows_transform_export() {
            self.status = "Mirroring is disabled for DICOM/DICONDE images".to_string();
            return;
        }
        self.image_transform.toggle_mirror_vertical();
        self.refresh_texture();
        self.status = "Toggled vertical mirror".to_string();
    }

    fn handle_keybindings(&mut self, ctx: &EguiContext) {
        if ctx.wants_keyboard_input() {
            return;
        }

        let actions = self.app_keybinds.clone();
        if keybind_pressed(ctx, &actions.previous_image.chord) {
            self.load_adjacent_image(ctx, -1);
            return;
        }
        if keybind_pressed(ctx, &actions.next_image.chord) {
            self.load_adjacent_image(ctx, 1);
            return;
        }
        if keybind_pressed(ctx, &actions.rotate_left.chord) {
            self.rotate_left();
            return;
        }
        if keybind_pressed(ctx, &actions.rotate_right.chord) {
            self.rotate_right();
            return;
        }
        if keybind_pressed(ctx, &actions.mirror_horizontal.chord) {
            self.toggle_mirror_horizontal();
            return;
        }
        if keybind_pressed(ctx, &actions.mirror_vertical.chord) {
            self.toggle_mirror_vertical();
            return;
        }
        if keybind_pressed(ctx, &actions.save_transformed_image.chord) {
            self.save_transformed_image();
            return;
        }

        for label in self.labels.clone() {
            if label.label_type == LabelType::Global && keybind_pressed(ctx, &label.keybind) {
                self.toggle_image_classification(label.id);
                self.status = format!("Toggled image label `{}`", label.name);
                return;
            }
        }

        for label in self.labels.clone() {
            if label.label_type != LabelType::Global && keybind_pressed(ctx, &label.keybind) {
                self.selected_label_id = Some(label.id);
                self.clear_transient_state();
                self.status = format!("Selected label `{}`", label.name);
                return;
            }
        }
    }

    fn show_left_panel(&mut self, ctx: &EguiContext) {
        egui::SidePanel::left("side_panel")
            .resizable(true)
            .default_width(380.0)
            .show(ctx, |ui| {
                egui::ScrollArea::vertical()
                    .id_source("sidebar_scroll")
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        CollapsingHeader::new("Image")
                            .default_open(true)
                            .show(ui, |ui| {
                                ui.label("Selected path");
                                ui.add_enabled(
                                    false,
                                    egui::TextEdit::singleline(&mut self.image_path_input),
                                );
                                if let Some(image) = &self.image {
                                    ui.separator();
                                    ui.label(format!("Size: {} x {}", image.width, image.height));
                                    ui.label(format!("Bit depth: {}", image.bit_depth));
                                    ui.label("Raw-pixel SHA-256");
                                    ui.monospace(&image.hash);
                                    let active_globals = self
                                        .global_labels()
                                        .filter(|label| self.image_classifications.contains(&label.id))
                                        .map(|label| label.name.clone())
                                        .collect::<Vec<_>>();
                                    ui.separator();
                                    ui.label("Global labels");
                                    if active_globals.is_empty() {
                                        ui.label("None");
                                    } else {
                                        for name in active_globals {
                                            ui.colored_label(Color32::LIGHT_GREEN, name);
                                        }
                                    }
                                }
                            });

                        CollapsingHeader::new("Browser")
                            .default_open(true)
                            .show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    if ui.button("Up").clicked() {
                                        if let Some(parent) = self.browser_dir.parent() {
                                            self.browser_dir = parent.to_path_buf();
                                        }
                                    }
                                    if ui.button("Reload").clicked() {
                                        ctx.request_repaint();
                                    }
                                    if ui.button("Prev").clicked() {
                                        self.load_adjacent_image(ctx, -1);
                                    }
                                    if ui.button("Next").clicked() {
                                        self.load_adjacent_image(ctx, 1);
                                    }
                                });
                                ui.monospace(self.browser_dir.display().to_string());

                                let mut selected_file = None;
                                egui::ScrollArea::vertical()
                                    .max_height(220.0)
                                    .show(ui, |ui| match list_browser_entries(&self.browser_dir) {
                                        Ok(entries) => {
                                            for entry in entries {
                                                match entry {
                                                    BrowserEntry::Directory(path) => {
                                                        let name = path
                                                            .file_name()
                                                            .map(|value| {
                                                                value.to_string_lossy().into_owned()
                                                            })
                                                            .unwrap_or_else(|| {
                                                                path.display().to_string()
                                                            });
                                                        if ui
                                                            .selectable_label(false, format!("[{name}]"))
                                                            .clicked()
                                                        {
                                                            self.browser_dir = path;
                                                        }
                                                    }
                                                    BrowserEntry::Image(path) => {
                                                        let name = path
                                                            .file_name()
                                                            .map(|value| {
                                                                value.to_string_lossy().into_owned()
                                                            })
                                                            .unwrap_or_else(|| {
                                                                path.display().to_string()
                                                            });
                                                        if ui.selectable_label(false, name).clicked() {
                                                            selected_file = Some(path);
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                        Err(error) => {
                                            ui.colored_label(
                                                Color32::RED,
                                                format!("Browser error: {error}"),
                                            );
                                        }
                                    });

                                if let Some(path) = selected_file {
                                    self.image_path_input = path.display().to_string();
                                    self.load_image(ctx);
                                }
                            });

                        CollapsingHeader::new("Labels")
                            .default_open(true)
                            .show(ui, |ui| {
                                let labels = self
                                    .labels
                                    .iter()
                                    .map(|label| {
                                        (
                                            label.id,
                                            label.name.clone(),
                                            label.color_rgb,
                                            label.label_type,
                                            label.keybind.clone(),
                                        )
                                    })
                                    .collect::<Vec<_>>();
                                for (class_id, name, color_rgb, label_type, keybind) in labels {
                                    ui.horizontal(|ui| {
                                        ui.colored_label(rgb(color_rgb), "■");
                                        match label_type {
                                            LabelType::Global => {
                                                let active = self.image_classifications.contains(&class_id);
                                                let text = if active {
                                                    format!("{name} (global)")
                                                } else {
                                                    format!("{name} (global, off)")
                                                };
                                                if ui.selectable_label(active, text).clicked() {
                                                    self.toggle_image_classification(class_id);
                                                }
                                            }
                                            _ => {
                                                let selected = self.selected_label_id == Some(class_id);
                                                let text = format!("{} ({})", name, label_type.as_str());
                                                if ui.selectable_label(selected, text).clicked() {
                                                    self.selected_label_id = Some(class_id);
                                                    self.clear_transient_state();
                                                }
                                            }
                                        }
                                        if !keybind.is_empty() {
                                            ui.monospace(keybind);
                                        }
                                    });
                                }
                            });

                        CollapsingHeader::new("View")
                            .default_open(true)
                            .show(ui, |ui| {
                                let brightness_changed = ui
                                    .add(
                                        egui::Slider::new(&mut self.brightness, -1.0..=1.0)
                                            .text("Brightness"),
                                    )
                                    .changed();
                                let contrast_changed = ui
                                    .add(
                                        egui::Slider::new(&mut self.contrast, 0.1..=3.0)
                                            .text("Contrast"),
                                    )
                                    .changed();
                                ui.label(format!("Zoom: {:.0}%", self.zoom * 100.0));
                                if ui.button("Reset view").clicked() {
                                    self.reset_view(ctx);
                                } else if brightness_changed || contrast_changed {
                                    self.refresh_texture();
                                }
                            });

                        CollapsingHeader::new("Transform")
                            .default_open(true)
                            .show(ui, |ui| {
                                let actions = self.app_keybinds.clone();
                                ui.horizontal(|ui| {
                                    if ui.button("Rotate left").clicked() {
                                        self.rotate_left();
                                    }
                                    ui.monospace(actions.rotate_left.chord);
                                });
                                ui.horizontal(|ui| {
                                    if ui.button("Rotate right").clicked() {
                                        self.rotate_right();
                                    }
                                    ui.monospace(actions.rotate_right.chord);
                                });
                                ui.horizontal(|ui| {
                                    if ui.button("Mirror horizontal").clicked() {
                                        self.toggle_mirror_horizontal();
                                    }
                                    ui.monospace(actions.mirror_horizontal.chord);
                                });
                                ui.horizontal(|ui| {
                                    if ui.button("Mirror vertical").clicked() {
                                        self.toggle_mirror_vertical();
                                    }
                                    ui.monospace(actions.mirror_vertical.chord);
                                });
                                ui.label("Save transformed image");
                                ui.text_edit_singleline(&mut self.transform_save_path_input);
                                ui.horizontal(|ui| {
                                    if ui.button("Save PNG").clicked() {
                                        self.save_transformed_image();
                                    }
                                    ui.monospace(actions.save_transformed_image.chord);
                                });
                            });

                        CollapsingHeader::new("Schema")
                            .default_open(true)
                            .show(ui, |ui| {
                                ui.monospace(
                                    schema::config_dir()
                                        .map(|path| path.display().to_string())
                                        .unwrap_or_else(|_| "<config path unavailable>".to_string()),
                                );
                                let schema_names = self.schema_names.clone();
                                let mut schema_to_load = None;
                                egui::ComboBox::from_label("Active schema")
                                    .selected_text(&self.selected_schema_name)
                                    .show_ui(ui, |ui| {
                                        for name in schema_names {
                                            if ui
                                                .selectable_label(self.selected_schema_name == name, &name)
                                                .clicked()
                                            {
                                                schema_to_load = Some(name);
                                            }
                                        }
                                    });
                                if let Some(name) = schema_to_load {
                                    self.load_schema_by_name(&name);
                                }
                                ui.horizontal(|ui| {
                                    ui.label("Save as");
                                    ui.text_edit_singleline(&mut self.schema_name_input);
                                });
                                ui.horizontal(|ui| {
                                    if ui.button("New schema").clicked() {
                                        self.new_schema();
                                    }
                                    if ui.button("Save schema").clicked() {
                                        self.save_schema();
                                    }
                                    if ui.button("Reload schema").clicked() {
                                        let name = self.selected_schema_name.clone();
                                        self.load_schema_by_name(&name);
                                    }
                                });
                                ui.horizontal(|ui| {
                                    ui.label("Import file");
                                    ui.text_edit_singleline(&mut self.schema_import_path_input);
                                });
                                ui.horizontal(|ui| {
                                    if ui.button("Import schema").clicked() {
                                        self.import_schema();
                                    }
                                });
                            });

                        CollapsingHeader::new("Schema Editor")
                            .default_open(true)
                            .show(ui, |ui| {
                                ui.heading("Labels");
                                let mut remove_index = None;
                                for (index, label) in self.schema_definition.labels.iter_mut().enumerate() {
                                    ui.separator();
                                    ui.horizontal(|ui| {
                                        ui.label(format!("#{}", index + 1));
                                        ui.text_edit_singleline(&mut label.name);
                                        if ui.button("Remove").clicked() {
                                            remove_index = Some(index);
                                        }
                                    });
                                    ui.horizontal(|ui| {
                                        ui.label("Type");
                                        egui::ComboBox::from_id_source(("schema-type", index))
                                            .selected_text(label.label_type.as_str())
                                            .show_ui(ui, |ui| {
                                                ui.selectable_value(
                                                    &mut label.label_type,
                                                    LabelType::Rectangle,
                                                    "rectangle",
                                                );
                                                ui.selectable_value(
                                                    &mut label.label_type,
                                                    LabelType::Polygon,
                                                    "polygon",
                                                );
                                                ui.selectable_value(
                                                    &mut label.label_type,
                                                    LabelType::Global,
                                                    "global",
                                                );
                                            });
                                    });
                                    ui.horizontal(|ui| {
                                        ui.label("Color");
                                        ui.add(
                                            egui::DragValue::new(&mut label.color_rgb[0])
                                                .clamp_range(0..=255)
                                                .speed(1),
                                        );
                                        ui.add(
                                            egui::DragValue::new(&mut label.color_rgb[1])
                                                .clamp_range(0..=255)
                                                .speed(1),
                                        );
                                        ui.add(
                                            egui::DragValue::new(&mut label.color_rgb[2])
                                                .clamp_range(0..=255)
                                                .speed(1),
                                        );
                                        ui.colored_label(rgb(label.color_rgb), "■");
                                    });
                                    ui.horizontal(|ui| {
                                        ui.label("Keybind");
                                        ui.text_edit_singleline(&mut label.keybind.chord);
                                    });
                                }
                                if let Some(index) = remove_index {
                                    self.schema_definition.labels.remove(index);
                                }
                                if ui.button("Add label").clicked() {
                                    self.add_label();
                                }

                                ui.separator();
                                ui.heading("Action Keybinds");
                                show_action_keybind_editor(ui, &mut self.app_keybinds);
                                if ui.button("Save app keybinds").clicked() {
                                    self.save_app_keybinds();
                                }
                            });

                        CollapsingHeader::new("Tools")
                            .default_open(true)
                            .show(ui, |ui| match self.selected_annotation_label() {
                                Some(label) => {
                                    ui.label(format!("Current label: {}", label.name));
                                    ui.label(format!("Shape type: {}", label.label_type.as_str()));
                                    ui.label("Right or middle drag pans");
                                    ui.label("Mouse wheel zooms");
                                    ui.label("Drag a selected vertex to edit an annotation");
                                    match label.label_type {
                                        LabelType::Rectangle => {
                                            ui.label("Primary drag on empty space draws a rectangle");
                                        }
                                        LabelType::Polygon => {
                                            ui.label("Primary click on empty space adds polygon vertices");
                                            ui.label("Press Enter or Finish to close");
                                            if ui.button("Finish polygon").clicked() {
                                                let points = std::mem::take(&mut self.polygon_points);
                                                self.save_shape(Shape::Polygon { points });
                                            }
                                            if ui.button("Cancel polygon").clicked() {
                                                self.polygon_points.clear();
                                                self.status = "Polygon cancelled".to_string();
                                            }
                                        }
                                        LabelType::Global => {}
                                    }
                                }
                                None => {
                                    ui.label("Select a rectangle or polygon label to annotate.");
                                }
                            });

                        CollapsingHeader::new("Annotations")
                            .default_open(true)
                            .show(ui, |ui| {
                                let annotations = self
                                    .annotations
                                    .iter()
                                    .map(|annotation| (annotation.id, annotation.class_name.clone()))
                                    .collect::<Vec<_>>();
                                for (annotation_id, class_name) in annotations {
                                    let selected = self.selected_annotation_id == Some(annotation_id);
                                    let label = format!("#{annotation_id} {class_name}");
                                    if ui.selectable_label(selected, label).clicked() {
                                        self.select_annotation(annotation_id);
                                    }
                                }
                                if ui.button("Delete selected").clicked() {
                                    self.delete_selected_annotation();
                                }
                            });
                    });
            });
    }

    fn show_canvas(&mut self, ctx: &EguiContext) {
        egui::CentralPanel::default().show(ctx, |ui| {
            let Some(texture) = &self.texture else {
                ui.centered_and_justified(|ui| {
                    ui.label("Select a PNG or TIFF from the browser panel.");
                });
                return;
            };
            let Some(image) = &self.image else {
                return;
            };

            let available = ui.available_size();
            let (image_width, image_height) = image.transformed_dimensions(self.image_transform);
            let image_size = vec2(image_width as f32, image_height as f32);
            let (viewport, response) = ui.allocate_exact_size(available, Sense::click_and_drag());
            let painter = ui.painter_at(viewport);
            let fit_scale = (viewport.width() / image_size.x)
                .min(viewport.height() / image_size.y)
                .max(0.01);
            let scale = fit_scale * self.zoom.max(0.05);
            let draw_size = image_size * scale;
            let origin = viewport.center() - draw_size * 0.5 + self.pan;
            let image_rect = Rect::from_min_size(origin, draw_size);

            painter.rect_filled(viewport, 0.0, Color32::from_gray(24));
            painter.image(
                texture.id(),
                image_rect,
                Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                Color32::WHITE,
            );

            self.handle_canvas_input(ctx, &response, viewport, image_rect, fit_scale);
            self.paint_annotations(&painter, image_rect.min, scale);
        });
    }

    fn handle_canvas_input(
        &mut self,
        ctx: &EguiContext,
        response: &egui::Response,
        viewport: Rect,
        image_rect: Rect,
        fit_scale: f32,
    ) {
        let Some(image) = &self.image else {
            return;
        };

        if response.hovered() {
            let pointer_delta = ctx.input(|input| input.pointer.delta());
            let panning =
                ctx.input(|input| input.pointer.secondary_down() || input.pointer.middle_down());
            if panning {
                self.pan += pointer_delta;
                ctx.request_repaint();
            }

            let scroll_delta = ctx.input(|input| input.smooth_scroll_delta.y);
            if scroll_delta.abs() > f32::EPSILON {
                let zoom_factor = (scroll_delta * 0.0015).exp();
                let previous_scale = fit_scale * self.zoom;
                self.zoom = (self.zoom * zoom_factor).clamp(0.1, 32.0);
                let next_scale = fit_scale * self.zoom;

                if let Some(pointer) = ctx.input(|input| input.pointer.hover_pos()) {
                    let (display_width, display_height) =
                        image.transformed_dimensions(self.image_transform);
                    let previous_origin = viewport.center()
                        - vec2(display_width as f32, display_height as f32) * previous_scale * 0.5
                        + self.pan;
                    let display_point =
                        screen_to_image(pointer, previous_origin, previous_scale).unwrap_or(
                            Point::new(display_width as f32 * 0.5, display_height as f32 * 0.5),
                        );
                    let image_point = self.transform_display_to_image(display_point, image);
                    let next_display_point = self.transform_image_to_display(image_point, image);
                    let next_origin = vec2(pointer.x, pointer.y)
                        - vec2(
                            next_display_point.x * next_scale,
                            next_display_point.y * next_scale,
                        );
                    self.pan = next_origin
                        - (viewport.center().to_vec2()
                            - vec2(display_width as f32, display_height as f32) * next_scale * 0.5);
                }
                ctx.request_repaint();
            }
        }

        if ctx.input(|input| input.key_pressed(egui::Key::Enter)) && self.polygon_points.len() >= 3
        {
            if self
                .selected_annotation_label()
                .map(|label| label.label_type)
                == Some(LabelType::Polygon)
            {
                let points = std::mem::take(&mut self.polygon_points);
                self.save_shape(Shape::Polygon { points });
            }
            return;
        }

        let pointer_position = response.interact_pointer_pos();
        let display_point = pointer_position
            .filter(|pointer| image_rect.contains(*pointer))
            .and_then(|pointer| {
                let (display_width, _) = image.transformed_dimensions(self.image_transform);
                screen_to_image(
                    pointer,
                    image_rect.min,
                    image_rect.width() / (display_width as f32).max(f32::EPSILON),
                )
            });
        let image_point = display_point
            .map(|display_point| self.transform_display_to_image(display_point, image))
            .map(|point| clamp_point_to_image(point, image));

        if let Some(point) = image_point {
            if response.drag_started_by(egui::PointerButton::Primary) {
                if let Some((annotation_id, handle_index)) =
                    self.pick_annotation_handle(point, fit_scale * self.zoom)
                {
                    self.begin_annotation_edit(annotation_id, handle_index);
                    return;
                }

                if let Some(annotation_id) = self.pick_annotation(point) {
                    self.select_annotation(annotation_id);
                    return;
                }

                if self
                    .selected_annotation_label()
                    .map(|label| label.label_type)
                    == Some(LabelType::Rectangle)
                {
                    self.rect_start = Some(point);
                    self.rect_current = Some(point);
                    self.polygon_points.clear();
                }
            }

            if self.annotation_edit.is_some() {
                if response.dragged_by(egui::PointerButton::Primary) {
                    self.update_annotation_edit(point);
                    ctx.request_repaint();
                }
                if response.drag_stopped_by(egui::PointerButton::Primary) {
                    self.finish_annotation_edit();
                }
                return;
            }

            if self.rect_start.is_some() && response.dragged_by(egui::PointerButton::Primary) {
                self.rect_current = Some(point);
                ctx.request_repaint();
            }

            if response.drag_stopped_by(egui::PointerButton::Primary) {
                let start = self.rect_start.take();
                let end = self.rect_current.take().or(Some(point));
                if let (Some(start), Some(end)) = (start, end) {
                    self.save_shape(Shape::Rectangle {
                        min: start,
                        max: end,
                    });
                }
                return;
            }

            if response.clicked_by(egui::PointerButton::Primary) {
                if let Some(annotation_id) = self.pick_annotation(point) {
                    self.select_annotation(annotation_id);
                    return;
                }

                if self
                    .selected_annotation_label()
                    .map(|label| label.label_type)
                    == Some(LabelType::Polygon)
                {
                    self.polygon_points.push(point);
                    self.selected_annotation_id = None;
                }
            }
        } else if response.drag_stopped_by(egui::PointerButton::Primary) {
            if self.annotation_edit.is_some() {
                self.finish_annotation_edit();
            }
            self.rect_start = None;
            self.rect_current = None;
        }
    }

    fn transform_image_to_display(&self, point: Point, image: &LoadedImage) -> Point {
        let (x, y) = self.image_transform.apply_to_point(
            (point.x, point.y),
            image.width as f32,
            image.height as f32,
        );
        Point::new(x, y)
    }

    fn transform_display_to_image(&self, point: Point, image: &LoadedImage) -> Point {
        let (x, y) = self.image_transform.invert_point(
            (point.x, point.y),
            image.width as f32,
            image.height as f32,
        );
        Point::new(x, y)
    }

    fn pick_annotation_handle(&self, point: Point, scale: f32) -> Option<(i64, usize)> {
        let max_distance = HANDLE_HIT_RADIUS / scale.max(0.01);
        let mut best: Option<(i64, usize, f32)> = None;

        if let Some(selected_id) = self.selected_annotation_id {
            if let Some(annotation) = self
                .annotations
                .iter()
                .find(|annotation| annotation.id == selected_id)
            {
                for (handle_index, candidate) in annotation.shape.points().iter().enumerate() {
                    let distance = distance(*candidate, point);
                    if distance <= max_distance {
                        return Some((annotation.id, handle_index));
                    }
                }
            }
        }

        for annotation in self.annotations.iter().rev() {
            if Some(annotation.id) == self.selected_annotation_id {
                continue;
            }
            for (handle_index, candidate) in annotation.shape.points().iter().enumerate() {
                let distance = distance(*candidate, point);
                if distance <= max_distance
                    && best
                        .as_ref()
                        .map(|(_, _, current)| distance < *current)
                        .unwrap_or(true)
                {
                    best = Some((annotation.id, handle_index, distance));
                }
            }
        }

        best.map(|(annotation_id, handle_index, _)| (annotation_id, handle_index))
    }

    fn pick_annotation(&self, point: Point) -> Option<i64> {
        if let Some(selected_id) = self.selected_annotation_id {
            if let Some(annotation) = self
                .annotations
                .iter()
                .find(|annotation| annotation.id == selected_id)
            {
                if annotation_contains_point(annotation, point) {
                    return Some(annotation.id);
                }
            }
        }

        self.annotations
            .iter()
            .rev()
            .find(|annotation| {
                Some(annotation.id) != self.selected_annotation_id
                    && annotation_contains_point(annotation, point)
            })
            .map(|annotation| annotation.id)
    }

    fn paint_annotations(&self, painter: &egui::Painter, origin: Pos2, scale: f32) {
        let Some(image) = &self.image else {
            return;
        };

        for annotation in &self.annotations {
            let is_selected = self.selected_annotation_id == Some(annotation.id);
            let color = rgb(annotation.color_rgb);
            let stroke = Stroke::new(if is_selected { 3.0 } else { 2.0 }, color);
            let points = annotation
                .shape
                .points()
                .into_iter()
                .map(|point| self.transform_image_to_display(point, image))
                .map(|point| image_to_screen(point, origin, scale))
                .collect::<Vec<_>>();

            if points.len() >= 2 {
                painter.add(EguiShape::closed_line(points.clone(), stroke));
                for point in &points {
                    painter.circle_filled(*point, if is_selected { 4.0 } else { 3.0 }, color);
                }
                if is_selected {
                    for point in &points {
                        painter.circle_filled(*point, HANDLE_RADIUS + 2.0, Color32::BLACK);
                        painter.circle_filled(*point, HANDLE_RADIUS, Color32::WHITE);
                    }
                }
            }
        }

        if let (Some(start), Some(end)) = (self.rect_start, self.rect_current) {
            let start = self.transform_image_to_display(start, image);
            let end = self.transform_image_to_display(end, image);
            painter.rect_stroke(
                Rect::from_two_pos(
                    image_to_screen(start, origin, scale),
                    image_to_screen(end, origin, scale),
                ),
                0.0,
                Stroke::new(1.0, Color32::YELLOW),
            );
        }

        if !self.polygon_points.is_empty() {
            let points = self
                .polygon_points
                .iter()
                .map(|point| self.transform_image_to_display(*point, image))
                .map(|point| image_to_screen(point, origin, scale))
                .collect::<Vec<_>>();
            painter.add(EguiShape::line(
                points.clone(),
                Stroke::new(2.0, Color32::YELLOW),
            ));
            for point in points {
                painter.circle_filled(point, 3.5, Color32::YELLOW);
            }
        }
    }
}

impl eframe::App for LabelerApp {
    fn update(&mut self, ctx: &EguiContext, _frame: &mut eframe::Frame) {
        self.handle_keybindings(ctx);

        TopBottomPanel::bottom("status_bar").show(ctx, |ui| {
            ui.label(&self.status);
        });

        self.show_left_panel(ctx);
        self.show_canvas(ctx);
    }
}

fn show_action_keybind_editor(ui: &mut egui::Ui, keybinds: &mut AppKeybinds) {
    show_named_keybind(ui, "Previous image", &mut keybinds.previous_image.chord);
    show_named_keybind(ui, "Next image", &mut keybinds.next_image.chord);
    show_named_keybind(ui, "Rotate left", &mut keybinds.rotate_left.chord);
    show_named_keybind(ui, "Rotate right", &mut keybinds.rotate_right.chord);
    show_named_keybind(
        ui,
        "Mirror horizontal",
        &mut keybinds.mirror_horizontal.chord,
    );
    show_named_keybind(ui, "Mirror vertical", &mut keybinds.mirror_vertical.chord);
    show_named_keybind(
        ui,
        "Save transformed image",
        &mut keybinds.save_transformed_image.chord,
    );
}

fn show_named_keybind(ui: &mut egui::Ui, label: &str, value: &mut String) {
    ui.horizontal(|ui| {
        ui.label(label);
        ui.text_edit_singleline(value);
    });
}

fn updated_shape_for_handle(shape: &Shape, handle_index: usize, point: Point) -> Option<Shape> {
    match shape {
        Shape::Rectangle { .. } => {
            let corners = shape.points();
            let opposite = corners[(handle_index + 2) % 4];
            Shape::Rectangle {
                min: opposite,
                max: point,
            }
            .normalized()
        }
        Shape::Polygon { points } => {
            let mut updated = points.clone();
            if handle_index >= updated.len() {
                return None;
            }
            updated[handle_index] = point;
            Shape::Polygon { points: updated }.normalized()
        }
    }
}

fn annotation_contains_point(annotation: &Annotation, point: Point) -> bool {
    match &annotation.shape {
        Shape::Rectangle { min, max } => {
            point.x >= min.x && point.x <= max.x && point.y >= min.y && point.y <= max.y
        }
        Shape::Polygon { points } => point_in_polygon(point, points),
    }
}

fn point_in_polygon(point: Point, polygon: &[Point]) -> bool {
    if polygon.len() < 3 {
        return false;
    }

    let mut inside = false;
    let mut previous = polygon[polygon.len() - 1];
    for current in polygon {
        let crosses = (current.y > point.y) != (previous.y > point.y);
        if crosses {
            let denominator = previous.y - current.y;
            if denominator.abs() <= f32::EPSILON {
                previous = *current;
                continue;
            }
            let x_intersection =
                (previous.x - current.x) * (point.y - current.y) / denominator + current.x;
            if point.x < x_intersection {
                inside = !inside;
            }
        }
        previous = *current;
    }
    inside
}

fn distance(left: Point, right: Point) -> f32 {
    ((left.x - right.x).powi(2) + (left.y - right.y).powi(2)).sqrt()
}

fn shape_matches_annotation_type(shape: &Shape, annotation_type: LabelType) -> bool {
    matches!(
        (shape, annotation_type),
        (Shape::Rectangle { .. }, LabelType::Rectangle)
            | (Shape::Polygon { .. }, LabelType::Polygon)
    )
}

fn screen_to_image(pos: Pos2, origin: Pos2, scale: f32) -> Option<Point> {
    if scale <= 0.0 {
        return None;
    }
    Some(Point::new(
        (pos.x - origin.x) / scale,
        (pos.y - origin.y) / scale,
    ))
}

fn image_to_screen(point: Point, origin: Pos2, scale: f32) -> Pos2 {
    pos2(origin.x + point.x * scale, origin.y + point.y * scale)
}

fn clamp_point_to_image(point: Point, image: &LoadedImage) -> Point {
    Point::new(
        point.x.clamp(0.0, image.width as f32),
        point.y.clamp(0.0, image.height as f32),
    )
}

fn rgb(color: [u8; 3]) -> Color32 {
    Color32::from_rgb(color[0], color[1], color[2])
}

fn deterministic_color(name: &str) -> [u8; 3] {
    let hash = blake3::hash(name.as_bytes());
    let bytes = hash.as_bytes();
    [
        80u8.saturating_add(bytes[0] / 2),
        80u8.saturating_add(bytes[1] / 2),
        80u8.saturating_add(bytes[2] / 2),
    ]
}

fn default_rotated_output_path(path: &Path) -> PathBuf {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let file_name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("image");
    parent.join(format!("{file_name}_rotated.png"))
}

fn navigation_root(browser_dir: &Path, current_path: &Path) -> PathBuf {
    if current_path.starts_with(browser_dir) {
        browser_dir.to_path_buf()
    } else {
        current_path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| browser_dir.to_path_buf())
    }
}

fn collect_supported_images(root: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    collect_supported_images_recursive(root, &mut files);
    files.sort();
    files
}

fn collect_supported_images_recursive(root: &Path, files: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(root) else {
        return;
    };

    let mut paths = entries
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .collect::<Vec<_>>();
    paths.sort();

    for path in paths {
        if path.is_dir() {
            collect_supported_images_recursive(&path, files);
        } else if is_supported_image_path(&path) {
            files.push(path);
        }
    }
}

fn parse_keybind(chord: &str) -> Option<ParsedKeybind> {
    let chord = chord.trim();
    if chord.is_empty() {
        return None;
    }

    let mut modifiers = Modifiers::default();
    let mut key = None;
    for token in chord
        .split('+')
        .map(|token| token.trim().to_ascii_lowercase())
    {
        match token.as_str() {
            "shift" => modifiers.shift = true,
            "ctrl" | "control" => modifiers.ctrl = true,
            "alt" | "option" => modifiers.alt = true,
            "cmd" | "command" | "super" => modifiers.command = true,
            _ => key = parse_key_name(&token),
        }
    }

    Some(ParsedKeybind {
        key: key?,
        modifiers,
    })
}

fn parse_key_name(token: &str) -> Option<Key> {
    match token {
        "a" => Some(Key::A),
        "b" => Some(Key::B),
        "c" => Some(Key::C),
        "d" => Some(Key::D),
        "e" => Some(Key::E),
        "f" => Some(Key::F),
        "g" => Some(Key::G),
        "h" => Some(Key::H),
        "i" => Some(Key::I),
        "j" => Some(Key::J),
        "k" => Some(Key::K),
        "l" => Some(Key::L),
        "m" => Some(Key::M),
        "n" => Some(Key::N),
        "o" => Some(Key::O),
        "p" => Some(Key::P),
        "q" => Some(Key::Q),
        "r" => Some(Key::R),
        "s" => Some(Key::S),
        "t" => Some(Key::T),
        "u" => Some(Key::U),
        "v" => Some(Key::V),
        "w" => Some(Key::W),
        "x" => Some(Key::X),
        "y" => Some(Key::Y),
        "z" => Some(Key::Z),
        "arrowup" | "up" => Some(Key::ArrowUp),
        "arrowdown" | "down" => Some(Key::ArrowDown),
        "arrowleft" | "left" => Some(Key::ArrowLeft),
        "arrowright" | "right" => Some(Key::ArrowRight),
        "space" => Some(Key::Space),
        "enter" | "return" => Some(Key::Enter),
        "tab" => Some(Key::Tab),
        "backspace" => Some(Key::Backspace),
        "escape" | "esc" => Some(Key::Escape),
        _ => None,
    }
}

fn keybind_pressed(ctx: &EguiContext, chord: &str) -> bool {
    let Some(parsed) = parse_keybind(chord) else {
        return false;
    };
    ctx.input(|input| {
        input.key_pressed(parsed.key)
            && input.modifiers.shift == parsed.modifiers.shift
            && input.modifiers.ctrl == parsed.modifiers.ctrl
            && input.modifiers.alt == parsed.modifiers.alt
            && input.modifiers.command == parsed.modifiers.command
    })
}

enum BrowserEntry {
    Directory(PathBuf),
    Image(PathBuf),
}

fn list_browser_entries(path: &PathBuf) -> Result<Vec<BrowserEntry>> {
    let mut directories = Vec::new();
    let mut files = Vec::new();

    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let entry_path = entry.path();
        if entry_path.is_dir() {
            directories.push(BrowserEntry::Directory(entry_path));
            continue;
        }

        if is_supported_image_path(&entry_path) {
            files.push(BrowserEntry::Image(entry_path));
        }
    }

    directories.sort_by(|left, right| browser_entry_name(left).cmp(&browser_entry_name(right)));
    files.sort_by(|left, right| browser_entry_name(left).cmp(&browser_entry_name(right)));
    directories.extend(files);
    Ok(directories)
}

fn is_supported_image_path(path: &Path) -> bool {
    matches!(
        path.extension()
            .and_then(|value| value.to_str())
            .map(|value| value.to_ascii_lowercase())
            .as_deref(),
        Some("png" | "tif" | "tiff")
    )
}

fn browser_entry_name(entry: &BrowserEntry) -> String {
    match entry {
        BrowserEntry::Directory(path) | BrowserEntry::Image(path) => path
            .file_name()
            .map(|value| value.to_string_lossy().to_ascii_lowercase())
            .unwrap_or_else(|| path.display().to_string()),
    }
}
