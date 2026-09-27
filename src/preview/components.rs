//! The component sheet: every component in `src/ui/`, in every variant and
//! every state it has, on one card, so a change to a component is judged
//! against its siblings rather than on the one surface that happened to be
//! open.
//!
//! `swaypplet --preview components.<page>` with a page of [`PAGES`]; the
//! harness renders it with `dev/render.sh --mode preview:components.controls`.
//! States that need a pointer or a keyboard (hover, pressed, focus) are
//! forced through GTK's state flags, so each is drawn without input and the
//! sheet is the same on every run.

use gtk4::prelude::*;
use gtk4::{Align, StateFlags};

use crate::ui::{self, Face, Kind, Size, Text, Tone};

/// The pages, each one screen tall at 1440×900.
pub const PAGES: [&str; 4] = ["controls", "inputs", "lists", "tiles"];

/// A state a control can be drawn in without a pointer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    Rest,
    Hover,
    Pressed,
    Focus,
    Disabled,
}

impl State {
    const ALL: [State; 5] = [
        State::Rest,
        State::Hover,
        State::Pressed,
        State::Focus,
        State::Disabled,
    ];

    fn name(self) -> &'static str {
        match self {
            State::Rest => "rest",
            State::Hover => "hover",
            State::Pressed => "pressed",
            State::Focus => "focus",
            State::Disabled => "disabled",
        }
    }

    fn apply(self, w: &impl IsA<gtk4::Widget>) {
        match self {
            State::Rest => {}
            State::Hover => w.set_state_flags(StateFlags::PRELIGHT, false),
            State::Pressed => w.set_state_flags(StateFlags::PRELIGHT | StateFlags::ACTIVE, false),
            State::Focus => {
                w.set_state_flags(StateFlags::FOCUSED | StateFlags::FOCUS_VISIBLE, false)
            }
            State::Disabled => w.set_sensitive(false),
        }
    }
}

/// A widget over its caption.
fn specimen(w: &impl IsA<gtk4::Widget>, caption: &str) -> gtk4::Box {
    let b = ui::vbox(2);
    b.set_valign(Align::Start);
    let holder = ui::hbox(0);
    holder.set_halign(Align::Start);
    holder.append(w);
    // The caption stands under the control, never widening it.
    holder.set_hexpand(false);
    b.append(&holder);
    let c = ui::text(caption, Text::Caption, Tone::Faint);
    b.append(&c);
    b
}

/// One line of the sheet: its name at the left, the specimens after it.
fn line(name: &str, specimens: impl IntoIterator<Item = gtk4::Box>) -> gtk4::Box {
    let row = ui::hbox(5);
    row.set_valign(Align::Start);
    let label = ui::overline(name, Tone::Muted);
    label.set_size_request(120, -1);
    label.set_valign(Align::Start);
    label.set_margin_top(crate::tokens::space(3));
    row.append(&label);
    for s in specimens {
        row.append(&s);
    }
    row
}

/// A control drawn once per state.
fn states<W: IsA<gtk4::Widget>>(name: &str, states: &[State], make: impl Fn() -> W) -> gtk4::Box {
    line(
        name,
        states.iter().map(|s| {
            let w = make();
            s.apply(&w);
            specimen(&w, s.name())
        }),
    )
}

/// A section heading over its lines.
fn block(title: &str, lines: impl IntoIterator<Item = gtk4::Box>) -> gtk4::Box {
    let b = ui::vbox(4);
    b.append(&ui::heading(title));
    for l in lines {
        b.append(&l);
    }
    b
}

/// The sheet's page `page` (one of [`PAGES`]), or the first when unknown.
/// `SWAYPPLET_PREVIEW_ALT=disabled-fade,press-shrink` draws the sheet under
/// the named alternatives of docs/component-zoo.html.
pub fn page(page: &str) -> gtk4::Box {
    let root = ui::vbox(6);
    root.set_hexpand(true);
    for alt in std::env::var("SWAYPPLET_PREVIEW_ALT")
        .unwrap_or_default()
        .split(',')
        .map(str::trim)
    {
        match alt {
            "disabled-fade" => root.add_css_class("alt-disabled-fade"),
            "press-shrink" => root.add_css_class("alt-press-shrink"),
            _ => {}
        }
    }
    match page {
        "inputs" => inputs(&root),
        "lists" => lists(&root),
        "tiles" => tiles(&root),
        _ => controls(&root),
    }
    root
}

