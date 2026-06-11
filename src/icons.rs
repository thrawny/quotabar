use std::fs;
use std::path::PathBuf;

/// Waybar icons embedded at compile time, written to disk for CSS to reference.
const ICONS: &[(&str, &str)] = &[
    ("claude.svg", include_str!("../assets/waybar/claude.svg")),
    ("openai.svg", include_str!("../assets/waybar/openai.svg")),
];

/// Directory where waybar icons live (~/.local/share/quotabar)
pub fn icon_dir() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("quotabar")
}

/// Write embedded icons to disk if missing or outdated.
pub fn ensure_icons() {
    let dir = icon_dir();
    if fs::create_dir_all(&dir).is_err() {
        return;
    }
    for (name, content) in ICONS {
        let path = dir.join(name);
        if fs::read_to_string(&path).ok().as_deref() != Some(*content) {
            let _ = fs::write(&path, content);
        }
    }
}
