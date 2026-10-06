//! DevTerm on GPUI. The first screen is the terminal, same as the Electron app.

fn main() {
    gpui::Application::new().run(|cx| {
        devterm_gpui::app::open(cx);
    });
}