// ── Page 1: buttons, chips, badges, text ────────────────────────────────

fn controls(root: &gtk4::Box) {
    let button = |kind: Kind| {
        let label = match kind {
            Kind::Primary => "Connect",
            Kind::Secondary => "Details",
            Kind::Flat => "Cancel",
            Kind::Destructive => "Forget",
        };
        move || ui::button(label, kind)
    };
    let armed = {
        let b = ui::button("Forget", Kind::Destructive);
        ui::set_armed(&b, true);
        specimen(&b, "armed")
    };
    let mut destructive = State::ALL
        .iter()
        .filter(|s| **s != State::Focus)
        .map(|s| {
            let b = ui::button("Forget", Kind::Destructive);
            s.apply(&b);
            specimen(&b, s.name())
        })
        .collect::<Vec<_>>();
    destructive.push(armed);

    let toggles = [
        ("off", false, State::Rest),
        ("on", true, State::Rest),
        ("on · hover", true, State::Hover),
        ("on · pressed", true, State::Pressed),
        ("on · focus", true, State::Focus),
        ("on · disabled", true, State::Disabled),
    ]
    .into_iter()
    .map(|(cap, on, st)| {
        let b = ui::toggle_button(
            Face::Glyph {
                glyph: ui::icons::MIC,
                tooltip: "Microphone",
            },
            Kind::Flat,
            Size::Normal,
        );
        b.set_active(on);
        st.apply(&b);
        specimen(&b, cap)
    });

    let sizes = [
        specimen(&ui::button("Details", Kind::Secondary), "normal · 30"),
        specimen(
            &ui::button_with(Face::Label("Details"), Kind::Secondary, Size::Small),
            "small · 24",
        ),
        specimen(
            &ui::button_with(Face::Label("Connect"), Kind::Primary, Size::Small),
            "small · primary",
        ),
        specimen(
            &ui::button_with(
                Face::Glyph {
                    glyph: ui::icons::CLOSE,
                    tooltip: "Close",
                },
                Kind::Secondary,
                Size::Normal,
            ),
            "icon · square",
        ),
        specimen(
            &ui::button_with(
                Face::Glyph {
                    glyph: ui::icons::CLOSE,
                    tooltip: "Close",
                },
                Kind::Flat,
                Size::Small,
            ),
            "icon · flat · small",
        ),
        {
            let b = ui::button("Shut down", Kind::Secondary);
            b.add_css_class("pill");
            specimen(&b, "pill")
        },
    ];

    root.append(&block(
        "Button",
        [
            states("Secondary", &State::ALL, button(Kind::Secondary)),
            states("Primary", &State::ALL, button(Kind::Primary)),
            states("Flat", &State::ALL, button(Kind::Flat)),
            line("Destructive", destructive),
            line("Toggle · flat", toggles),
            line("Sizes and faces", sizes),
        ],
    ));

    // Chips.
    let chip_states = State::ALL.iter().map(|s| {
        let c = ui::toggle_chip("Personal");
        s.apply(&c);
        specimen(&c, s.name())
    });
    let chip_on = [
        ("checked", State::Rest),
        ("checked · hover", State::Hover),
        ("checked · focus", State::Focus),
        ("checked · disabled", State::Disabled),
    ]
    .into_iter()
    .map(|(cap, s)| {
        let c = ui::toggle_chip("Work");
        c.set_active(true);
        s.apply(&c);
        specimen(&c, cap)
    });
    let rich = |name: &str, picked: bool| {
        let face = ui::hbox(3);
        face.append(&ui::avatar(name, None, 24, true));
        face.append(&ui::text(name, Text::Label, Tone::Fg));
        let c = ui::chip(Face::Child(face.upcast_ref()));
        if picked {
            ui::set_handoff(&c, Some(ui::Handoff::Picked));
        }
        c
    };
    let rich_chips = [
        specimen(&rich("meros", false), "rich"),
        {
            let c = rich("melvin", false);
            State::Hover.apply(&c);
            specimen(&c, "rich · hover")
        },
        specimen(&rich("meros", true), "rich · picked"),
    ];
    let small_pills = [
        specimen(&ui::badge("3", ui::BadgeTone::Alert), "badge · alert"),
        specimen(&ui::badge("12", ui::BadgeTone::Neutral), "badge · neutral"),
        specimen(&ui::key("Super+Space"), "key"),
        specimen(&ui::key("⏎"), "key · glyph"),
        specimen(
            &ui::status(ui::Status::Success, "Connected"),
            "status · success",
        ),
        specimen(
            &ui::status(ui::Status::Warning, "Weak signal"),
            "status · warning",
        ),
        specimen(&ui::status(ui::Status::Danger, "Failed"), "status · danger"),
        specimen(&ui::status(ui::Status::Neutral, "Off"), "status · neutral"),
    ];
    root.append(&block(
        "Chip, badge, key, status",
        [
            line("Toggle chip", chip_states.chain(chip_on)),
            line("Rich chip", rich_chips),
            line("Badge · key · status", small_pills),
        ],
    ));

    // Type and tones.
    let scale = [
        (Text::Display, "36 display"),
        (Text::DisplaySm, "28 display-sm"),
        (Text::Title, "18 title"),
        (Text::TitleSm, "15 title-sm"),
        (Text::Body, "13 body"),
        (Text::Label, "12 label"),
        (Text::Caption, "11 caption"),
    ]
    .into_iter()
    .map(|(t, cap)| specimen(&ui::text("Åsa Öberg", t, Tone::Fg), cap));
    let tones = [
        (Tone::Fg, "fg"),
        (Tone::Muted, "muted"),
        (Tone::Faint, "faint"),
        (Tone::Accent, "accent · 15 strong"),
        (Tone::Success, "success"),
        (Tone::Warning, "warning"),
        (Tone::Danger, "danger"),
    ]
    .into_iter()
    .map(|(tone, cap)| {
        let size = if tone == Tone::Accent {
            Text::TitleSm
        } else {
            Text::Body
        };
        let l = ui::text("Wi-Fi · Öresund", size, tone);
        if tone == Tone::Accent {
            ui::set_weight(&l, ui::Weight::Strong);
        }
        specimen(&l, cap)
    });
    let roles = [
        specimen(&ui::overline("Sender", Tone::Muted), "overline"),
        specimen(&ui::heading("Available networks"), "heading"),
        {
            let l = ui::text("12:45", Text::Title, Tone::Fg);
            ui::set_numeric(&l, true);
            specimen(&l, "numeric")
        },
        {
            let l = ui::text("nixos-rebuild switch", Text::Body, Tone::Fg);
            ui::set_mono(&l, true);
            specimen(&l, "mono")
        },
        {
            let l = gtk4::Label::new(Some(ui::icons::BLUETOOTH));
            ui::glyph::adopt(&l, Text::Title, Tone::Muted);
            specimen(&l, "glyph · title · muted")
        },
        {
            let l = ui::text("Disabled control", Text::Body, Tone::Fg);
            l.set_sensitive(false);
            specimen(&l, "disabled")
        },
    ];
    root.append(&block(
        "Text",
        [
            line("Scale", scale),
            line("Tones", tones),
            line("Roles", roles),
        ],
    ));
}

