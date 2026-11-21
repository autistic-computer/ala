use std::{
    collections::{HashMap, HashSet},
    env, fs,
    path::{Component, Path, PathBuf},
    sync::Arc,
};

use ala::{
    Hsla, Rgba, SharedString, SvgRenderer, WindowControlHoverStyle, WindowControlIcon,
    WindowControlIcons, WindowControlStyle, px,
};

const TARGET_ICON_SIZE: u32 = 16;
const MAX_SVG_BYTES: u64 = 256 * 1024;
const MAX_CSS_BYTES: u64 = 2 * 1024 * 1024;
const MIN_BUTTON_SIZE: f32 = 14.0;
const MAX_BUTTON_SIZE: f32 = 24.0;
const BREEZE_DEFAULT_BUTTON_SIZE: f32 = 18.0;
const ADWAITA_DEFAULT_BUTTON_SIZE: f32 = 24.0;
const BREEZE_GAP: f32 = 6.0;
const ADWAITA_GAP: f32 = 8.0;
const BREEZE_INK_FRACTION: f32 = 10.5 / 18.0;
const ADWAITA_TARGET_INK: f32 = 10.5;
const ADWAITA_MAX_ICON_SIZE: f32 = 16.0;
const CONTROL_ICON_NAMES: [&str; 4] = [
    "window-minimize-symbolic",
    "window-maximize-symbolic",
    "window-restore-symbolic",
    "window-close-symbolic",
];

#[derive(Clone, Copy, PartialEq, Eq)]
enum DirectoryType {
    Fixed,
    Scalable,
    Threshold,
}

struct ThemeDirectory {
    subdir: String,
    size: u32,
    scale: u32,
    kind: DirectoryType,
    min_size: u32,
    max_size: u32,
    threshold: u32,
}

impl ThemeDirectory {
    fn distance(&self, target: u32) -> u32 {
        match self.kind {
            DirectoryType::Fixed => self.size.abs_diff(target),
            DirectoryType::Scalable => {
                if target < self.min_size {
                    self.min_size - target
                } else if target > self.max_size {
                    target - self.max_size
                } else {
                    0
                }
            }
            DirectoryType::Threshold => {
                let min = self.size.saturating_sub(self.threshold);
                let max = self.size.saturating_add(self.threshold);
                if target < min {
                    min - target
                } else if target > max {
                    target - max
                } else {
                    0
                }
            }
        }
    }
}

struct ParsedIndex {
    directories: Vec<ThemeDirectory>,
    inherits: Vec<String>,
}

struct LoadedIcon {
    icon: WindowControlIcon,
    ink_ratio: f32,
}

pub(crate) fn is_kde_desktop() -> bool {
    env::var("XDG_CURRENT_DESKTOP").is_ok_and(|desktop| {
        desktop
            .split(':')
            .any(|part| part.eq_ignore_ascii_case("kde"))
    })
}

pub(crate) fn read_configured_icon_theme() -> Option<String> {
    let config_dir = xdg_config_home()?;
    read_configured_icon_theme_in(&config_dir, is_kde_desktop())
}

pub(crate) fn read_configured_gtk_theme() -> Option<String> {
    let config_dir = xdg_config_home()?;
    read_configured_gtk_theme_in(&config_dir)
}

pub(crate) fn read_configured_negative_color() -> Option<Hsla> {
    let config_dir = xdg_config_home()?;
    read_configured_negative_color_in(&config_dir)
}

pub(crate) fn parse_kde_rgb(value: &str) -> Option<Hsla> {
    let mut parts = value.split(',').map(str::trim);
    let r = parts.next()?.parse::<u8>().ok()?;
    let g = parts.next()?.parse::<u8>().ok()?;
    let b = parts.next()?.parse::<u8>().ok()?;
    if parts.next().is_some() {
        return None;
    }
    Some(
        Rgba {
            r: f32::from(r) / 255.0,
            g: f32::from(g) / 255.0,
            b: f32::from(b) / 255.0,
            a: 1.0,
        }
        .into(),
    )
}

pub(crate) fn load_window_control_icons(
    icon_theme: &str,
    gtk_theme: Option<&str>,
    negative_color: Option<Hsla>,
) -> Option<WindowControlIcons> {
    let base_dirs = icon_base_dirs();
    let theme_dirs = gtk_theme_base_dirs();
    let config_dir = xdg_config_home();
    load_window_control_icons_in(
        icon_theme,
        gtk_theme,
        negative_color,
        is_kde_desktop(),
        &base_dirs,
        &theme_dirs,
        config_dir.as_deref(),
    )
}

