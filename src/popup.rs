use crate::cache::CacheState;
use crate::config::Config;
use crate::mock::mock_snapshots;
use crate::models::{Provider, UsageSnapshot};
use crate::pace::{self, UsagePace};
use anyhow::Result;
use chrono::Utc;
use gtk4::gdk::Display;
use gtk4::gdk_pixbuf::{Colorspace, Pixbuf};
use gtk4::prelude::*;
use gtk4::{
    Align, Application, ApplicationWindow, Box as GtkBox, CssProvider, Image, Label, LinkButton,
    Orientation, ProgressBar,
};
use gtk4_layer_shell::{Edge, Layer, LayerShell};
use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

const APP_ID: &str = "com.quotabar.popup";

pub fn run(use_mock: bool) -> Result<()> {
    let app = Application::builder().application_id(APP_ID).build();
    let window_state: Rc<RefCell<Option<ApplicationWindow>>> = Rc::new(RefCell::new(None));

    app.connect_activate(move |app| {
        if let Some(window) = window_state.borrow().as_ref() {
            if window.is_visible() {
                window.close();
                app.quit();
                return;
            }
        }

        let (snapshots, errors) = if use_mock {
            (mock_snapshots(), HashMap::new())
        } else {
            CacheState::load()
                .ok()
                .flatten()
                .map(|c| (c.snapshots, c.errors))
                .unwrap_or_default()
        };

        let window = build_ui(app, snapshots, errors, use_mock);
        *window_state.borrow_mut() = Some(window);
    });

    app.run_with_args::<&str>(&[]);
    Ok(())
}

fn build_ui(
    app: &Application,
    snapshots: HashMap<Provider, UsageSnapshot>,
    errors: HashMap<Provider, String>,
    use_mock: bool,
) -> ApplicationWindow {
    let window = ApplicationWindow::builder()
        .application(app)
        .default_width(320)
        .default_height(400)
        .build();
    let app_clone = app.clone();
    window.connect_close_request(move |_| {
        app_clone.quit();
        gtk4::glib::Propagation::Proceed
    });

    // Layer shell setup
    window.init_layer_shell();
    window.set_layer(Layer::Overlay);
    window.set_anchor(Edge::Top, true);
    window.set_anchor(Edge::Right, true);
    window.set_margin(Edge::Top, 40);
    window.set_margin(Edge::Right, 10);
    window.set_keyboard_mode(gtk4_layer_shell::KeyboardMode::OnDemand);

    // Load config
    let config = Config::load().unwrap_or_default();

    // Load CSS
    let css_watcher = load_css(use_mock, &config.general.theme);

    // Main container
    let main_box = GtkBox::new(Orientation::Vertical, 0);
    main_box.add_css_class("popup-container");

    let selected_provider = config.general.selected_provider;
    let selected_state: Rc<RefCell<Option<Provider>>> = Rc::new(RefCell::new(selected_provider));
    let sections: Rc<RefCell<Vec<(Provider, GtkBox)>>> = Rc::new(RefCell::new(Vec::new()));

    // Provider sections
    let providers = [Provider::Claude, Provider::Codex, Provider::OpenCode];
    for provider in providers {
        if let Some(snapshot) = snapshots.get(&provider) {
            let section =
                create_provider_section(snapshot, errors.get(&provider).map(|s| s.as_str()));
            if Some(snapshot.provider) == selected_provider {
                section.add_css_class("selected");
            }
            sections
                .borrow_mut()
                .push((snapshot.provider, section.clone()));

            let section_provider = snapshot.provider;
            let sections_clone = Rc::clone(&sections);
            let selected_state = Rc::clone(&selected_state);
            let window_clone = window.clone();
            let click_controller = gtk4::GestureClick::new();
            click_controller.connect_released(move |_, _, _, _| {
                let mut current = selected_state.borrow_mut();
                if *current == Some(section_provider) {
                    window_clone.close();
                    return;
                }
                if let Ok(mut config) = Config::load() {
                    config.general.selected_provider = Some(section_provider);
                    let _ = config.save();
                }
                *current = Some(section_provider);
                for (provider, section) in sections_clone.borrow().iter() {
                    if *provider == section_provider {
                        section.add_css_class("selected");
                    } else {
                        section.remove_css_class("selected");
                    }
                }
            });
            section.add_controller(click_controller);
            main_box.append(&section);
        } else if let Some(error) = errors.get(&provider) {
            let section = create_provider_error_section(&provider, error);
            main_box.append(&section);
        }
    }

    // Footer with last update time
    let footer = create_footer(&snapshots);
    main_box.append(&footer);

    window.set_child(Some(&main_box));

    // Close on Escape or click outside
    let window_clone = window.clone();
    let key_controller = gtk4::EventControllerKey::new();
    key_controller.connect_key_pressed(move |_, key, _, _| {
        if key == gtk4::gdk::Key::Escape
            || key == gtk4::gdk::Key::Return
            || key == gtk4::gdk::Key::KP_Enter
        {
            window_clone.close();
            gtk4::glib::Propagation::Stop
        } else {
            gtk4::glib::Propagation::Proceed
        }
    });
    window.add_controller(key_controller);

    // Track active state for visual feedback
    let main_box_clone = main_box.clone();
    window.connect_is_active_notify(move |win| {
        if win.is_active() {
            main_box_clone.add_css_class("focused");
        } else {
            main_box_clone.remove_css_class("focused");
        }
    });

    window.present();
    if let Some(watcher) = css_watcher {
        std::mem::forget(watcher);
    }
    window
}

