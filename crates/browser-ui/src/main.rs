use aegis_browser_ui::AegisApp;

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default().with_inner_size([1100.0, 720.0]),
        ..Default::default()
    };

    eframe::run_native(
        "Aegis Browser",
        options,
        Box::new(|_creation_context| Ok(Box::<AegisApp>::default())),
    )
}