fn load_window_control_icons_in(
    icon_theme: &str,
    gtk_theme: Option<&str>,
    negative_color: Option<Hsla>,
    kde_desktop: bool,
    base_dirs: &[PathBuf],
    theme_dirs: &[PathBuf],
    config_dir: Option<&Path>,
) -> Option<WindowControlIcons> {
    let icon_theme = icon_theme.trim();
    if !is_single_segment(icon_theme) {
        return None;
    }

    let renderer = SvgRenderer::new(Arc::new(()));
    let mut resolved: [Option<LoadedIcon>; 4] = [None, None, None, None];
    let mut visited = HashSet::new();

    collect_from_theme(
        icon_theme,
        icon_theme,
        base_dirs,
        &renderer,
        &mut visited,
        &mut resolved,
    );
    if resolved.iter().any(Option::is_none) && !visited.contains("hicolor") {
        collect_from_theme(
            icon_theme,
            "hicolor",
            base_dirs,
            &renderer,
            &mut visited,
            &mut resolved,
        );
    }

    let [minimize, maximize, restore, close] = resolved;
    let minimize = minimize?;
    let maximize = maximize?;
    let restore = restore?;
    let close = close?;

    let max_ink_ratio = minimize
        .ink_ratio
        .max(maximize.ink_ratio)
        .max(close.ink_ratio)
        .clamp(0.35, 1.0);
    let style = resolve_style(
        icon_theme,
        gtk_theme,
        negative_color,
        kde_desktop,
        max_ink_ratio,
        theme_dirs,
        config_dir,
    );

    Some(WindowControlIcons {
        minimize: minimize.icon,
        maximize: maximize.icon,
        restore: restore.icon,
        close: close.icon,
        style,
    })
}

fn resolve_style(
    icon_theme: &str,
    gtk_theme: Option<&str>,
    negative_color: Option<Hsla>,
    kde_desktop: bool,
    max_ink_ratio: f32,
    theme_dirs: &[PathBuf],
    config_dir: Option<&Path>,
) -> WindowControlStyle {
    let gtk_lower = gtk_theme
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_ascii_lowercase);
    let icon_lower = icon_theme.to_ascii_lowercase();

    let hover = if gtk_lower
        .as_deref()
        .is_some_and(|name| name.starts_with("breeze"))
        || (gtk_lower.is_none() && kde_desktop)
        || icon_lower.starts_with("breeze")
    {
        WindowControlHoverStyle::Breeze {
            negative: negative_color,
        }
    } else if gtk_lower
        .as_deref()
        .is_some_and(|name| name.starts_with("yaru"))
        || icon_lower.starts_with("yaru")
    {
        WindowControlHoverStyle::Yaru
    } else {
        WindowControlHoverStyle::Adwaita
    };

    let breeze_size = match hover {
        WindowControlHoverStyle::Breeze { .. } => config_dir.and_then(read_breeze_button_size_in),
        _ => None,
    };
    let gtk_css_size = gtk_theme
        .map(str::trim)
        .filter(|name| is_single_segment(name))
        .and_then(|theme| read_gtk_theme_button_size_in(theme, theme_dirs));
    let default_button_size = match hover {
        WindowControlHoverStyle::Breeze { .. } => BREEZE_DEFAULT_BUTTON_SIZE,
        WindowControlHoverStyle::Adwaita | WindowControlHoverStyle::Yaru => {
            ADWAITA_DEFAULT_BUTTON_SIZE
        }
    };
    let button_size = breeze_size
        .or(gtk_css_size)
        .unwrap_or(default_button_size)
        .clamp(MIN_BUTTON_SIZE, MAX_BUTTON_SIZE);

    let icon_size = match hover {
        WindowControlHoverStyle::Breeze { .. } => {
            let target_ink = button_size * BREEZE_INK_FRACTION;
            (target_ink / max_ink_ratio)
                .round()
                .clamp(10.0, (button_size - 2.0).max(10.0))
        }
        WindowControlHoverStyle::Adwaita | WindowControlHoverStyle::Yaru => {
            (ADWAITA_TARGET_INK / max_ink_ratio).round().clamp(
                10.0,
                ADWAITA_MAX_ICON_SIZE.min((button_size - 2.0).max(10.0)),
            )
        }
    };

    let gap = match hover {
        WindowControlHoverStyle::Breeze { .. } if button_size <= 20.0 => BREEZE_GAP,
        _ => ADWAITA_GAP,
    };

    WindowControlStyle {
        button_size: px(button_size),
        icon_size: px(icon_size),
        gap: px(gap),
        hover,
    }
}

