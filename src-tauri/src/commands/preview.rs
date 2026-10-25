// The preview of a theme: the user's side of it. The 2.x program did all of it
// in shell scripts; the mechanics are the same, written here:
//
// - A theme that draws on a screen is shown in a nested X server (Xephyr, full
//   screen, so it works the same on X11 and, through XWayland, on Wayland).
//   plymouthd runs on it as root, through the helper, and the picture of the
//   nested screen is taken here, as the user.
// - A theme that uses the text console (tribar, bgrt) is shown in the
//   terminal of a full screen xterm, where the helper runs as root, and the
//   picture of the window of the xterm is taken.
//
// The picture ends as the preview.png of the folder of the theme, as in the
// 2.x program, so it goes away with the theme. The user cannot write there: the
// picture is left in the runtime folder and the helper, which is root, installs
// it.
//
// What is not as in the 2.x scripts: the picture is taken a few seconds after
// the helper says the splash is on screen, not a fixed time after the nested
// server started, which a slow password would have spent; and the X authority
// file of the text mode is a private copy in the runtime folder, not one in
// /tmp that anybody can read.

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use super::environment::find_tool;
use super::jobs::JobLog;
use super::privileged::{spawn_helper, valid_theme_name, Op};
use super::themes;

// The picture is taken this long after the splash is on screen. The 2.x
// scripts took it about four seconds after the splash was asked for.
const CAPTURE_DELAY: Duration = Duration::from_secs(4);
// The nested server takes a moment to be ready, as in the 2.x script.
const XEPHYR_START_WAIT: Duration = Duration::from_secs(2);
// How long the user has to type the password before the preview gives up.
const PASSWORD_WAIT: Duration = Duration::from_secs(120);
const DASH_SCHEMA: &str = "org.gnome.shell.extensions.dash-to-panel";

fn sleep(duration: Duration) {
    std::thread::sleep(duration);
}

fn quiet(program: &Path, args: &[&str]) -> bool {
    Command::new(program).args(args).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).status().map(|s| s.success()).unwrap_or(false)
}

/// The lowest display number nobody uses, as the 2.x script looked for it.
fn free_display() -> u32 {
    (1..=200).find(|n| !Path::new(&format!("/tmp/.X11-unix/X{n}")).exists() && !Path::new(&format!("/tmp/.X{n}-lock")).exists()).unwrap_or(10)
}

fn is_gnome() -> bool {
    std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default().to_lowercase().contains("gnome")
}

/// Takes the picture of a whole X display.
fn capture_display(display: &str, out: &Path) -> bool {
    let out_text = out.to_string_lossy().into_owned();
    if let Some(scrot) = find_tool("scrot") {
        let ok = Command::new(scrot).args(["-o", &out_text]).env("DISPLAY", display).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).status().map(|s| s.success()).unwrap_or(false);
        return ok && file_has_content(out);
    }
    if let Some(import) = find_tool("import") {
        let ok = quiet(&import, &["-display", display, "-window", "root", &out_text]);
        return ok && file_has_content(out);
    }
    false
}

fn file_has_content(path: &Path) -> bool {
    std::fs::metadata(path).map(|m| m.is_file() && m.len() > 0).unwrap_or(false)
}

/// What has to be put back when the nested display goes away, however it goes
/// away: the server is stopped, root's access to the user's X is closed again
/// and the GNOME setting that was changed for the preview is restored.
struct NestedDisplay {
    xephyr: Option<Child>,
    xhost: Option<PathBuf>,
    dash_to_panel: Option<String>,
}

impl Drop for NestedDisplay {
    fn drop(&mut self) {
        if let Some(mut xephyr) = self.xephyr.take() {
            let _ = xephyr.kill();
            let _ = xephyr.wait();
        }
        if let Some(xhost) = self.xhost.take() {
            quiet(&xhost, &["-local:root"]);
        }
        if let Some(original) = self.dash_to_panel.take() {
            if let Some(gsettings) = find_tool("gsettings") {
                quiet(&gsettings, &["set", DASH_SCHEMA, "intellihide", &original]);
            }
        }
    }
}

fn missing(log: &JobLog, program: &str) -> bool {
    log.step(&format!("missing {program}"));
    false
}

