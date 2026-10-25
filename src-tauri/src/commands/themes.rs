// The Plymouth themes installed on the system and the one in use. This is
// only reading: a theme is a folder with a .plymouth file, and nothing here
// writes to it.

use serde::Serialize;
use std::path::{Path, PathBuf};

pub const THEME_DIRS: &[&str] = &["/usr/share/plymouth/themes", "/lib/plymouth/themes", "/usr/local/share/plymouth/themes"];
const PLYMOUTHD_CONF: &str = "/etc/plymouth/plymouthd.conf";
const DEFAULT_LINK: &str = "/usr/share/plymouth/themes/default.plymouth";
const MAX_PREVIEW_BYTES: u64 = 8 * 1024 * 1024;
// The images a theme may carry that show how it looks.
const PREVIEW_FILES: &[&str] = &["preview.png", "preview.jpg", "preview.jpeg", "screenshot.png", "background.png"];

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ThemeInfo {
    /// The name of the folder, which is the name Plymouth knows it by.
    pub name: String,
    pub description: String,
    pub module: String,
    pub path: String,
    pub has_preview: bool,
    /// False for a theme that only writes text to the screen.
    pub is_graphical: bool,
}

/// The values of the [Plymouth Theme] section of a .plymouth file that matter
/// here: the description and the module that draws the theme.
fn parse_plymouth_file(text: &str) -> (String, String) {
    let mut description = String::new();
    let mut module = String::new();
    let mut in_section = false;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_section = line.eq_ignore_ascii_case("[Plymouth Theme]");
            continue;
        }
        if !in_section {
            continue;
        }
        if let Some((key, value)) = line.split_once('=') {
            match key.trim().to_lowercase().as_str() {
                "description" => description = value.trim().to_string(),
                "modulename" => module = value.trim().to_lowercase(),
                _ => {}
            }
        }
    }
    (description, module)
}

// Whether the folder has images or a script, down to two levels, for a theme
// that does not say which module it uses.
fn has_visual_assets(dir: &Path) -> bool {
    fn walk(dir: &Path, depth: u8) -> bool {
        let Ok(entries) = std::fs::read_dir(dir) else { return false };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                if depth > 0 && walk(&path, depth - 1) {
                    return true;
                }
                continue;
            }
            let name = path.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
            if name == "preview.png" || name == "thumbnail.png" {
                continue;
            }
            if name.ends_with(".png") || name.ends_with(".jpg") || name.ends_with(".jpeg") || name.ends_with(".script") {
                return true;
            }
        }
        false
    }
    walk(dir, 2)
}

/// Text and details themes only print; any other module draws something.
fn is_graphical(module: &str, dir: &Path) -> bool {
    match module {
        "text" | "details" => false,
        "" => has_visual_assets(dir),
        _ => true,
    }
}

fn plymouth_file_of(dir: &Path) -> Option<PathBuf> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_file() && path.extension().map(|e| e == "plymouth").unwrap_or(false))
        .collect();
    files.sort();
    files.into_iter().next()
}

fn preview_file_of(dir: &Path) -> Option<PathBuf> {
    PREVIEW_FILES.iter().map(|name| dir.join(name)).find(|path| path.is_file())
}

fn read_themes(dirs: &[&str]) -> Vec<ThemeInfo> {
    let mut themes: Vec<ThemeInfo> = Vec::new();
    for base in dirs {
        let Ok(entries) = std::fs::read_dir(base) else { continue };
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let Some(config) = plymouth_file_of(&path) else { continue };
            let name = entry.file_name().to_string_lossy().to_string();
            // The same theme in two folders counts once: the first one wins.
            if themes.iter().any(|theme| theme.name == name) {
                continue;
            }
            let text = std::fs::read_to_string(&config).unwrap_or_default();
            let (description, module) = parse_plymouth_file(&text);
            themes.push(ThemeInfo {
                description: if description.is_empty() { name.replace('-', " ") } else { description },
                is_graphical: is_graphical(&module, &path),
                has_preview: preview_file_of(&path).is_some(),
                path: path.to_string_lossy().to_string(),
                module,
                name,
            });
        }
    }
    themes.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    themes
}

/// One installed theme, by the name of its folder.
pub fn theme_info(name: &str) -> Option<ThemeInfo> {
    read_themes(THEME_DIRS).into_iter().find(|theme| theme.name == name)
}

#[tauri::command]
pub async fn list_themes() -> Vec<ThemeInfo> {
    tauri::async_runtime::spawn_blocking(|| read_themes(THEME_DIRS)).await.unwrap_or_default()
}

/// The theme the system uses, from plymouthd.conf, or from where the default
/// link points when the file does not say.
fn current_theme_from(conf: Option<&str>, link: Option<&Path>) -> Option<String> {
    if let Some(text) = conf {
        let mut in_daemon = false;
        for line in text.lines() {
            let line = line.trim();
            if line.starts_with('[') {
                in_daemon = line.eq_ignore_ascii_case("[Daemon]");
                continue;
            }
            if in_daemon {
                if let Some((key, value)) = line.split_once('=') {
                    if key.trim().eq_ignore_ascii_case("Theme") && !value.trim().is_empty() {
                        return Some(value.trim().to_string());
                    }
                }
            }
        }
    }
    link?.parent()?.file_name().map(|name| name.to_string_lossy().to_string())
}

