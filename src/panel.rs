use std::cell::RefCell;
use std::rc::Rc;

use gtk4::prelude::*;

use crate::notifications::store::NotificationStore;
use crate::widgets::{
    audio::AudioSection,
    bluetooth::BluetoothSection,
    brightness::BrightnessSection,
    header::HeaderSection,
    media::MediaSection,
    network::NetworkSection,
    notifications::NotificationsSection,
    power::PowerSection,
};

pub struct Panel {
    pub window: gtk4::Window,
    header: HeaderSection,
    notifications: NotificationsSection,
    media: MediaSection,
    audio: AudioSection,
    brightness: BrightnessSection,
    network: NetworkSection,
    bluetooth: BluetoothSection,
    power: PowerSection,
}

impl Panel {
    pub fn new(window: gtk4::Window, store: Rc<RefCell<NotificationStore>>) -> Self {
        let outer_box = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Vertical)
            .valign(gtk4::Align::End)
            .build();
        outer_box.add_css_class("panel-outer");

        let scroll = gtk4::ScrolledWindow::builder()
            .vscrollbar_policy(gtk4::PolicyType::Automatic)
            .hscrollbar_policy(gtk4::PolicyType::Never)
            .propagate_natural_height(true)
            .build();

        let content_box = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Vertical)
            .spacing(8)
            .build();
        content_box.add_css_class("panel-content");

        let header = HeaderSection::new(store.clone());
        let notifications = NotificationsSection::new(store);
        let media = MediaSection::new();
        let audio = AudioSection::new();
        let brightness = BrightnessSection::new();
        let network = NetworkSection::new();
        let bluetooth = BluetoothSection::new();
        let power = PowerSection::new();

        content_box.append(header.widget());
        content_box.append(notifications.widget());
        content_box.append(media.widget());
        content_box.append(audio.widget());
        content_box.append(brightness.widget());
        content_box.append(network.widget());
        content_box.append(bluetooth.widget());
        content_box.append(power.widget());

        scroll.set_child(Some(&content_box));
        outer_box.append(&scroll);
        window.set_child(Some(&outer_box));

        let key_controller = gtk4::EventControllerKey::new();
        {
            let window_clone = window.clone();
            key_controller.connect_key_pressed(move |_, key, _, _| {
                if key == gtk4::gdk::Key::Escape {
                    window_clone.set_visible(false);
                    glib::Propagation::Stop
                } else {
                    glib::Propagation::Proceed
                }
            });
        }
        window.add_controller(key_controller);

        Self {
            window,
            header,
            notifications,
            media,
            audio,
            brightness,
            network,
            bluetooth,
            power,
        }
    }

    pub fn toggle(&self) {
        if self.window.is_visible() {
            self.window.set_visible(false);
        } else {
            self.refresh();
            self.window.set_visible(true);
        }
    }

    pub fn refresh(&self) {
        self.header.refresh();
        self.notifications.refresh();
        self.media.refresh();
        self.audio.refresh();
        self.brightness.refresh();
        self.network.refresh();
        self.bluetooth.refresh();
        self.power.refresh();
    }

    pub fn refresh_audio(&self) {
        self.audio.refresh();
    }

    pub fn refresh_brightness(&self) {
        self.brightness.refresh();
    }
}