/// A picture is taken into `partial` and only becomes `picture` when it is
/// complete, which is what the helper waits for.
fn complete(partial: &Path, picture: &Path) -> bool {
    file_has_content(partial) && std::fs::rename(partial, picture).is_ok()
}

fn x11_preview(theme: &str, log: JobLog, partial: &Path, picture: &Path) -> bool {
    if std::env::var_os("DISPLAY").is_none() {
        log.step("noDisplay");
        return false;
    }
    let Some(xephyr) = find_tool("Xephyr") else { return missing(&log, "Xephyr") };
    let Some(xhost) = find_tool("xhost") else { return missing(&log, "xhost") };
    if find_tool("scrot").is_none() && find_tool("import").is_none() {
        return missing(&log, "scrot");
    }
    let Ok(runtime) = std::env::var("XDG_RUNTIME_DIR") else {
        log.line("XDG_RUNTIME_DIR is not set");
        return false;
    };

    let mut nested = NestedDisplay { xephyr: None, xhost: None, dash_to_panel: None };
    let number = free_display();
    let display = format!(":{number}");

    log.step("opening");
    // Root has to be allowed on the user's X for plymouthd to reach the nested
    // display: closed again when the preview ends.
    quiet(&xhost, &["+local:root"]);
    nested.xhost = Some(xhost);
    match Command::new(xephyr).args([display.as_str(), "-fullscreen", "-no-host-grab"]).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn() {
        Ok(child) => nested.xephyr = Some(child),
        Err(e) => {
            log.line(&format!("Xephyr could not be started: {e}"));
            log.step("xephyrFailed");
            return false;
        }
    }
    sleep(XEPHYR_START_WAIT);
    if let Some(xephyr) = nested.xephyr.as_mut() {
        if xephyr.try_wait().ok().flatten().is_some() {
            log.step("xephyrFailed");
            return false;
        }
    }

    // On GNOME the panel of dash-to-panel would stay over the nested screen:
    // it is hidden for the preview.
    if is_gnome() {
        if let Some(gsettings) = find_tool("gsettings") {
            if let Ok(output) = Command::new(&gsettings).args(["get", DASH_SCHEMA, "intellihide"]).stdin(Stdio::null()).stderr(Stdio::null()).output() {
                let original = String::from_utf8_lossy(&output.stdout).trim().to_string();
                if output.status.success() && !original.is_empty() {
                    quiet(&gsettings, &["set", DASH_SCHEMA, "intellihide", "true"]);
                    nested.dash_to_panel = Some(original);
                }
            }
        }
    }

    let ops = vec![Op::PreviewX11 { theme: theme.to_string(), display: display.clone(), xdg_runtime: runtime, picture: picture.to_string_lossy().into_owned() }];
    let mut helper = match spawn_helper(&ops, None) {
        Ok(helper) => helper,
        Err(e) => {
            log.line(&e);
            return false;
        }
    };

    let stderr_reader = helper.stderr.take().map(|stderr| {
        std::thread::spawn(move || {
            for line in BufReader::new(stderr).lines().flatten() {
                log.line(&line);
            }
        })
    });
    let mut capture: Option<std::thread::JoinHandle<bool>> = None;
    if let Some(stdout) = helper.stdout.take() {
        for line in BufReader::new(stdout).lines().flatten() {
            if line == "@@shown" {
                log.step(&format!("showing {theme}"));
                let display = display.clone();
                let partial = partial.to_path_buf();
                let picture = picture.to_path_buf();
                capture = Some(std::thread::spawn(move || {
                    sleep(CAPTURE_DELAY);
                    capture_display(&display, &partial) && complete(&partial, &picture)
                }));
            } else {
                log.line(&line);
            }
        }
    }
    let helper_ok = helper.wait().map(|s| s.success()).unwrap_or(false);
    if let Some(reader) = stderr_reader {
        let _ = reader.join();
    }
    let captured = capture.map(|handle| handle.join().unwrap_or(false)).unwrap_or(false);
    drop(nested);
    if !captured {
        log.step("captureFailed");
    }
    helper_ok && captured
}