fn load_css(use_mock: bool, theme_name: &str) -> Option<RecommendedWatcher> {
    let display = Display::default().expect("Could not get default display");
    let theme_css = crate::themes::get(theme_name);

    // Built-in provider: theme colors + base layout
    let builtin = CssProvider::new();
    let base_css = if use_mock {
        std::fs::read_to_string("src/popup.css")
            .unwrap_or_else(|_| include_str!("popup.css").to_string())
    } else {
        include_str!("popup.css").to_string()
    };
    builtin.load_from_data(&format!("{}\n{}", theme_css, base_css));

    gtk4::style_context_add_provider_for_display(
        &display,
        &builtin,
        gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );

    // User overrides (additive, higher priority)
    let user_path = dirs::config_dir().map(|p| p.join("quotabar").join("style.css"));
    if let Some(ref path) = user_path {
        if path.exists() {
            let user = CssProvider::new();
            user.load_from_path(path);
            gtk4::style_context_add_provider_for_display(
                &display,
                &user,
                gtk4::STYLE_PROVIDER_PRIORITY_USER,
            );
        }
    }

    // Hot-reload in mock mode (watches src/popup.css)
    if !use_mock {
        return None;
    }

    let reload_provider = builtin.clone();
    let reload_theme = theme_css.to_string();
    let (tx, rx) = std::sync::mpsc::channel::<()>();

    gtk4::glib::timeout_add_local(Duration::from_millis(200), move || {
        let mut changed = false;
        while rx.try_recv().is_ok() {
            changed = true;
        }
        if changed {
            let base = std::fs::read_to_string("src/popup.css").unwrap_or_default();
            reload_provider.load_from_data(&format!("{}\n{}", reload_theme, base));
            println!("CSS reloaded");
        }
        gtk4::glib::ControlFlow::Continue
    });

    let mut watcher =
        match notify::recommended_watcher(move |result: notify::Result<notify::Event>| {
            if result.is_ok() {
                let _ = tx.send(());
            }
        }) {
            Ok(watcher) => watcher,
            Err(_) => return None,
        };

    let watch_path = PathBuf::from("src/popup.css");
    if watcher
        .watch(&watch_path, RecursiveMode::NonRecursive)
        .is_err()
    {
        return None;
    }

    Some(watcher)
}