// ── Page 2: switch, check, slider, field, dropdown, progress ────────────

fn inputs(root: &gtk4::Box) {
    let switches = [
        ("off", false, State::Rest),
        ("off · hover", false, State::Hover),
        ("on", true, State::Rest),
        ("on · hover", true, State::Hover),
        ("on · focus", true, State::Focus),
        ("off · disabled", false, State::Disabled),
        ("on · disabled", true, State::Disabled),
    ]
    .into_iter()
    .map(|(cap, on, st)| {
        let s = ui::switch();
        s.set_active(on);
        st.apply(&s);
        specimen(&s, cap)
    });
    let checks = [
        ("off", false, State::Rest),
        ("off · hover", false, State::Hover),
        ("on", true, State::Rest),
        ("on · hover", true, State::Hover),
        ("on · focus", true, State::Focus),
        ("off · disabled", false, State::Disabled),
        ("on · disabled", true, State::Disabled),
    ]
    .into_iter()
    .map(|(cap, on, st)| {
        let c = ui::check("Fill the card");
        c.set_active(on);
        st.apply(&c);
        specimen(&c, cap)
    });
    root.append(&block(
        "Switch and check",
        [line("Switch", switches), line("Check", checks)],
    ));

    // Sliders: each in its row, at a fixed width.
    let slider = |value: f64, over: bool, st: State, dense: bool| {
        let r = ui::slider_row(
            ui::icons::SPEAKER_MED,
            0.0,
            if over { 150.0 } else { 100.0 },
            1.0,
        );
        r.scale.set_value(value);
        r.value.set_text(&format!("{value:.0}%"));
        r.root.set_size_request(220, -1);
        if over {
            ui::set_over_range(&r.scale, true);
        }
        if dense {
            r.scale.add_css_class("dense");
        }
        st.apply(&r.scale);
        r.root
    };
    let sliders = [
        specimen(&slider(64.0, false, State::Rest, false), "rest"),
        specimen(&slider(64.0, false, State::Hover, false), "hover"),
        specimen(&slider(64.0, false, State::Focus, false), "focus"),
        specimen(&slider(64.0, false, State::Disabled, false), "disabled"),
        specimen(&slider(120.0, true, State::Rest, false), "over range"),
    ];
    let dense = [
        specimen(&slider(30.0, false, State::Rest, true), "dense · rest"),
        specimen(&slider(30.0, false, State::Hover, true), "dense · hover"),
        specimen(&slider(30.0, false, State::Focus, true), "dense · focus"),
    ];
    root.append(&block(
        "Slider",
        [line("Slider row", sliders), line("Dense", dense)],
    ));

    // Fields.
    let entry = |text: &str, placeholder: &str, st: State, field_state: Option<ui::FieldState>| {
        let e = gtk4::Entry::new();
        e.set_placeholder_text(Some(placeholder));
        e.set_text(text);
        e.set_size_request(180, -1);
        let f = ui::field("", &e);
        if let Some(fs) = field_state {
            ui::set_field_state(&f.root, fs, true);
        }
        if st == State::Focus {
            e.set_state_flags(StateFlags::FOCUSED | StateFlags::FOCUS_WITHIN, false);
        } else {
            st.apply(&e);
        }
        f.root
    };
    let entries = [
        specimen(
            &entry("", "Search", State::Rest, None),
            "rest · placeholder",
        ),
        specimen(&entry("", "Search", State::Hover, None), "hover"),
        specimen(&entry("Öresund", "", State::Focus, None), "focus"),
        specimen(&entry("Öresund", "", State::Disabled, None), "disabled"),
    ];
    let auth = [
        specimen(
            &entry("", "Password", State::Rest, Some(ui::FieldState::Armed)),
            "armed · pulses",
        ),
        specimen(
            &entry("••••••", "", State::Rest, Some(ui::FieldState::Busy)),
            "busy",
        ),
        specimen(
            &entry("••••••", "", State::Rest, Some(ui::FieldState::Reject)),
            "reject · flash",
        ),
        {
            let e = gtk4::Entry::new();
            e.set_placeholder_text(Some("Search apps, settings, files"));
            e.set_size_request(320, -1);
            ui::entry::adopt(&e, ui::FieldSize::Large);
            specimen(&e, "large · 40")
        },
    ];
    let labelled = {
        let e = gtk4::Entry::new();
        e.set_text("meros");
        e.set_size_request(200, -1);
        let f = ui::field("Username", &e);
        specimen(&f.root, "labelled field")
    };
    let area = {
        let (scroller, view) = ui::text_area(2);
        view.buffer()
            .set_text("Describe what went wrong.\nÅ, ä and ö stay.");
        scroller.set_size_request(320, -1);
        specimen(&scroller, "text area · 2 lines")
    };
    let dropdowns = State::ALL
        .iter()
        .filter(|s| **s != State::Pressed)
        .map(|s| {
            let d = ui::dropdown(&["Auto: light by day", "Dark", "Light"]);
            d.set_size_request(200, -1);
            s.apply(&d);
            specimen(&d, &format!("dropdown · {}", s.name()))
        });
    root.append(&block(
        "Field and dropdown",
        [
            line("Entry", entries),
            line("Auth states", auth),
            line("Field · area", [labelled, area]),
            line("Dropdown", dropdowns),
        ],
    ));

    // Progress.
    let progress = |fraction: f64, status: Option<ui::Status>, breathing: bool, cap: &str| {
        let p = ui::progress(fraction);
        p.set_size_request(180, -1);
        p.set_valign(Align::Center);
        ui::set_progress_status(&p, status);
        ui::set_breathing(&p, breathing);
        specimen(&p, cap)
    };
    root.append(&block(
        "Progress",
        [line(
            "Level",
            [
                progress(0.64, None, false, "plain"),
                progress(0.9, Some(ui::Status::Success), false, "success"),
                progress(0.2, Some(ui::Status::Warning), false, "warning"),
                progress(0.08, Some(ui::Status::Danger), false, "danger"),
                progress(0.5, None, true, "breathing"),
            ],
        )],
    ));
}