fn collect_from_theme(
    requested_theme: &str,
    theme: &str,
    base_dirs: &[PathBuf],
    renderer: &SvgRenderer,
    visited: &mut HashSet<String>,
    resolved: &mut [Option<LoadedIcon>; 4],
) {
    if !is_single_segment(theme) || !visited.insert(theme.to_owned()) {
        return;
    }
    if resolved.iter().all(Option::is_some) {
        return;
    }

    let Some(index) = load_theme_index(theme, base_dirs) else {
        return;
    };

    for dir in &index.directories {
        if resolved.iter().all(Option::is_some) {
            return;
        }
        for base in base_dirs {
            let candidate_dir = base.join(theme).join(&dir.subdir);
            if !candidate_dir.is_dir() {
                continue;
            }
            for (slot, name) in resolved.iter_mut().zip(CONTROL_ICON_NAMES) {
                if slot.is_some() {
                    continue;
                }
                if let Some(icon) =
                    load_symbolic_svg(requested_theme, name, &candidate_dir, renderer)
                {
                    *slot = Some(icon);
                }
            }
        }
    }

    for parent in &index.inherits {
        if resolved.iter().all(Option::is_some) {
            return;
        }
        collect_from_theme(
            requested_theme,
            parent,
            base_dirs,
            renderer,
            visited,
            resolved,
        );
    }
}

fn load_symbolic_svg(
    requested_theme: &str,
    name: &str,
    dir: &Path,
    renderer: &SvgRenderer,
) -> Option<LoadedIcon> {
    for ext in ["svg", "svgz"] {
        let path = dir.join(format!("{name}.{ext}"));
        let Ok(metadata) = fs::metadata(&path) else {
            continue;
        };
        if !metadata.is_file() || metadata.len() == 0 || metadata.len() > MAX_SVG_BYTES {
            continue;
        }
        let Ok(bytes) = fs::read(&path) else {
            continue;
        };
        let Ok(parsed) = renderer.parse_svg(&bytes) else {
            continue;
        };
        let key = SharedString::from(format!(
            "system-window-control/{requested_theme}/{name}:{}",
            path.display()
        ));
        return Some(LoadedIcon {
            icon: WindowControlIcon {
                key,
                svg: Arc::from(bytes),
            },
            ink_ratio: parsed.ink_ratio(),
        });
    }
    None
}

fn load_theme_index(theme: &str, base_dirs: &[PathBuf]) -> Option<ParsedIndex> {
    for base in base_dirs {
        let index_path = base.join(theme).join("index.theme");
        if let Ok(contents) = fs::read_to_string(&index_path) {
            return Some(parse_theme_index(&contents));
        }
    }
    None
}

fn parse_theme_index(contents: &str) -> ParsedIndex {
    let mut section: Option<&str> = None;
    let mut dir_names = Vec::new();
    let mut inherits = Vec::new();
    let mut sections: HashMap<&str, HashMap<&str, &str>> = HashMap::new();

    for raw_line in contents.lines() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(header) = line.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
            section = Some(header.trim());
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        let value = value.trim();
        match section {
            Some("Icon Theme") => match key {
                "Directories" => {
                    dir_names.extend(
                        value
                            .split(',')
                            .map(str::trim)
                            .filter(|dir| is_safe_subdir(dir)),
                    );
                }
                "Inherits" => {
                    inherits.extend(
                        value
                            .split(',')
                            .map(str::trim)
                            .filter(|parent| is_single_segment(parent))
                            .map(str::to_owned),
                    );
                }
                _ => {}
            },
            Some(name) => {
                sections.entry(name).or_default().insert(key, value);
            }
            None => {}
        }
    }

    let mut directories = Vec::new();
    for subdir in dir_names {
        let Some(props) = sections.get(subdir) else {
            continue;
        };
        let Some(size) = props
            .get("Size")
            .and_then(|value| value.parse::<u32>().ok())
        else {
            continue;
        };
        if size == 0 {
            continue;
        }
        let scale = props
            .get("Scale")
            .and_then(|value| value.parse::<u32>().ok())
            .filter(|&scale| scale > 0)
            .unwrap_or(1);
        let kind = match props.get("Type").copied() {
            Some("Fixed") => DirectoryType::Fixed,
            Some("Scalable") => DirectoryType::Scalable,
            _ => DirectoryType::Threshold,
        };
        let min_size = props
            .get("MinSize")
            .and_then(|value| value.parse::<u32>().ok())
            .unwrap_or(size);
        let max_size = props
            .get("MaxSize")
            .and_then(|value| value.parse::<u32>().ok())
            .unwrap_or(size);
        let threshold = props
            .get("Threshold")
            .and_then(|value| value.parse::<u32>().ok())
            .unwrap_or(2);
        directories.push(ThemeDirectory {
            subdir: subdir.to_owned(),
            size,
            scale,
            kind,
            min_size,
            max_size,
            threshold,
        });
    }

    directories.sort_by_key(|dir| {
        (
            dir.distance(TARGET_ICON_SIZE),
            dir.size.abs_diff(TARGET_ICON_SIZE),
            dir.scale,
        )
    });

    ParsedIndex {
        directories,
        inherits,
    }
}