/// The window id of the last xterm, which is the one just opened.
fn xterm_window(env: &[(&str, &str)]) -> Option<String> {
    let xdotool = find_tool("xdotool")?;
    let mut command = Command::new(xdotool);
    command.args(["search", "--class", "xterm"]).stdin(Stdio::null()).stderr(Stdio::null());
    for (key, value) in env {
        command.env(key, value);
    }
    let output = command.output().ok()?;
    String::from_utf8_lossy(&output.stdout).lines().map(str::trim).filter(|line| !line.is_empty()).last().map(str::to_string)
}

fn capture_window(window: &str, out: &Path, env: &[(&str, &str)]) -> bool {
    let out_text = out.to_string_lossy().into_owned();
    if let Some(import) = find_tool("import") {
        let mut command = Command::new(import);
        command.args(["-window", window, &out_text]).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
        for (key, value) in env {
            command.env(key, value);
        }
        if command.status().map(|s| s.success()).unwrap_or(false) && file_has_content(out) {
            return true;
        }
    }
    // Without ImageMagick, the focused window is the xterm that was just opened.
    if let Some(scrot) = find_tool("scrot") {
        let mut command = Command::new(scrot);
        command.args(["-u", "-o", &out_text]).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
        for (key, value) in env {
            command.env(key, value);
        }
        return command.status().map(|s| s.success()).unwrap_or(false) && file_has_content(out);
    }
    false
}

/// Removes the files the text preview shares with root, whatever happens.
struct SharedFiles(Vec<PathBuf>);

impl Drop for SharedFiles {
    fn drop(&mut self) {
        for path in &self.0 {
            let _ = std::fs::remove_file(path);
        }
    }
}

fn tty_preview(theme: &str, log: JobLog, partial: &Path, picture: &Path) -> bool {
    let Some(xterm) = find_tool("xterm") else { return missing(&log, "xterm") };
    if find_tool("xdotool").is_none() {
        return missing(&log, "xdotool");
    }
    if find_tool("import").is_none() && find_tool("scrot").is_none() {
        return missing(&log, "import");
    }
    let Ok(display) = std::env::var("DISPLAY") else {
        log.step("noDisplay");
        return false;
    };
    let Ok(runtime) = std::env::var("XDG_RUNTIME_DIR") else {
        log.line("XDG_RUNTIME_DIR is not set");
        return false;
    };
    let home = std::env::var("HOME").unwrap_or_default();
    let xauth_original = std::env::var("XAUTHORITY").unwrap_or_else(|_| format!("{home}/.Xauthority"));

    let id = format!("{}-{}", std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0));
    let xauthority = Path::new(&runtime).join(format!("soplos-plymouth-xauth-{id}"));
    let mark = Path::new(&runtime).join(format!("soplos-plymouth-{id}.mark"));
    let done = Path::new(&runtime).join(format!("soplos-plymouth-{id}.done"));
    let _shared = SharedFiles(vec![xauthority.clone(), mark.clone(), done.clone()]);

    // A private copy of the X authority file, in the runtime folder of the
    // user, readable only by them (root reads it anyway).
    if let Err(e) = std::fs::copy(&xauth_original, &xauthority) {
        log.line(&format!("Could not copy the X authority file: {e}"));
        return false;
    }
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&xauthority, std::fs::Permissions::from_mode(0o600));
    }
    let xauthority_text = xauthority.to_string_lossy().into_owned();

    let ops = vec![Op::PreviewTty {
        theme: theme.to_string(),
        display: display.clone(),
        xauthority: xauthority_text.clone(),
        xdg_runtime: runtime.clone(),
        mark: mark.to_string_lossy().into_owned(),
        done: done.to_string_lossy().into_owned(),
        picture: picture.to_string_lossy().into_owned(),
    }];
    // The request file is written here and not by spawn_helper: the xterm runs
    // pkexec itself, with the terminal it is in, which only it knows.
    let request = match super::privileged::prepare_request(&ops) {
        Ok(request) => request,
        Err(e) => {
            log.line(&e);
            return false;
        }
    };

    log.step("opening");
    let env = [("DISPLAY", display.as_str()), ("XAUTHORITY", xauthority_text.as_str())];
    let mut window = match Command::new(&xterm)
        .args(["-fa", "Monospace", "-fs", "12", "-bg", "black", "-fg", "white", "+rv"])
        .args(["-xrm", "xterm*vt100.scrollBar: false", "-xrm", "xterm*vt100.visualBell: false", "-fullscreen", "-e", "bash", "-c"])
        .arg(r#"T=$(tty); exec pkexec "$1" --run-ops "$2" "$T""#)
        .arg("soplos-plymouth-manager")
        .arg(&request.0)
        .arg(&request.1)
        .env("DISPLAY", &display)
        .env("XAUTHORITY", &xauthority)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(child) => child,
        Err(e) => {
            log.line(&format!("xterm could not be started: {e}"));
            return false;
        }
    };

    // The splash is on screen when the helper creates the mark file.
    let started = Instant::now();
    let mut shown = false;
    while started.elapsed() < PASSWORD_WAIT {
        if mark.exists() {
            shown = true;
            break;
        }
        if window.try_wait().ok().flatten().is_some() {
            break;
        }
        sleep(Duration::from_millis(250));
    }
    let mut captured = false;
    if shown {
        log.step(&format!("showing {theme}"));
        sleep(CAPTURE_DELAY);
        if let Some(id) = xterm_window(&env) {
            captured = capture_window(&id, partial, &env) && complete(partial, picture);
        }
        // Whatever happened, the helper is told it can stop.
        let _ = std::fs::write(&done, b"");
    }
    // The xterm closes by itself when the helper ends.
    let waited = Instant::now();
    while waited.elapsed() < Duration::from_secs(20) {
        if window.try_wait().ok().flatten().is_some() {
            break;
        }
        sleep(Duration::from_millis(250));
    }
    let _ = window.kill();
    let _ = window.wait();
    if shown && !captured {
        log.step("captureFailed");
    }
    shown && captured
}

