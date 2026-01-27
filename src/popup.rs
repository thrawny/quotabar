use crate::cache::CacheState;
use crate::config::Config;
use crate::mock::mock_snapshots;
use crate::models::{Provider, UsageSnapshot};
use anyhow::Result;
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

        let snapshots = if use_mock {
            mock_snapshots()
        } else {
            CacheState::load()
                .ok()
                .flatten()
                .map(|c| c.snapshots)
                .unwrap_or_default()
        };

        let window = build_ui(app, snapshots, use_mock);
        *window_state.borrow_mut() = Some(window);
    });

    app.run_with_args::<&str>(&[]);
    Ok(())
}

fn build_ui(
    app: &Application,
    snapshots: HashMap<Provider, UsageSnapshot>,
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

    // Load CSS
    let css_watcher = load_css(use_mock);

    // Main container
    let main_box = GtkBox::new(Orientation::Vertical, 0);
    main_box.add_css_class("popup-container");

    let selected_provider = Config::load()
        .ok()
        .and_then(|config| config.general.selected_provider);
    let selected_state: Rc<RefCell<Option<Provider>>> = Rc::new(RefCell::new(selected_provider));
    let sections: Rc<RefCell<Vec<(Provider, GtkBox)>>> = Rc::new(RefCell::new(Vec::new()));

    // Provider sections
    let providers = [Provider::Claude, Provider::Codex, Provider::OpenCode];
    for provider in providers {
        if let Some(snapshot) = snapshots.get(&provider) {
            let section = create_provider_section(snapshot);
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

fn load_css(use_mock: bool) -> Option<RecommendedWatcher> {
    let provider = CssProvider::new();
    let css_path = resolve_css_path(use_mock);

    // Try user CSS first, fall back to built-in
    if let Some(path) = css_path.as_ref().filter(|p| p.exists()) {
        provider.load_from_path(path);
    } else {
        provider.load_from_data(include_str!("popup.css"));
    }

    gtk4::style_context_add_provider_for_display(
        &Display::default().expect("Could not get default display"),
        &provider,
        gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );

    let path = css_path?;

    if !path.exists() {
        return None;
    }

    let provider_for_reload = provider.clone();
    let reload_path = path.clone();
    let (tx, rx) = std::sync::mpsc::channel::<()>();
    gtk4::glib::timeout_add_local(Duration::from_millis(200), move || {
        let mut changed = false;
        while rx.try_recv().is_ok() {
            changed = true;
        }
        if changed {
            provider_for_reload.load_from_path(&reload_path);
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

    if watcher
        .watch(path.as_path(), RecursiveMode::NonRecursive)
        .is_err()
    {
        return None;
    }

    Some(watcher)
}

fn resolve_css_path(use_mock: bool) -> Option<PathBuf> {
    if use_mock {
        return Some(PathBuf::from("src").join("popup.css"));
    }

    dirs::config_dir().map(|p| p.join("quotabar").join("style.css"))
}

fn create_provider_section(snapshot: &UsageSnapshot) -> GtkBox {
    let section = GtkBox::new(Orientation::Vertical, 8);
    section.add_css_class("provider-section");

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

    // Primary quota bar (5-hour session)
    if let Some(ref primary) = snapshot.primary {
        let bar = create_quota_bar(
            "Current session",
            primary.used_percent,
            primary.reset_description.as_deref(),
        );
        section.append(&bar);
    }

    // Secondary quota bar (7-day all models)
    if let Some(ref secondary) = snapshot.secondary {
        let bar = create_quota_bar(
            "Current week (all models)",
            secondary.used_percent,
            secondary.reset_description.as_deref(),
        );
        section.append(&bar);
    }

    // Tertiary quota bar (7-day model-specific)
    if let Some(ref tertiary) = snapshot.tertiary {
        let bar = create_quota_bar(
            "Current week (Sonnet only)",
            tertiary.used_percent,
            tertiary.reset_description.as_deref(),
        );
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

    section
}

fn create_quota_bar(label: &str, used_percent: f64, reset: Option<&str>) -> GtkBox {
    let container = GtkBox::new(Orientation::Vertical, 4);
    container.add_css_class("quota-bar-container");

    // Progress bar (shows used percentage)
    let bar = ProgressBar::new();
    bar.set_fraction(used_percent / 100.0);
    bar.add_css_class("quota-bar");

    // Add status class based on usage
    if used_percent >= 90.0 {
        bar.add_css_class("critical");
    } else if used_percent >= 75.0 {
        bar.add_css_class("warning");
    }

    container.append(&bar);

    // Label row with percentage
    let label_row = GtkBox::new(Orientation::Horizontal, 0);

    let label_widget = Label::new(Some(label));
    label_widget.add_css_class("quota-label");
    label_row.append(&label_widget);

    let percent_label = Label::new(Some(&format!("{:.0}% used", used_percent)));
    percent_label.add_css_class("quota-percent");
    percent_label.set_hexpand(true);
    percent_label.set_halign(Align::End);
    label_row.append(&percent_label);

    container.append(&label_row);

    // Reset time
    if let Some(reset_text) = reset {
        let reset_label = Label::new(Some(&format!("Resets {}", reset_text)));
        reset_label.add_css_class("reset-time");
        reset_label.set_halign(Align::Start);
        container.append(&reset_label);
    }

    container
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
