use crate::{
    db::Database,
    geometry::{Annotation, LabelClass, Point, Shape},
    image_data::{load_png, LoadedImage},
};
use anyhow::{Context, Result};
use eframe::egui::{
    self, pos2, vec2, Color32, Context as EguiContext, Pos2, Rect, Sense, Shape as EguiShape,
    Stroke, TextureHandle, TextureOptions, TopBottomPanel, Vec2,
};
use std::{fs, path::PathBuf};

const DB_PATH: &str = "labels.sqlite3";

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tool {
    Rectangle,
    Polygon,
}

pub struct LabelerApp {
    db: Database,
    classes: Vec<LabelClass>,
    selected_class_id: Option<i64>,
    annotations: Vec<Annotation>,
    image: Option<LoadedImage>,
    texture: Option<TextureHandle>,
    browser_dir: PathBuf,
    image_path_input: String,
    new_class_name: String,
    status: String,
    tool: Tool,
    rect_start: Option<Point>,
    rect_current: Option<Point>,
    polygon_points: Vec<Point>,
    selected_annotation_id: Option<i64>,
    zoom: f32,
    pan: Vec2,
    brightness: f32,
    contrast: f32,
}

impl LabelerApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Result<Self> {
        let db = Database::open(&PathBuf::from(DB_PATH))?;
        let classes = db.list_classes()?;
        let selected_class_id = classes.first().map(|class| class.id);
        let browser_dir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));

        cc.egui_ctx.set_pixels_per_point(1.25);

        Ok(Self {
            db,
            classes,
            selected_class_id,
            annotations: Vec::new(),
            image: None,
            texture: None,
            browser_dir,
            image_path_input: String::new(),
            new_class_name: String::new(),
            status: format!("Database: {}", DB_PATH),
            tool: Tool::Rectangle,
            rect_start: None,
            rect_current: None,
            polygon_points: Vec::new(),
            selected_annotation_id: None,
            zoom: 1.0,
            pan: Vec2::ZERO,
            brightness: 0.0,
            contrast: 1.0,
        })
    }

    fn load_image(&mut self, ctx: &EguiContext) {
        match self.try_load_image(ctx) {
            Ok(()) => {}
            Err(error) => self.status = format!("Load failed: {error:#}"),
        }
    }

    fn try_load_image(&mut self, ctx: &EguiContext) -> Result<()> {
        let path = PathBuf::from(self.image_path_input.trim());
        let image = load_png(&path)?;
        self.db.upsert_image(
            &image.hash,
            image.width,
            image.height,
            image.bit_depth,
            &image.path,
        )?;
        let annotations = self.db.list_annotations(&image.hash)?;
        let display = image.adjusted_display(self.brightness, self.contrast);
        let texture = ctx.load_texture(format!("image:{}", image.hash), display, TextureOptions::NEAREST);

        self.status = format!(
            "Loaded {} ({}x{}, {}-bit, hash {})",
            image.path, image.width, image.height, image.bit_depth, image.hash
        );
        self.image = Some(image);
        self.texture = Some(texture);
        self.annotations = annotations;
        self.rect_start = None;
        self.rect_current = None;
        self.polygon_points.clear();
        self.selected_annotation_id = None;
        self.reset_view(ctx);
        Ok(())
    }

    fn refresh_texture(&mut self) {
        if let (Some(image), Some(texture)) = (&self.image, &mut self.texture) {
            texture.set(image.adjusted_display(self.brightness, self.contrast), TextureOptions::NEAREST);
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

    fn add_class(&mut self) {
        if self.new_class_name.trim().is_empty() {
            self.status = "Class name cannot be empty".to_string();
            return;
        }

        let name = self.new_class_name.trim().to_string();
        let color = deterministic_color(&name);
        match self.db.add_class(&name, color) {
            Ok(class_id) => {
                self.classes.push(LabelClass {
                    id: class_id,
                    name: name.clone(),
                    color_rgb: color,
                });
                self.classes.sort_by(|a, b| a.name.cmp(&b.name));
                self.selected_class_id = Some(class_id);
                self.new_class_name.clear();
                self.status = format!("Added class `{name}`");
            }
            Err(error) => self.status = format!("Failed to add class: {error:#}"),
        }
    }

    fn save_shape(&mut self, shape: Shape) {
        let Some(image) = &self.image else {
            self.status = "Load an image first".to_string();
            return;
        };
        let Some(class_id) = self.selected_class_id else {
            self.status = "Select a class before creating labels".to_string();
            return;
        };
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
                self.status = format!("Deleted annotation {annotation_id}");
            }
            Err(error) => self.status = format!("Delete failed: {error:#}"),
        }
    }

    fn show_left_panel(&mut self, ctx: &EguiContext) {
        egui::SidePanel::left("side_panel")
            .resizable(true)
            .default_width(320.0)
            .show(ctx, |ui| {
                ui.heading("Image");
                ui.label("Selected path");
                ui.add_enabled(false, egui::TextEdit::singleline(&mut self.image_path_input));
                if let Some(image) = &self.image {
                    ui.separator();
                    ui.label(format!("Size: {} x {}", image.width, image.height));
                    ui.label(format!("Bit depth: {}", image.bit_depth));
                    ui.label("Raw-pixel hash");
                    ui.monospace(&image.hash);
                }

                ui.separator();
                ui.heading("Browser");
                ui.horizontal(|ui| {
                    if ui.button("Up").clicked() {
                        if let Some(parent) = self.browser_dir.parent() {
                            self.browser_dir = parent.to_path_buf();
                        }
                    }
                    if ui.button("Reload").clicked() {
                        ctx.request_repaint();
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
                                            .map(|value| value.to_string_lossy().into_owned())
                                            .unwrap_or_else(|| path.display().to_string());
                                        if ui.selectable_label(false, format!("[{name}]")).clicked() {
                                            self.browser_dir = path;
                                        }
                                    }
                                    BrowserEntry::Png(path) => {
                                        let name = path
                                            .file_name()
                                            .map(|value| value.to_string_lossy().into_owned())
                                            .unwrap_or_else(|| path.display().to_string());
                                        if ui.selectable_label(false, name).clicked() {
                                            selected_file = Some(path);
                                        }
                                    }
                                }
                            }
                        }
                        Err(error) => {
                            ui.colored_label(Color32::RED, format!("Browser error: {error}"));
                        }
                    });

                if let Some(path) = selected_file {
                    self.image_path_input = path.display().to_string();
                    self.load_image(ctx);
                }

                ui.separator();
                ui.heading("View");
                let brightness_changed = ui
                    .add(egui::Slider::new(&mut self.brightness, -1.0..=1.0).text("Brightness"))
                    .changed();
                let contrast_changed = ui
                    .add(egui::Slider::new(&mut self.contrast, 0.1..=3.0).text("Contrast"))
                    .changed();
                ui.label(format!("Zoom: {:.0}%", self.zoom * 100.0));
                if ui.button("Reset view").clicked() {
                    self.reset_view(ctx);
                } else if brightness_changed || contrast_changed {
                    self.refresh_texture();
                }

                ui.separator();
                ui.heading("Classes");
                for class in &self.classes {
                    let color = rgb(class.color_rgb);
                    let selected = self.selected_class_id == Some(class.id);
                    ui.horizontal(|ui| {
                        ui.colored_label(color, "■");
                        if ui.selectable_label(selected, &class.name).clicked() {
                            self.selected_class_id = Some(class.id);
                        }
                    });
                }

                ui.horizontal(|ui| {
                    ui.text_edit_singleline(&mut self.new_class_name);
                    if ui.button("Add").clicked() {
                        self.add_class();
                    }
                });

                ui.separator();
                ui.heading("Tools");
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut self.tool, Tool::Rectangle, "Rectangle");
                    ui.selectable_value(&mut self.tool, Tool::Polygon, "Polygon");
                });
                ui.label("Primary drag draws");
                ui.label("Right or middle drag pans");
                ui.label("Mouse wheel zooms");
                if self.tool == Tool::Polygon {
                    ui.label("Click to add vertices");
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

                ui.separator();
                ui.heading("Annotations");
                for annotation in &self.annotations {
                    let selected = self.selected_annotation_id == Some(annotation.id);
                    let label = format!("#{} {}", annotation.id, annotation.class_name);
                    if ui.selectable_label(selected, label).clicked() {
                        self.selected_annotation_id = Some(annotation.id);
                    }
                }
                if ui.button("Delete selected").clicked() {
                    self.delete_selected_annotation();
                }
            });
    }

    fn show_canvas(&mut self, ctx: &EguiContext) {
        egui::CentralPanel::default().show(ctx, |ui| {
            let Some(texture) = &self.texture else {
                ui.centered_and_justified(|ui| {
                    ui.label("Select a PNG from the browser panel.");
                });
                return;
            };
            let Some(image) = &self.image else {
                return;
            };

            let available = ui.available_size();
            let image_size = vec2(image.width as f32, image.height as f32);
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
            let panning = ctx.input(|input| input.pointer.secondary_down() || input.pointer.middle_down());
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
                    let previous_origin =
                        viewport.center() - vec2(image.width as f32, image.height as f32) * previous_scale * 0.5
                            + self.pan;
                    let image_point = screen_to_image(pointer, previous_origin, previous_scale)
                        .unwrap_or(Point::new(image.width as f32 * 0.5, image.height as f32 * 0.5));
                    let next_origin = vec2(pointer.x, pointer.y) - vec2(image_point.x * next_scale, image_point.y * next_scale);
                    self.pan = next_origin - (viewport.center().to_vec2()
                        - vec2(image.width as f32, image.height as f32) * next_scale * 0.5);
                }
                ctx.request_repaint();
            }
        }

        if ctx.input(|input| input.key_pressed(egui::Key::Enter)) && self.polygon_points.len() >= 3 {
            let points = std::mem::take(&mut self.polygon_points);
            self.save_shape(Shape::Polygon { points });
            return;
        }

        let pointer_position = response.interact_pointer_pos();
        let image_point = pointer_position
            .filter(|pointer| image_rect.contains(*pointer))
            .and_then(|pointer| screen_to_image(pointer, image_rect.min, image_rect.width() / image.width as f32));

        match self.tool {
            Tool::Rectangle => {
                if response.drag_started_by(egui::PointerButton::Primary) {
                    self.rect_start = image_point.map(|point| clamp_point_to_image(point, image));
                    self.rect_current = self.rect_start;
                }
                if response.dragged_by(egui::PointerButton::Primary) {
                    self.rect_current = image_point.map(|point| clamp_point_to_image(point, image));
                }
                if response.drag_stopped_by(egui::PointerButton::Primary) {
                    let start = self.rect_start.take();
                    let end = self
                        .rect_current
                        .take()
                        .or_else(|| image_point.map(|point| clamp_point_to_image(point, image)));
                    if let (Some(start), Some(end)) = (start, end) {
                        self.save_shape(Shape::Rectangle { min: start, max: end });
                    }
                }
            }
            Tool::Polygon => {
                if response.clicked_by(egui::PointerButton::Primary) {
                    if let Some(point) = image_point.map(|point| clamp_point_to_image(point, image)) {
                        self.polygon_points.push(point);
                    }
                }
            }
        }
    }

    fn paint_annotations(&self, painter: &egui::Painter, origin: Pos2, scale: f32) {
        for annotation in &self.annotations {
            let is_selected = self.selected_annotation_id == Some(annotation.id);
            let color = rgb(annotation.color_rgb);
            let stroke = Stroke::new(if is_selected { 3.0 } else { 2.0 }, color);
            let points = annotation
                .shape
                .points()
                .into_iter()
                .map(|point| image_to_screen(point, origin, scale))
                .collect::<Vec<_>>();

            if points.len() >= 2 {
                painter.add(EguiShape::closed_line(points.clone(), stroke));
                for point in points {
                    painter.circle_filled(point, if is_selected { 4.0 } else { 3.0 }, color);
                }
            }
        }

        if let (Some(start), Some(end)) = (self.rect_start, self.rect_current) {
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
                .map(|point| image_to_screen(*point, origin, scale))
                .collect::<Vec<_>>();
            painter.add(EguiShape::line(points.clone(), Stroke::new(2.0, Color32::YELLOW)));
            for point in points {
                painter.circle_filled(point, 3.5, Color32::YELLOW);
            }
        }
    }
}

