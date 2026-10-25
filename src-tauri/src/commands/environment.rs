// What the window needs to know about the system it runs on: the desktop,
// the display protocol (X11 or Wayland) and which tool builds the initramfs.
// Soplos uses dracut and Debian uses initramfs-tools, so both are supported;
// mkinitcpio is there for the distributions that have it. Nothing here
// changes anything: it only looks.

use serde::Serialize;
use std::path::PathBuf;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EnvironmentInfo {
    pub desktop_environment: String,
    pub display_protocol: String,
    /// The tool that rebuilds the initramfs: "dracut", "initramfs-tools",
    /// "mkinitcpio" or "none".
    pub initramfs: String,
    /// Every one of them found, since a system can have more than one.
    pub initramfs_available: Vec<String>,
}

// The sbin directories are looked at too: a graphical session often has no
// sbin in its PATH, and plymouthd and update-initramfs live there.
const EXTRA_DIRS: &[&str] = &["/usr/local/sbin", "/usr/sbin", "/sbin"];

/// The path of a program, looked for in PATH and in the sbin directories.
pub fn find_tool(name: &str) -> Option<PathBuf> {
    let mut dirs: Vec<PathBuf> = std::env::var_os("PATH").map(|path| std::env::split_paths(&path).collect()).unwrap_or_default();
    dirs.extend(EXTRA_DIRS.iter().map(PathBuf::from));
    dirs.into_iter().map(|dir| dir.join(name)).find(|candidate| candidate.is_file())
}

// The initramfs tools, in the order they are preferred, with the name each
// one is shown with. dracut comes first because it is what Soplos uses.
const INITRAMFS_TOOLS: &[(&str, &str)] = &[("dracut", "dracut"), ("update-initramfs", "initramfs-tools"), ("mkinitcpio", "mkinitcpio")];

/// The tool to use out of the programs found, or "none".
fn choose_initramfs(found: &[&str]) -> &'static str {
    INITRAMFS_TOOLS.iter().find(|(program, _)| found.contains(program)).map(|(_, name)| *name).unwrap_or("none")
}

fn desktop_from(xdg_current_desktop: &str, gnome_session: bool, kde_session: bool) -> &'static str {
    let current = xdg_current_desktop.to_lowercase();
    if current.contains("gnome") {
        "gnome"
    } else if current.contains("kde") || current.contains("plasma") {
        "kde"
    } else if current.contains("xfce") {
        "xfce"
    } else if gnome_session {
        "gnome"
    } else if kde_session {
        "kde"
    } else {
        "unknown"
    }
}

fn protocol_from(session_type: &str, wayland_display: bool, x_display: bool) -> &'static str {
    let session = session_type.to_lowercase();
    if session == "wayland" || wayland_display {
        "wayland"
    } else if session == "x11" || x_display {
        "x11"
    } else {
        "unknown"
    }
}

#[tauri::command]
pub async fn get_environment_info() -> EnvironmentInfo {
    tauri::async_runtime::spawn_blocking(|| {
        let desktop = desktop_from(
            &std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default(),
            std::env::var_os("GNOME_DESKTOP_SESSION_ID").is_some(),
            std::env::var_os("KDE_SESSION_VERSION").is_some(),
        );
        let protocol = protocol_from(
            &std::env::var("XDG_SESSION_TYPE").unwrap_or_default(),
            std::env::var_os("WAYLAND_DISPLAY").is_some(),
            std::env::var_os("DISPLAY").is_some(),
        );
        let found: Vec<&str> = INITRAMFS_TOOLS.iter().map(|(program, _)| *program).filter(|program| find_tool(program).is_some()).collect();
        EnvironmentInfo {
            desktop_environment: desktop.to_string(),
            display_protocol: protocol.to_string(),
            initramfs: choose_initramfs(&found).to_string(),
            initramfs_available: INITRAMFS_TOOLS.iter().filter(|(program, _)| found.contains(program)).map(|(_, name)| name.to_string()).collect(),
        }
    })
    .await
    .unwrap_or(EnvironmentInfo {
        desktop_environment: "unknown".to_string(),
        display_protocol: "unknown".to_string(),
        initramfs: "none".to_string(),
        initramfs_available: Vec::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dracut_is_preferred_and_initramfs_tools_is_still_supported() {
        assert_eq!(choose_initramfs(&["dracut"]), "dracut");
        assert_eq!(choose_initramfs(&["update-initramfs"]), "initramfs-tools");
        assert_eq!(choose_initramfs(&["update-initramfs", "dracut"]), "dracut");
        assert_eq!(choose_initramfs(&["mkinitcpio"]), "mkinitcpio");
        assert_eq!(choose_initramfs(&[]), "none");
    }

    #[test]
    fn the_desktop_comes_from_the_session_name_or_from_what_it_leaves_behind() {
        assert_eq!(desktop_from("XFCE", false, false), "xfce");
        assert_eq!(desktop_from("KDE", false, false), "kde");
        assert_eq!(desktop_from("ubuntu:GNOME", false, false), "gnome");
        assert_eq!(desktop_from("", true, false), "gnome");
        assert_eq!(desktop_from("", false, true), "kde");
        assert_eq!(desktop_from("", false, false), "unknown");
    }

    #[test]
    fn wayland_wins_over_a_display_variable_that_xwayland_also_sets() {
        assert_eq!(protocol_from("wayland", true, true), "wayland");
        assert_eq!(protocol_from("", true, true), "wayland");
        assert_eq!(protocol_from("x11", false, true), "x11");
        assert_eq!(protocol_from("", false, true), "x11");
        assert_eq!(protocol_from("", false, false), "unknown");
    }
}