#[tauri::command]
pub async fn get_current_theme() -> Option<String> {
    tauri::async_runtime::spawn_blocking(|| {
        let conf = std::fs::read_to_string(PLYMOUTHD_CONF).ok();
        let link = std::fs::read_link(DEFAULT_LINK).ok();
        current_theme_from(conf.as_deref(), link.as_deref())
    })
    .await
    .ok()
    .flatten()
}

const BASE64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

fn base64(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = (u32::from(chunk[0]) << 16) | (u32::from(*chunk.get(1).unwrap_or(&0)) << 8) | u32::from(*chunk.get(2).unwrap_or(&0));
        out.push(BASE64[(n >> 18) as usize & 63] as char);
        out.push(BASE64[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 { BASE64[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if chunk.len() > 2 { BASE64[n as usize & 63] as char } else { '=' });
    }
    out
}

fn image_data_url(file: &Path) -> Option<String> {
    let mime = match file.extension()?.to_string_lossy().to_lowercase().as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        _ => return None,
    };
    let meta = std::fs::metadata(file).ok()?;
    if meta.len() > MAX_PREVIEW_BYTES {
        return None;
    }
    let bytes = std::fs::read(file).ok()?;
    Some(format!("data:{mime};base64,{}", base64(&bytes)))
}

/// The picture of a theme: the preview.png in its folder, which is the one the
/// preview makes (as in the 2.x program) or the one the theme brings.
fn preview_data_url(dir: &Path) -> Option<String> {
    let file = preview_file_of(dir)?;
    image_data_url(&file)
}

/// The image of one theme, asked for when its card is shown, so the list
/// itself does not have to carry every image at once.
#[tauri::command]
pub async fn read_theme_preview(name: String) -> Option<String> {
    if name.is_empty() || name.contains(['/', '\\', '\0']) || name.starts_with('.') {
        return None;
    }
    tauri::async_runtime::spawn_blocking(move || {
        let folder = THEME_DIRS.iter().map(|base| Path::new(base).join(&name)).find(|dir| dir.is_dir())?;
        preview_data_url(&folder)
    })
    .await
    .ok()
    .flatten()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("soplos-plymouth-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn the_description_and_the_module_come_from_the_theme_section_only() {
        let text = "[Plymouth Theme]\nName=Soplos\nDescription=Soplos Theme\nModuleName=Script\n\n[script]\nDescription=not this one\nModuleName=other\n";
        assert_eq!(parse_plymouth_file(text), ("Soplos Theme".to_string(), "script".to_string()));
        assert_eq!(parse_plymouth_file(""), (String::new(), String::new()));
    }

    #[test]
    fn text_and_details_are_not_graphical_and_any_other_module_is() {
        let dir = scratch("modules");
        assert!(!is_graphical("text", &dir));
        assert!(!is_graphical("details", &dir));
        assert!(is_graphical("script", &dir));
        assert!(is_graphical("two-step", &dir));
        // No module: it depends on what the folder holds.
        assert!(!is_graphical("", &dir));
        std::fs::write(dir.join("logo.png"), b"x").unwrap();
        assert!(is_graphical("", &dir));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_folder_without_a_plymouth_file_is_not_a_theme_and_the_first_folder_wins() {
        let first = scratch("first");
        let second = scratch("second");
        for (base, description) in [(&first, "From the first"), (&second, "From the second")] {
            let theme = base.join("soplos");
            std::fs::create_dir_all(&theme).unwrap();
            std::fs::write(theme.join("soplos.plymouth"), format!("[Plymouth Theme]\nDescription={description}\nModuleName=script\n")).unwrap();
            std::fs::create_dir_all(base.join("not-a-theme")).unwrap();
        }
        std::fs::write(second.join("soplos").join("preview.png"), b"x").unwrap();
        let themes = read_themes(&[first.to_str().unwrap(), second.to_str().unwrap()]);
        assert_eq!(themes.len(), 1);
        assert_eq!(themes[0].description, "From the first");
        assert!(!themes[0].has_preview);
        let _ = std::fs::remove_dir_all(&first);
        let _ = std::fs::remove_dir_all(&second);
    }

    #[test]
    fn the_current_theme_is_the_one_of_the_conf_or_where_the_link_points() {
        let conf = "[Daemon]\nTheme=bgrt\nShowDelay=0\n";
        assert_eq!(current_theme_from(Some(conf), None).as_deref(), Some("bgrt"));
        assert_eq!(current_theme_from(Some("[Daemon]\nShowDelay=0\n"), Some(Path::new("/usr/share/plymouth/themes/soplos/soplos.plymouth"))).as_deref(), Some("soplos"));
        assert_eq!(current_theme_from(Some("Theme=other\n"), None), None);
        assert_eq!(current_theme_from(None, None), None);
    }

    #[test]
    fn base64_matches_the_known_encodings() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn a_name_that_leaves_the_themes_folder_has_no_preview() {
        let result = tauri::async_runtime::block_on(read_theme_preview("../../etc".to_string()));
        assert!(result.is_none());
    }
}
