mod app;
mod osd;
mod panel;
mod theme;
mod widgets;

fn main() {
    env_logger::init();
    app::run();
}