/// Shows a theme and takes the picture of it, which the helper leaves as the
/// preview.png of the theme. True when the picture was made and installed.
pub fn run(theme: String, log: JobLog) -> bool {
    if !valid_theme_name(&theme) {
        log.line("Not a valid theme name");
        return false;
    }
    let Some(info) = themes::theme_info(&theme) else {
        log.line("The theme is not installed");
        return false;
    };
    if !info.is_graphical {
        log.step("notPreviewable");
        return false;
    }
    let Ok(runtime) = std::env::var("XDG_RUNTIME_DIR") else {
        log.line("XDG_RUNTIME_DIR is not set");
        return false;
    };
    // scrot and import choose the image format from the extension, so the
    // unfinished picture has to end in .png too.
    let id = format!("{}-{}", std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0));
    let picture = Path::new(&runtime).join(format!("soplos-plymouth-picture-{id}.png"));
    let partial = Path::new(&runtime).join(format!("soplos-plymouth-picture-{id}.part.png"));
    let _shared = SharedFiles(vec![picture.clone(), partial.clone()]);

    // The themes that use the text console cannot be drawn on a nested X
    // server, so they get a terminal of their own.
    if matches!(info.module.as_str(), "tribar" | "bgrt") {
        tty_preview(&theme, log, &partial, &picture)
    } else {
        x11_preview(&theme, log, &partial, &picture)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_free_display_is_looked_for_from_one() {
        let number = free_display();
        assert!((1..=200).contains(&number) || number == 10);
        assert!(!Path::new(&format!("/tmp/.X11-unix/X{number}")).exists() || number == 10);
    }

    #[test]
    fn an_empty_or_missing_file_is_not_a_picture() {
        let dir = std::env::temp_dir().join(format!("soplos-plymouth-pic-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        assert!(!file_has_content(&dir.join("missing.png")));
        std::fs::write(dir.join("empty.png"), b"").unwrap();
        assert!(!file_has_content(&dir.join("empty.png")));
        std::fs::write(dir.join("a.png"), b"x").unwrap();
        assert!(file_has_content(&dir.join("a.png")));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_shared_files_are_removed_when_the_preview_ends() {
        let dir = std::env::temp_dir().join(format!("soplos-plymouth-shared-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("soplos-plymouth-x.mark");
        std::fs::write(&file, b"").unwrap();
        drop(SharedFiles(vec![file.clone(), dir.join("never-created")]));
        assert!(!file.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
