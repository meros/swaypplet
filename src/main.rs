mod app;
mod panel;
mod theme;
mod widgets;

fn main() {
    env_logger::init();
    app::run();
}