// ── Page 3: rows, lists, sections, tiles, menus, cards, avatars ─────────

fn lists(root: &gtk4::Box) {
    // Rows.
    let row_button = |st: State, selected: bool, busy: bool| {
        let (b, r) = ui::row_button(ui::icons::HEADPHONES, "WH-1000XM5", "Connected · 72 %");
        r.end.append(&ui::text("A2DP", Text::Label, Tone::Faint));
        b.set_size_request(210, -1);
        ui::set_selected(&b, selected);
        ui::set_busy(&b, busy);
        st.apply(&b);
        b
    };
    let rows = [
        specimen(&row_button(State::Rest, false, false), "rest"),
        specimen(&row_button(State::Hover, false, false), "hover"),
        specimen(&row_button(State::Pressed, false, false), "pressed"),
        specimen(&row_button(State::Focus, false, false), "focus"),
        specimen(&row_button(State::Rest, true, false), "selected"),
    ];
    let busy = specimen(&row_button(State::Rest, false, true), "busy");
    let list = {
        let l = ui::list();
        l.set_size_request(280, -1);
        l.set_selection_mode(gtk4::SelectionMode::Single);
        for (i, (icon, title, sub)) in [
            (ui::icons::BLUETOOTH, "Bluetooth", "On · 2 connected"),
            (ui::icons::DISPLAY, "Displays", "Laptop + LG 28H2U"),
            (ui::icons::KEYBOARD, "Input", "Swedish · Caps as Escape"),
        ]
        .into_iter()
        .enumerate()
        {
            let r = ui::row(icon, title, sub);
            let lr = ui::list_row(&r.root);
            l.append(&lr);
            if i == 1 {
                l.select_row(Some(&lr));
            }
            if i == 2 {
                lr.set_state_flags(StateFlags::PRELIGHT, false);
            }
        }
        specimen(&l, "list · one selected, one hovered")
    };
    let switch_row = {
        let (r, s) = ui::switch_row("Night light", "Sun · 3500 K");
        s.set_active(true);
        r.root.set_size_request(260, -1);
        specimen(&r.root, "switch row")
    };
    root.append(&block(
        "Row and list",
        [
            line("Row button", rows),
            line("List", [list, switch_row, busy]),
        ],
    ));

    // Sections and disclosures.
    let section = |open: bool| {
        let s = ui::section(ui::icons::BLUETOOTH, "Bluetooth", "2 connected");
        s.body
            .append(&ui::text("Body content", Text::Body, Tone::Muted));
        ui::pad(&s.body, 3);
        s.set_open(open);
        s.root.set_size_request(280, -1);
        s.root.clone()
    };
    let disclosure = |open: bool, st: State| {
        let d = ui::disclosure("More options");
        d.body
            .append(&ui::text("Hidden until opened", Text::Label, Tone::Muted));
        d.set_open(open);
        st.apply(&d.header);
        d.root.set_size_request(200, -1);
        d.root
    };
    root.append(&block(
        "Expander",
        [
            line(
                "Section",
                [
                    specimen(&section(false), "closed"),
                    specimen(&section(true), "open"),
                ],
            ),
            line(
                "Disclosure",
                [
                    specimen(&disclosure(false, State::Rest), "closed"),
                    specimen(&disclosure(false, State::Hover), "hover"),
                    specimen(&disclosure(true, State::Rest), "open"),
                ],
            ),
        ],
    ));

    // Menu, cards, avatars.
    let menu = {
        let card = ui::vbox(0);
        ui::card::adopt(&card, ui::Card::Solid);
        let m = ui::menu();
        m.set_size_request(220, -1);
        let items = [
            ("Open", "Enter", false, State::Rest),
            ("Pin", "P", false, State::Hover),
            ("Rename", "F2", false, State::Pressed),
            ("Forget", "Del", true, State::Rest),
        ];
        for (label, accel, danger, st) in items {
            let i = ui::menu_item(label, accel, danger);
            st.apply(&i);
            m.append(&i);
        }
        card.append(&m);
        specimen(&card, "menu on a solid card")
    };
    let group = {
        let g = ui::group(2);
        ui::pad(&g, 3);
        g.set_size_request(200, -1);
        g.append(&ui::text("A group fill", Text::Body, Tone::Fg));
        g.append(&ui::text("fill-1 on the glass", Text::Label, Tone::Muted));
        specimen(&g, "group")
    };
    let well = {
        let w = ui::well();
        ui::pad(&w, 3);
        w.set_size_request(200, -1);
        let l = ui::text("sudo nixos-rebuild switch", Text::Label, Tone::Fg);
        ui::set_mono(&l, true);
        w.append(&l);
        specimen(&w, "well")
    };
    let pill_group = {
        let g = ui::pill_group(1);
        let b = ui::button_with(
            Face::Glyph {
                glyph: ui::icons::SPEAKER_MED,
                tooltip: "Mute",
            },
            Kind::Flat,
            Size::Normal,
        );
        b.add_css_class("pill");
        g.append(&b);
        let s = gtk4::Scale::with_range(gtk4::Orientation::Horizontal, 0.0, 100.0, 1.0);
        s.set_value(64.0);
        s.set_size_request(120, -1);
        ui::slider::adopt(&s, ui::Density::Dense);
        g.append(&s);
        specimen(&g, "pill group")
    };
    let avatars = {
        let b = ui::hbox(4);
        b.set_valign(Align::Center);
        b.append(&ui::avatar("meros", None, 40, true));
        let current = ui::avatar("melvin", None, 40, false);
        current.add_css_class("active");
        b.append(&current);
        b.append(&ui::avatar("åsa", None, 32, true));
        b.append(&ui::avatar("ola", None, 24, false));
        specimen(&b, "avatar · presence · current · sizes")
    };
    root.append(&block(
        "Menu, card fills, avatar",
        [line(
            "Menu · fills",
            [menu, group, well, pill_group, avatars],
        )],
    ));
}

