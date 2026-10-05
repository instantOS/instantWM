//! Freedesktop icon lookup, including private StatusNotifierItem search paths.
//!
//! Search only directories declared by index.theme: scanning an entire theme
//! recursively is both expensive and loses its size and inheritance semantics.
use std::collections::{HashMap, HashSet};
use std::path::{Component, Path, PathBuf};

#[derive(Default)]
pub(super) struct IconThemes {
    pub(super) roots: Vec<PathBuf>,
    pub(super) pixmaps: Vec<PathBuf>,
}

pub(super) type Ini = HashMap<String, HashMap<String, String>>;

pub(super) fn parse_ini(text: &str) -> Ini {
    let mut result = Ini::new();
    let mut section = String::new();
    for line in text.lines().map(str::trim) {
        if line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        if let Some(name) = line.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
            section = name.to_owned();
        } else if let Some((key, value)) = line.split_once('=') {
            result
                .entry(section.clone())
                .or_default()
                .insert(key.trim().to_owned(), value.trim().to_owned());
        }
    }
    result
}

fn safe_relative(path: &Path) -> bool {
    !path.as_os_str().is_empty() && path.components().all(|c| matches!(c, Component::Normal(_)))
}

fn icon_file(dir: &Path, name: &str) -> Option<PathBuf> {
    let path = Path::new(name);
    if !safe_relative(path) {
        return None;
    }
    // A few clients publish filenames rather than extensionless icon names.
    let direct = dir.join(path);
    if direct.is_file() {
        return Some(direct);
    }
    ["png", "svg"]
        .into_iter()
        .map(|ext| dir.join(format!("{name}.{ext}")))
        .find(|p| p.is_file())
}

impl IconThemes {
    pub(super) fn system() -> Self {
        let mut roots = Vec::new();
        if let Some(home) = dirs::home_dir() {
            roots.push(home.join(".icons"));
        }
        if let Some(data) = dirs::data_dir() {
            roots.push(data.join("icons"));
        }
        let data_dirs = std::env::var_os("XDG_DATA_DIRS")
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "/usr/local/share:/usr/share".into());
        let data: Vec<_> = std::env::split_paths(&data_dirs)
            .filter(|p| p.is_absolute())
            .collect();
        roots.extend(data.iter().map(|p| p.join("icons")));
        Self {
            roots,
            pixmaps: data.iter().map(|p| p.join("pixmaps")).collect(),
        }
    }

    pub(super) fn find(
        &self,
        name: &str,
        extra: &str,
        theme: &str,
        height: u32,
    ) -> Option<PathBuf> {
        if name.is_empty() {
            return None;
        }
        let path = Path::new(name);
        if path.is_absolute() {
            return path.is_file().then(|| path.to_owned());
        }
        if !safe_relative(path) {
            return None;
        }
        let extra = Path::new(extra);
        let mut roots = self.roots.clone();
        if extra.is_absolute() {
            if let Some(icon) = icon_file(extra, name) {
                return Some(icon);
            }
            // IconThemePath may point at a theme itself or at an icon base dir.
            if let Some(index) = read_index(extra)
                && let Some(icon) = find_in_theme(&[extra.to_owned()], &index, name, height)
            {
                return Some(icon);
            }
            roots.insert(0, extra.to_owned());
        }
        let mut visited = HashSet::new();
        if let Some(icon) = find_inherited(&roots, theme, name, height, &mut visited) {
            return Some(icon);
        }
        if let Some(icon) = find_inherited(&roots, "hicolor", name, height, &mut visited) {
            return Some(icon);
        }
        roots
            .iter()
            .chain(&self.pixmaps)
            .find_map(|root| icon_file(root, name))
    }
}

fn read_index(dir: &Path) -> Option<Ini> {
    Some(parse_ini(
        &std::fs::read_to_string(dir.join("index.theme")).ok()?,
    ))
}

fn find_inherited(
    roots: &[PathBuf],
    theme: &str,
    name: &str,
    height: u32,
    visited: &mut HashSet<String>,
) -> Option<PathBuf> {
    if !safe_relative(Path::new(theme)) || !visited.insert(theme.to_owned()) || visited.len() > 64 {
        return None;
    }
    let dirs: Vec<_> = roots.iter().map(|root| root.join(theme)).collect();
    // The first index.theme defines the theme, even if its icons span roots.
    let index = dirs.iter().find_map(|dir| read_index(dir))?;
    if let Some(icon) = find_in_theme(&dirs, &index, name, height) {
        return Some(icon);
    }
    index
        .get("Icon Theme")?
        .get("Inherits")?
        .split(',')
        .map(str::trim)
        .find_map(|parent| find_inherited(roots, parent, name, height, visited))
}

fn find_in_theme(dirs: &[PathBuf], index: &Ini, name: &str, height: u32) -> Option<PathBuf> {
    let header = index.get("Icon Theme")?;
    let directories = header
        .get("Directories")
        .into_iter()
        .chain(header.get("ScaledDirectories"))
        .flat_map(|s| s.split(','))
        .map(str::trim);
    let mut best: Option<(u32, PathBuf)> = None;
    for subdir in directories {
        if !safe_relative(Path::new(subdir)) {
            continue;
        }
        let Some(section) = index.get(subdir) else {
            continue;
        };
        let Some(distance) = directory_distance(section, height) else {
            continue;
        };
        for dir in dirs {
            if let Some(icon) = icon_file(&dir.join(subdir), name) {
                if distance == 0 {
                    return Some(icon);
                }
                if best.as_ref().is_none_or(|(old, _)| distance < *old) {
                    best = Some((distance, icon));
                }
            }
        }
    }
    best.map(|(_, path)| path)
}

