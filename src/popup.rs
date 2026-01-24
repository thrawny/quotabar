use crate::cache::CacheState;
use crate::mock::mock_snapshots;
use crate::models::{Provider, UsageSnapshot};
use anyhow::Result;
use gtk4::gdk::Display;
use gtk4::prelude::*;
use gtk4::{
    Align, Application, ApplicationWindow, Box as GtkBox, CssProvider, Label, Orientation,
    ProgressBar,
};
use gtk4_layer_shell::{Edge, Layer, LayerShell};
use std::collections::HashMap;

const APP_ID: &str = "com.quotabar.popup";

pub fn run(use_mock: bool) -> Result<()> {
    let app = Application::builder().application_id(APP_ID).build();

    app.connect_activate(move |app| {
        let snapshots = if use_mock {
            mock_snapshots()
        } else {
            CacheState::load()
                .ok()
                .flatten()
                .map(|c| c.snapshots)
                .unwrap_or_default()
        };

        build_ui(app, snapshots);
    });

    app.run_with_args::<&str>(&[]);
    Ok(())
}

fn build_ui(app: &Application, snapshots: HashMap<Provider, UsageSnapshot>) {
    let window = ApplicationWindow::builder()
        .application(app)
        .default_width(320)
        .default_height(400)
        .build();

    // Layer shell setup
    window.init_layer_shell();
    window.set_layer(Layer::Overlay);
    window.set_anchor(Edge::Top, true);
    window.set_anchor(Edge::Right, true);
    window.set_margin(Edge::Top, 40);
    window.set_margin(Edge::Right, 10);
    window.set_keyboard_mode(gtk4_layer_shell::KeyboardMode::OnDemand);

    // Load CSS
    load_css();

    // Main container
    let main_box = GtkBox::new(Orientation::Vertical, 0);
    main_box.add_css_class("popup-container");

    // Header
    let header = create_header();
    main_box.append(&header);

    // Provider sections
    let providers = [Provider::Claude, Provider::Codex, Provider::OpenCode];
    for provider in providers {
        if let Some(snapshot) = snapshots.get(&provider) {
            let section = create_provider_section(snapshot);
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
        if key == gtk4::gdk::Key::Escape {
            window_clone.close();
            gtk4::glib::Propagation::Stop
        } else {
            gtk4::glib::Propagation::Proceed
        }
    });
    window.add_controller(key_controller);

    // Close on any click
    let window_clone = window.clone();
    let click_controller = gtk4::GestureClick::new();
    click_controller.connect_released(move |_, _, _, _| {
        window_clone.close();
    });
    window.add_controller(click_controller);

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
}

fn load_css() {
    let provider = CssProvider::new();

    // Try user CSS first, fall back to built-in
    let user_css = dirs::config_dir()
        .map(|p| p.join("quotabar").join("style.css"))
        .filter(|p| p.exists());

    if let Some(path) = user_css {
        provider.load_from_path(&path);
    } else {
        provider.load_from_data(include_str!("popup.css"));
    }

    gtk4::style_context_add_provider_for_display(
        &Display::default().expect("Could not get default display"),
        &provider,
        gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );
}

fn create_header() -> GtkBox {
    let header = GtkBox::new(Orientation::Horizontal, 8);
    header.add_css_class("header");

    let title = Label::new(Some("Quota Status"));
    title.add_css_class("header-title");
    header.append(&title);

    header
}

fn create_provider_section(snapshot: &UsageSnapshot) -> GtkBox {
    let section = GtkBox::new(Orientation::Vertical, 8);
    section.add_css_class("provider-section");

    // Provider header with icon and name
    let header = GtkBox::new(Orientation::Horizontal, 8);
    header.add_css_class("provider-header");

    let icon = Label::new(Some(snapshot.provider.icon()));
    icon.add_css_class("provider-icon");
    header.append(&icon);

    let name = Label::new(Some(snapshot.provider.display_name()));
    name.add_css_class("provider-name");
    header.append(&name);

    // Plan badge if available
    if let Some(ref identity) = snapshot.identity {
        if let Some(ref plan) = identity.plan {
            let badge = Label::new(Some(plan));
            badge.add_css_class("plan-badge");
            badge.set_hexpand(true);
            badge.set_halign(Align::End);
            header.append(&badge);
        }
    }

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