fn read_breeze_button_size_in(config_dir: &Path) -> Option<f32> {
    let contents = fs::read_to_string(config_dir.join("breezerc")).ok()?;
    let value = parse_ini_value(&contents, "Windeco", "ButtonSize")?;
    match value.as_str() {
        "ButtonTiny" => Some(14.0),
        "ButtonSmall" => Some(16.0),
        "ButtonNormal" => Some(18.0),
        "ButtonLarge" => Some(22.0),
        "ButtonVeryLarge" => Some(26.0),
        "ButtonHuge" => Some(30.0),
        _ => None,
    }
}

fn read_gtk_theme_button_size_in(gtk_theme: &str, theme_dirs: &[PathBuf]) -> Option<f32> {
    for base in theme_dirs {
        for sub in ["gtk-4.0/gtk.css", "gtk-3.0/gtk.css"] {
            let path = base.join(gtk_theme).join(sub);
            let Ok(meta) = fs::metadata(&path) else {
                continue;
            };
            if !meta.is_file() || meta.len() == 0 || meta.len() > MAX_CSS_BYTES {
                continue;
            }
            let Ok(contents) = fs::read_to_string(&path) else {
                continue;
            };
            if let Some(size) = parse_gtk_titlebutton_size(&contents) {
                return Some(size);
            }
        }
    }
    None
}

fn parse_gtk_titlebutton_size(css: &str) -> Option<f32> {
    let cleaned = strip_css_comments(css);
    for block in cleaned.split('}') {
        let Some((selectors, body)) = block.split_once('{') else {
            continue;
        };
        let targets_button = selectors.split(',').any(selector_targets_titlebutton);
        if !targets_button {
            continue;
        }
        let mut min_width = None;
        let mut min_height = None;
        let mut background_size = None;
        for decl in body.split(';') {
            let Some((prop, value)) = decl.split_once(':') else {
                continue;
            };
            let prop = prop.trim();
            let first_px = value
                .split_whitespace()
                .next()
                .and_then(|token| token.strip_suffix("px"))
                .and_then(|num| num.parse::<f32>().ok())
                .filter(|&px| (12.0..=32.0).contains(&px));
            match prop {
                "min-width" => min_width = first_px.or(min_width),
                "min-height" => min_height = first_px.or(min_height),
                "background-size" => background_size = first_px.or(background_size),
                _ => {}
            }
        }
        if let Some(size) = match (min_width, min_height) {
            (Some(w), Some(h)) => Some(w.min(h)),
            (Some(w), None) => Some(w),
            (None, Some(h)) => Some(h),
            (None, None) => background_size,
        } {
            return Some(size);
        }
    }
    None
}

fn selector_targets_titlebutton(selector: &str) -> bool {
    let selector = selector.trim();
    if selector.is_empty() || selector.contains(":not(.titlebutton)") {
        return false;
    }
    let last = selector
        .split([' ', '\t', '\n', '\r', '>'])
        .rfind(|part| !part.is_empty())
        .unwrap_or_default();
    if last.starts_with("image") || last.starts_with("label") {
        return false;
    }
    if last.contains("titlebutton") {
        return true;
    }
    selector.contains("windowcontrols") && (last == "button" || last.starts_with("button."))
}