fn directory_distance(section: &HashMap<String, String>, height: u32) -> Option<u32> {
    let number = |key: &str| section.get(key).and_then(|v| v.parse::<u32>().ok());
    let size = number("Size")?;
    let scale = number("Scale").unwrap_or(1).max(1);
    let (min, max) = match section
        .get("Type")
        .map(String::as_str)
        .unwrap_or("Threshold")
    {
        "Fixed" => (size, size),
        "Scalable" => (
            number("MinSize").unwrap_or(size),
            number("MaxSize").unwrap_or(size),
        ),
        _ => {
            let threshold = number("Threshold").unwrap_or(2);
            (
                size.saturating_sub(threshold),
                size.saturating_add(threshold),
            )
        }
    };
    let min = min.saturating_mul(scale);
    let max = max.saturating_mul(scale).max(min);
    Some(if height < min {
        min - height
    } else {
        height.saturating_sub(max)
    })
}

/// GTK and KDE store the icon theme independently of the compositor. An
/// explicit instantWM setting takes precedence; no desktop process is needed.
pub(super) fn desktop_icon_theme() -> String {
    if let Some(config) = dirs::config_dir() {
        for (file, section, key) in [
            ("gtk-4.0/settings.ini", "Settings", "gtk-icon-theme-name"),
            ("gtk-3.0/settings.ini", "Settings", "gtk-icon-theme-name"),
            ("kdeglobals", "Icons", "Theme"),
        ] {
            if let Ok(text) = std::fs::read_to_string(config.join(file))
                && let Some(value) = parse_ini(&text).get(section).and_then(|s| s.get(key))
                && !value.is_empty()
            {
                return value.trim_matches('"').to_owned();
            }
        }
    }
    "Adwaita".to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn theme(root: &Path, name: &str, inherits: &str) {
        let dir = root.join(name);
        std::fs::create_dir_all(dir.join("small")).unwrap();
        std::fs::create_dir_all(dir.join("large")).unwrap();
        std::fs::write(dir.join("index.theme"), format!("[Icon Theme]\nDirectories=small,large\nInherits={inherits}\n[small]\nSize=16\nType=Fixed\n[large]\nSize=32\nType=Fixed\n")).unwrap();
    }

    #[test]
    fn lookup_obeys_recursive_inheritance_size_root_priority_and_hicolor() {
        let user = tempfile::tempdir().unwrap();
        let system = tempfile::tempdir().unwrap();
        for root in [user.path(), system.path()] {
            theme(root, "Selected", "Parent");
            theme(root, "Parent", "Grandparent");
            theme(root, "Grandparent", "Selected"); // A cycle must terminate.
            theme(root, "hicolor", "");
        }
        let themes = IconThemes {
            roots: vec![user.path().to_owned(), system.path().to_owned()],
            pixmaps: vec![],
        };
        let small = user.path().join("Grandparent/small/network.svg");
        let large = system.path().join("Grandparent/large/network.png");
        std::fs::write(&small, "").unwrap();
        std::fs::write(&large, "").unwrap();
        assert_eq!(themes.find("network", "", "Selected", 16), Some(small));
        assert_eq!(
            themes.find("network", "", "Selected", 30),
            Some(large.clone())
        );
        let override_icon = user.path().join("Grandparent/large/network.svg");
        std::fs::write(&override_icon, "").unwrap();
        assert_eq!(
            themes.find("network", "", "Selected", 32),
            Some(override_icon)
        );
        let fallback = system.path().join("hicolor/small/fallback.svg");
        std::fs::write(&fallback, "").unwrap();
        assert_eq!(themes.find("fallback", "", "Selected", 16), Some(fallback));
        assert!(themes.find("missing", "", "Selected", 16).is_none());
    }

    #[test]
    fn private_paths_absolute_files_and_pixmaps_are_supported() {
        let dir = tempfile::tempdir().unwrap();
        let themes = IconThemes {
            roots: vec![],
            pixmaps: vec![dir.path().to_owned()],
        };
        let file = dir.path().join("tray.png");
        std::fs::write(&file, "").unwrap();
        assert_eq!(
            themes.find(file.to_str().unwrap(), "", "missing", 24),
            Some(file.clone())
        );
        assert_eq!(
            themes.find("tray", dir.path().to_str().unwrap(), "missing", 24),
            Some(file.clone())
        );
        assert_eq!(themes.find("tray", "", "missing", 24), Some(file));
        theme(dir.path(), "Private", "");
        let private = dir.path().join("Private/large/app.svg");
        std::fs::write(&private, "").unwrap();
        let base = dir.path().join("Private");
        assert_eq!(
            themes.find("app", base.to_str().unwrap(), "missing", 32),
            Some(private.clone())
        );
        assert_eq!(
            themes.find("app", dir.path().to_str().unwrap(), "Private", 32),
            Some(private)
        );
        assert!(
            themes
                .find("../tray", dir.path().to_str().unwrap(), "missing", 24)
                .is_none()
        );
    }

    #[test]
    fn scalable_threshold_and_scaled_directories_match_physical_size() {
        let section = |text| parse_ini(text).remove("icon").unwrap();
        let scalable = section("[icon]\nSize=24\nType=Scalable\nMinSize=16\nMaxSize=64\n");
        assert_eq!(directory_distance(&scalable, 48), Some(0));
        assert_eq!(directory_distance(&scalable, 80), Some(16));
        let scaled = section("[icon]\nSize=24\nScale=2\nType=Fixed\n");
        assert_eq!(directory_distance(&scaled, 48), Some(0));
        let threshold = section("[icon]\nSize=24\nThreshold=4\n");
        assert_eq!(directory_distance(&threshold, 28), Some(0));
        assert_eq!(directory_distance(&threshold, 30), Some(2));
    }
}
