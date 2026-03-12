mod app;
mod db;
mod geometry;
mod image_data;

use app::LabelerApp;

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions::default();
    eframe::run_native(
        "Image Labeler",
        options,
        Box::new(|cc| Box::new(LabelerApp::new(cc).expect("failed to start app"))),
    )
}