fn strip_css_comments(css: &str) -> String {
    let mut out = String::with_capacity(css.len());
    let mut rest = css;
    while let Some((before, after)) = rest.split_once("/*") {
        out.push_str(before);
        match after.split_once("*/") {
            Some((_, tail)) => rest = tail,
            None => return out,
        }
    }
    out.push_str(rest);
    out
}

fn read_configured_icon_theme_in(config_dir: &Path, prefer_kde: bool) -> Option<String> {
    let kde = ("kdeglobals", "Icons", "Theme");
    let gtk3 = ("gtk-3.0/settings.ini", "Settings", "gtk-icon-theme-name");
    let gtk4 = ("gtk-4.0/settings.ini", "Settings", "gtk-icon-theme-name");
    let order = if prefer_kde {
        [kde, gtk3, gtk4]
    } else {
        [gtk3, gtk4, kde]
    };

    for (rel_path, section, key) in order {
        let Ok(contents) = fs::read_to_string(config_dir.join(rel_path)) else {
            continue;
        };
        if let Some(theme) = parse_ini_value(&contents, section, key) {
            return Some(theme);
        }
    }
    None
}

fn read_configured_gtk_theme_in(config_dir: &Path) -> Option<String> {
    for rel_path in ["gtk-3.0/settings.ini", "gtk-4.0/settings.ini"] {
        let Ok(contents) = fs::read_to_string(config_dir.join(rel_path)) else {
            continue;
        };
        if let Some(theme) = parse_ini_value(&contents, "Settings", "gtk-theme-name") {
            return Some(theme);
        }
    }
    None
}

fn read_configured_negative_color_in(config_dir: &Path) -> Option<Hsla> {
    if let Ok(contents) = fs::read_to_string(config_dir.join("kdeglobals"))
        && let Some(raw) = parse_ini_value(&contents, "Colors:Window", "ForegroundNegative")
        && let Some(color) = parse_kde_rgb(&raw)
    {
        return Some(color);
    }
    if let Ok(contents) = fs::read_to_string(config_dir.join("gtk-3.0/colors.css")) {
        for line in contents.lines() {
            let line = line.trim();
            if let Some(rest) = line.strip_prefix("@define-color error_color_breeze ")
                && let Some(hex) = rest.trim().strip_suffix(';')
                && let Some(color) = parse_hex_color(hex.trim())
            {
                return Some(color);
            }
        }
    }
    None
}

fn parse_hex_color(hex: &str) -> Option<Hsla> {
    let hex = hex.strip_prefix('#')?;
    if hex.len() != 6 {
        return None;
    }
    let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
    let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
    let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
    Some(
        Rgba {
            r: f32::from(r) / 255.0,
            g: f32::from(g) / 255.0,
            b: f32::from(b) / 255.0,
            a: 1.0,
        }
        .into(),
    )
}

fn parse_ini_value(contents: &str, target_section: &str, target_key: &str) -> Option<String> {
    let mut in_section = false;
    for raw_line in contents.lines() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        if let Some(header) = line.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
            in_section = header.trim() == target_section;
            continue;
        }
        if !in_section {
            continue;
        }
        if let Some((key, value)) = line.split_once('=')
            && key.trim() == target_key
        {
            let value = value.trim().trim_matches('"').trim_matches('\'').trim();
            if !value.is_empty() {
                return Some(value.to_owned());
            }
        }
    }
    None
}

fn xdg_config_home() -> Option<PathBuf> {
    if let Some(dir) = env::var_os("XDG_CONFIG_HOME").filter(|v| !v.is_empty()) {
        let path = PathBuf::from(dir);
        if path.is_absolute() {
            return Some(path);
        }
    }
    env::var_os("HOME")
        .filter(|v| !v.is_empty())
        .map(|home| PathBuf::from(home).join(".config"))
}

fn icon_base_dirs() -> Vec<PathBuf> {
    xdg_data_subdirs(".icons", "icons")
}

fn gtk_theme_base_dirs() -> Vec<PathBuf> {
    xdg_data_subdirs(".themes", "themes")
}