// ── Page 4: tiles ───────────────────────────────────────────────────────

fn tiles(root: &gtk4::Box) {
    let tile = |on: bool, st: State, loading: bool| {
        let t = ui::tile_toggle(ui::icons::BLUETOOTH, "Bluetooth");
        t.set_active(on);
        t.set_size_request(130, -1);
        ui::set_loading(&t, loading);
        st.apply(&t);
        t
    };
    let tiles = [
        specimen(&tile(false, State::Rest, false), "off"),
        specimen(&tile(false, State::Hover, false), "off · hover"),
        specimen(&tile(true, State::Rest, false), "on"),
        specimen(&tile(true, State::Hover, false), "on · hover"),
        specimen(&tile(true, State::Pressed, false), "on · pressed"),
        specimen(&tile(true, State::Focus, false), "on · focus"),
        specimen(&tile(false, State::Disabled, false), "disabled"),
        specimen(&tile(false, State::Rest, true), "loading"),
    ];
    let split = |on: bool, status: &str, st: State, detail_open: bool| {
        let t = ui::tile_split(ui::icons::NOTIFICATION, "Do not disturb");
        t.toggle.set_active(on);
        ui::set_tile_status(&t, status);
        t.root.set_size_request(200, -1);
        st.apply(&t.toggle);
        if detail_open {
            t.detail.add_css_class("open");
            State::Hover.apply(&t.detail);
        }
        t.root
    };
    let splits = [
        specimen(&split(false, "", State::Rest, false), "off"),
        specimen(
            &split(true, "Until 07:00", State::Rest, false),
            "on · status",
        ),
        specimen(
            &split(true, "Until 07:00", State::Hover, false),
            "on · body hover",
        ),
        specimen(
            &split(true, "Until 07:00", State::Focus, false),
            "on · body focus",
        ),
        specimen(&split(false, "", State::Rest, true), "detail open"),
    ];
    root.append(&block(
        "Tile",
        [line("Toggle tile", tiles), line("Split tile", splits)],
    ));
}
