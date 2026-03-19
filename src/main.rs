mod app;
mod elephant;
mod icons;
mod launcher;
mod layer_shell;
mod notifications;
mod osd;
mod panel;
mod spawn;
mod theme;
mod widgets;

fn main() {
    env_logger::init();
    app::run();
}