fn xdg_data_subdirs(home_dotdir: &str, xdg_subdir: &str) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    let mut push_unique = |path: PathBuf| {
        if !dirs.contains(&path) {
            dirs.push(path);
        }
    };

    let home = env::var_os("HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from);
    if let Some(home) = &home {
        push_unique(home.join(home_dotdir));
    }

    if let Some(data_home) = env::var_os("XDG_DATA_HOME").filter(|v| !v.is_empty()) {
        let path = PathBuf::from(data_home);
        if path.is_absolute() {
            push_unique(path.join(xdg_subdir));
        }
    } else if let Some(home) = &home {
        push_unique(home.join(".local/share").join(xdg_subdir));
    }

    let data_dirs = env::var_os("XDG_DATA_DIRS").filter(|v| !v.is_empty());
    let default_data_dirs = std::ffi::OsString::from("/usr/local/share:/usr/share");
    for dir in env::split_paths(data_dirs.as_ref().unwrap_or(&default_data_dirs)) {
        if !dir.as_os_str().is_empty() {
            push_unique(dir.join(xdg_subdir));
        }
    }

    dirs
}

fn is_single_segment(name: &str) -> bool {
    let mut components = Path::new(name).components();
    matches!(
        (components.next(), components.next()),
        (Some(Component::Normal(_)), None)
    )
}