fn create_provider_section(snapshot: &UsageSnapshot, error: Option<&str>) -> GtkBox {
    let section = GtkBox::new(Orientation::Vertical, 8);
    section.add_css_class("provider-section");
    if error.is_some() {
        section.add_css_class("provider-stale");
    }

    // Provider header with icon and name
    let header = GtkBox::new(Orientation::Horizontal, 8);
    header.add_css_class("provider-header");

    let icon: gtk4::Widget = if let Some(image) = provider_icon(&snapshot.provider) {
        image.upcast()
    } else {
        let label = Label::new(Some(snapshot.provider.icon()));
        label.add_css_class("provider-icon");
        label.set_halign(Align::Center);
        label.set_valign(Align::Center);
        label.set_yalign(0.5);
        label.upcast()
    };
    let icon_box = GtkBox::new(Orientation::Vertical, 0);
    icon_box.set_size_request(20, 20);
    icon_box.set_halign(Align::Center);
    icon_box.set_valign(Align::Center);
    icon_box.append(&icon);
    header.append(&icon_box);

    let name = Label::new(Some(snapshot.provider.display_name()));
    name.add_css_class("provider-name");
    name.set_valign(Align::Center);
    name.set_yalign(0.5);
    header.append(&name);

    let right_side = GtkBox::new(Orientation::Horizontal, 6);
    right_side.set_hexpand(true);
    right_side.set_halign(Align::End);
    right_side.set_valign(Align::Center);

    if let Some(url) = snapshot.provider.usage_url() {
        let link = LinkButton::new(url);
        link.set_label("Usage");
        link.add_css_class("usage-link");
        // GTK's default handler goes through the desktop portal, which fails
        // silently on layer-shell windows; open via xdg-open instead
        link.connect_activate_link(move |_| {
            if let Err(e) = std::process::Command::new("xdg-open").arg(url).spawn() {
                eprintln!("Failed to open {}: {}", url, e);
            }
            gtk4::glib::Propagation::Stop
        });
        right_side.append(&link);
    }

    // Plan badge if available
    if let Some(ref identity) = snapshot.identity {
        if let Some(ref plan) = identity.plan {
            let badge = Label::new(Some(plan));
            badge.add_css_class("plan-badge");
            right_side.append(&badge);
        }
    }

    header.append(&right_side);
    section.append(&header);

    let now = Utc::now();

    // Primary quota bar (5-hour session)
    if let Some(ref primary) = snapshot.primary {
        let expired = primary.is_expired(snapshot.updated_at, now);
        let bar = create_quota_bar("Current session", primary, None, expired);
        section.append(&bar);
    }

    // Secondary quota bar (7-day all models)
    if let Some(ref secondary) = snapshot.secondary {
        let expired = secondary.is_expired(snapshot.updated_at, now);
        let pace = if expired {
            None
        } else {
            pace::compute_pace(snapshot.provider, secondary, now)
        };
        let bar = create_quota_bar(
            "Current week (all models)",
            secondary,
            pace.as_ref(),
            expired,
        );
        section.append(&bar);
    }

    // Tertiary quota bar (7-day model-specific)
    if let Some(ref tertiary) = snapshot.tertiary {
        let expired = tertiary.is_expired(snapshot.updated_at, now);
        let bar = create_quota_bar("Current week (Sonnet only)", tertiary, None, expired);
        section.append(&bar);
    }

    // Cost info
    if let Some(ref cost) = snapshot.cost {
        let cost_box = GtkBox::new(Orientation::Horizontal, 4);
        cost_box.add_css_class("cost-info");

        let cost_label = Label::new(Some(&format!(
            "${:.2} / ${:.2} {}",
            cost.used,
            cost.limit,
            cost.period.as_deref().unwrap_or("")
        )));
        cost_label.add_css_class("cost-text");
        cost_box.append(&cost_label);

        section.append(&cost_box);
    }

    // Show error banner if fetch failed (snapshot is stale/carried forward)
    if let Some(err) = error {
        let error_label = Label::new(Some(err));
        error_label.add_css_class("error-text");
        error_label.set_wrap(true);
        error_label.set_halign(Align::Start);
        section.append(&error_label);
    }

    section
}

fn create_quota_bar(
    label: &str,
    window: &crate::models::RateWindow,
    pace: Option<&UsagePace>,
    expired: bool,
) -> GtkBox {
    let container = GtkBox::new(Orientation::Vertical, 4);
    container.add_css_class("quota-bar-container");
    if expired {
        container.add_css_class("stale");
    }

    let used_percent = window.used_percent;

    // Progress bar (shows used percentage; empty when the window has lapsed
    // and the cached value no longer applies)
    let bar = ProgressBar::new();
    bar.set_fraction(if expired { 0.0 } else { used_percent / 100.0 });
    bar.add_css_class("quota-bar");

    if !expired {
        if used_percent >= 90.0 {
            bar.add_css_class("critical");
        } else if used_percent >= 75.0 {
            bar.add_css_class("warning");
        }
    }

    container.append(&bar);

    // Label row with percentage
    let label_row = GtkBox::new(Orientation::Horizontal, 0);

    let label_widget = Label::new(Some(label));
    label_widget.add_css_class("quota-label");
    label_row.append(&label_widget);

    let percent_text = if expired {
        "unknown".to_string()
    } else {
        format!("{:.0}% used", used_percent)
    };
    let percent_label = Label::new(Some(&percent_text));
    percent_label.add_css_class("quota-percent");
    percent_label.set_hexpand(true);
    percent_label.set_halign(Align::End);
    label_row.append(&percent_label);

    container.append(&label_row);

    // Reset time (the cached description is meaningless once the window lapsed)
    if expired {
        let reset_label = Label::new(Some("Window lapsed · refresh pending"));
        reset_label.add_css_class("reset-time");
        reset_label.set_halign(Align::Start);
        container.append(&reset_label);
    } else if let Some(reset_text) = window.reset_description.as_deref() {
        let reset_label = Label::new(Some(&format!("Resets {}", reset_text)));
        reset_label.add_css_class("reset-time");
        reset_label.set_halign(Align::Start);
        container.append(&reset_label);
    }

    // Pace info row
    if let Some(pace) = pace {
        let left = pace::format_pace_left(pace);
        let right = pace::format_pace_right(pace);

        let pace_text = match right {
            Some(ref r) => format!("{} · {}", left, r),
            None => left,
        };

        let pace_label = Label::new(Some(&pace_text));
        pace_label.add_css_class("pace-info");
        pace_label.set_halign(Align::Start);

        match pace.stage {
            pace::PaceStage::SlightlyAhead | pace::PaceStage::Ahead | pace::PaceStage::FarAhead => {
                pace_label.add_css_class("pace-deficit");
            }
            pace::PaceStage::SlightlyBehind
            | pace::PaceStage::Behind
            | pace::PaceStage::FarBehind => {
                pace_label.add_css_class("pace-reserve");
            }
            pace::PaceStage::OnTrack => {
                pace_label.add_css_class("pace-ontrack");
            }
        }

        container.append(&pace_label);
    }

    container
}

