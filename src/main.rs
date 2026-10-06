//! DevTerm on GPUI. The first screen is the terminal, same as the Electron app.

fn main() {
    gpui::Application::new()
        .with_assets(devterm_gpui::icons::IconAssets)
        .run(|cx| {
            devterm_gpui::app::open(cx);
        });
}