impl eframe::App for LabelerApp {
    fn update(&mut self, ctx: &EguiContext, _frame: &mut eframe::Frame) {
        TopBottomPanel::bottom("status_bar").show(ctx, |ui| {
            ui.label(&self.status);
        });

        self.show_left_panel(ctx);
        self.show_canvas(ctx);
    }
}

fn screen_to_image(pos: Pos2, origin: Pos2, scale: f32) -> Option<Point> {
    if scale <= 0.0 {
        return None;
    }
    Some(Point::new((pos.x - origin.x) / scale, (pos.y - origin.y) / scale))
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

enum BrowserEntry {
    Directory(PathBuf),
    Png(PathBuf),
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

        let extension = entry_path
            .extension()
            .map(|value| value.to_string_lossy().to_ascii_lowercase());
        if extension.as_deref() == Some("png") {
            files.push(BrowserEntry::Png(entry_path));
        }
    }

    directories.sort_by(|left, right| browser_entry_name(left).cmp(&browser_entry_name(right)));
    files.sort_by(|left, right| browser_entry_name(left).cmp(&browser_entry_name(right)));
    directories.extend(files);
    Ok(directories)
}

fn browser_entry_name(entry: &BrowserEntry) -> String {
    match entry {
        BrowserEntry::Directory(path) | BrowserEntry::Png(path) => path
            .file_name()
            .map(|value| value.to_string_lossy().to_ascii_lowercase())
            .unwrap_or_else(|| path.display().to_string()),
    }
}