fn create_provider_error_section(provider: &Provider, error: &str) -> GtkBox {
    let section = GtkBox::new(Orientation::Vertical, 8);
    section.add_css_class("provider-section");
    section.add_css_class("provider-error");

    let header = GtkBox::new(Orientation::Horizontal, 8);
    header.add_css_class("provider-header");

    let icon: gtk4::Widget = if let Some(image) = provider_icon(provider) {
        image.upcast()
    } else {
        let label = Label::new(Some(provider.icon()));
        label.add_css_class("provider-icon");
        label.set_halign(Align::Center);
        label.set_valign(Align::Center);
        label.set_yalign(0.5);
        label.upcast()
    };
    let icon_box = GtkBox::new(Orientation::Vertical, 0);
    icon_box.set_size_request(20, 20);
    icon_box.set_halign(Align::Center);
    icon_box.set_valign(Align::Center);
    icon_box.append(&icon);
    header.append(&icon_box);

    let name = Label::new(Some(provider.display_name()));
    name.add_css_class("provider-name");
    name.set_valign(Align::Center);
    name.set_yalign(0.5);
    header.append(&name);

    section.append(&header);

    let error_label = Label::new(Some(error));
    error_label.add_css_class("error-text");
    error_label.set_wrap(true);
    error_label.set_halign(Align::Start);
    section.append(&error_label);

    section
}

fn create_footer(snapshots: &HashMap<Provider, UsageSnapshot>) -> GtkBox {
    let footer = GtkBox::new(Orientation::Horizontal, 8);
    footer.add_css_class("footer");

    // Find most recent update time (convert to local)
    let last_update = snapshots
        .values()
        .map(|s| s.updated_at)
        .max()
        .map(|t| t.with_timezone(&chrono::Local).format("%H:%M").to_string())
        .unwrap_or_else(|| "Unknown".to_string());

    let update_label = Label::new(Some(&format!("Updated at {}", last_update)));
    update_label.add_css_class("footer-text");
    footer.append(&update_label);

    footer
}

fn provider_icon(provider: &Provider) -> Option<Image> {
    let svg_bytes = match provider {
        Provider::Claude => include_bytes!("../assets/claude.svg").as_slice(),
        Provider::Codex => include_bytes!("../assets/openai.svg").as_slice(),
        Provider::OpenCode => include_bytes!("../assets/opencode-logo-dark.svg").as_slice(),
    };

    let svg_string = String::from_utf8_lossy(svg_bytes).replace("currentColor", "white");
    let size = 16;
    let pixbuf = render_svg_icon(svg_string.as_bytes(), size)?;
    let image = Image::from_pixbuf(Some(&pixbuf));
    image.add_css_class("provider-icon");
    image.set_pixel_size(size);
    Some(image)
}

fn render_svg_icon(svg_bytes: &[u8], size: i32) -> Option<Pixbuf> {
    let options = resvg::usvg::Options::default();
    let tree = resvg::usvg::Tree::from_data(svg_bytes, &options).ok()?;
    let mut pixmap = resvg::tiny_skia::Pixmap::new(size as u32, size as u32)?;
    let view = tree.size();
    let scale = (size as f32 / view.width()).min(size as f32 / view.height());
    let scaled_w = view.width() * scale;
    let scaled_h = view.height() * scale;
    let tx = (size as f32 - scaled_w) / 2.0;
    let ty = (size as f32 - scaled_h) / 2.0;
    let transform = resvg::tiny_skia::Transform::from_scale(scale, scale).post_translate(tx, ty);
    let mut pixmap_mut = pixmap.as_mut();
    resvg::render(&tree, transform, &mut pixmap_mut);
    let data = pixmap.take();
    let row_stride = size * 4;
    Some(Pixbuf::from_mut_slice(
        data,
        Colorspace::Rgb,
        true,
        8,
        size,
        size,
        row_stride,
    ))
}