fn is_safe_subdir(path: &str) -> bool {
    let mut components = Path::new(path).components().peekable();
    components.peek().is_some() && components.all(|c| matches!(c, Component::Normal(_)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ala::{DevicePixels, Size, SvgSize};
    use std::time::{SystemTime, UNIX_EPOCH};

    const BREEZE_MIN: &str = r##"<?xml version="1.0" encoding="UTF-8"?>
<svg viewBox="0 0 16 16" xmlns="http://www.w3.org/2000/svg">
    <style type="text/css" id="current-color-scheme">.ColorScheme-Text { color: #fcfcfc; }</style>
    <path d="M8 11.707l-6-6L2.707 5 8 10.293 13.293 5l.707.707-6 6z" class="ColorScheme-Text" fill="currentColor"/>
</svg>"##;

    const ADWAITA_MAX: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="16px" height="16px" viewBox="0 0 16 16"><path fill="#2e3436" d="M4 4v8h8V4zm2 2h4v4H6z"/></svg>"##;

    const SIMPLE_RESTORE: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16"><path d="M4 4h8v8H4z" fill="currentColor"/></svg>"#;

    const SIMPLE_CLOSE: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16"><path d="M4 4h8v8H4z" fill="currentColor"/></svg>"#;

    fn temp_root(tag: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock after epoch")
            .as_nanos();
        let root = env::temp_dir().join(format!(
            "ala-icon-theme-{tag}-{}-{nanos}",
            std::process::id()
        ));
        fs::create_dir_all(&root).expect("create temp test dir");
        root
    }

    #[test]
    fn resolves_16px_symbolic_icons_across_inherits_and_ignores_non_symbolic() {
        let root = temp_root("resolve");
        let child = root.join("breeze-child");
        let parent = root.join("parent");
        let themes = root.join("themes");
        let config = root.join("config");
        fs::create_dir_all(child.join("actions/16")).unwrap();
        fs::create_dir_all(child.join("actions/22")).unwrap();
        fs::create_dir_all(parent.join("symbolic/ui")).unwrap();
        fs::create_dir_all(themes.join("Breeze/gtk-4.0")).unwrap();
        fs::create_dir_all(&config).unwrap();

        fs::write(
            child.join("index.theme"),
            "[Icon Theme]\nInherits=parent\nDirectories=actions/22,actions/16\n\n[actions/22]\nSize=22\nType=Fixed\n\n[actions/16]\nSize=16\nType=Fixed\n",
        )
        .unwrap();
        fs::write(
            parent.join("index.theme"),
            "[Icon Theme]\nDirectories=symbolic/ui\n\n[symbolic/ui]\nSize=16\nMinSize=8\nMaxSize=512\nType=Scalable\n",
        )
        .unwrap();
        fs::write(
            themes.join("Breeze/gtk-4.0/gtk.css"),
            "headerbar windowcontrols button { min-height: 18px; min-width: 18px; }",
        )
        .unwrap();

        fs::write(
            child.join("actions/22/window-minimize-symbolic.svg"),
            SIMPLE_CLOSE,
        )
        .unwrap();
        fs::write(
            child.join("actions/16/window-minimize-symbolic.svg"),
            BREEZE_MIN,
        )
        .unwrap();
        fs::write(
            child.join("actions/16/window-maximize-symbolic.svg"),
            ADWAITA_MAX,
        )
        .unwrap();
        fs::write(child.join("actions/16/window-close.svg"), SIMPLE_CLOSE).unwrap();
        fs::write(
            parent.join("symbolic/ui/window-restore-symbolic.svg"),
            SIMPLE_RESTORE,
        )
        .unwrap();

        let negative = parse_kde_rgb("218,68,83").expect("valid rgb");
        assert!(
            load_window_control_icons_in(
                "breeze-child",
                Some("Breeze"),
                Some(negative),
                true,
                std::slice::from_ref(&root),
                std::slice::from_ref(&themes),
                Some(&config),
            )
            .is_none()
        );

        fs::write(
            parent.join("symbolic/ui/window-close-symbolic.svg"),
            SIMPLE_CLOSE,
        )
        .unwrap();
        let icons = load_window_control_icons_in(
            "breeze-child",
            Some("Breeze"),
            Some(negative),
            true,
            std::slice::from_ref(&root),
            std::slice::from_ref(&themes),
            Some(&config),
        )
        .expect("all four symbolic icons resolve across theme inheritance");
        assert_eq!(icons.minimize.svg.as_ref(), BREEZE_MIN.as_bytes());
        assert_eq!(icons.maximize.svg.as_ref(), ADWAITA_MAX.as_bytes());
        assert_eq!(icons.restore.svg.as_ref(), SIMPLE_RESTORE.as_bytes());
        assert_eq!(icons.close.svg.as_ref(), SIMPLE_CLOSE.as_bytes());
        assert_eq!(
            icons.style,
            WindowControlStyle {
                button_size: px(18.0),
                icon_size: px(14.0),
                gap: px(6.0),
                hover: WindowControlHoverStyle::Breeze {
                    negative: Some(negative),
                },
            }
        );

        fs::write(
            config.join("breezerc"),
            "[Windeco]\nButtonSize=ButtonLarge\n",
        )
        .unwrap();
        let large = load_window_control_icons_in(
            "breeze-child",
            Some("Breeze"),
            Some(negative),
            true,
            std::slice::from_ref(&root),
            std::slice::from_ref(&themes),
            Some(&config),
        )
        .expect("breezerc button size override resolves");
        assert_eq!(large.style.button_size, px(22.0));
        assert_eq!(large.style.icon_size, px(17.0));

        let renderer = SvgRenderer::new(Arc::new(()));
        for svg in [&icons.minimize.svg, &icons.maximize.svg] {
            let parsed = renderer.parse_svg(svg).expect("symbolic svg parses");
            let image = renderer
                .render_parsed(
                    &parsed,
                    SvgSize::ExactSize(Size::new(DevicePixels(16), DevicePixels(16))),
                )
                .expect("symbolic svg rasterizes");
            assert!(
                image
                    .as_bytes(0)
                    .expect("frame bytes")
                    .chunks_exact(4)
                    .any(|px| px[3] > 0),
                "rasterized symbolic svg must produce visible alpha"
            );
        }

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn reads_configured_themes_colors_and_gtk_button_size() {
        let root = temp_root("config");
        fs::create_dir_all(root.join("gtk-3.0")).unwrap();
        fs::write(
            root.join("kdeglobals"),
            "[Icons]\nTheme=breeze-dark\n\n[Colors:Window]\nForegroundNegative=218,68,83\n",
        )
        .unwrap();
        fs::write(
            root.join("gtk-3.0/settings.ini"),
            "[Settings]\ngtk-icon-theme-name=Adwaita\ngtk-theme-name=Breeze\n",
        )
        .unwrap();

        assert_eq!(
            read_configured_icon_theme_in(&root, true).as_deref(),
            Some("breeze-dark")
        );
        assert_eq!(
            read_configured_icon_theme_in(&root, false).as_deref(),
            Some("Adwaita")
        );
        assert_eq!(
            read_configured_gtk_theme_in(&root).as_deref(),
            Some("Breeze")
        );
        assert_eq!(
            read_configured_negative_color_in(&root),
            parse_kde_rgb("218,68,83")
        );
        assert_eq!(
            parse_gtk_titlebutton_size(
                "headerbar.selection-mode button:not(.titlebutton) { min-width: 30px; }\n\
                 window headerbar button.titlebutton { min-height: 18px; min-width: 18px; }"
            ),
            Some(18.0)
        );
        assert_eq!(
            parse_gtk_titlebutton_size(
                "windowcontrols button { min-width: 24px; min-height: 24px; border-radius: 9999px; }"
            ),
            Some(24.0)
        );

        let _ = fs::remove_dir_all(&root);
    }
}
