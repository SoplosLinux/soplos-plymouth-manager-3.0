// Privileged operations. The app never hands a shell script to root: the
// frontend asks for named operations, this module validates them on the user's
// side, writes them as JSON to a private file and re-invokes this same binary
// under pkexec as `--run-ops <file>`. The root side validates every operation
// again, against the live system, before running it (the file is only a
// request) and executes each one with a fixed argv, never through a shell.
// Several operations travel in one pkexec call, so removing three themes asks
// for the password once.
//
// What root does is what the 2.x program did with its shell scripts: set the
// default theme and rebuild the initramfs, unpack an archive and copy the
// theme into the themes folder, delete a theme folder, and show a theme in a
// nested X window (or in a terminal) so the user can see it and a picture of it
// can be taken. The root side prints `@@step <message> [argument]` lines, which
// the window translates, and `@@shown` when the preview is on screen.

use serde::{Deserialize, Serialize};
use std::io::Write;
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use super::jobs::{failed_job, start_job};
use super::themes::THEME_DIRS;

// Where an installed theme goes.
const INSTALL_DIR: &str = "/usr/share/plymouth/themes";
// The themes the system itself needs, which are never removed.
const CRITICAL_THEMES: &[&str] = &["text", "details", "spinner", "bgrt"];

const MAX_OPS_FILE_BYTES: u64 = 1_000_000;
const MAX_ARCHIVE_BYTES: u64 = 200 * 1024 * 1024;
const MAX_THEME_BYTES: u64 = 300 * 1024 * 1024;
const MAX_THEME_FILES: usize = 5000;

const ARCHIVE_EXTENSIONS: &[&str] = &[".zip", ".tar.gz", ".tgz", ".tar.xz", ".tar.bz2"];

// The times of the preview, those of the 2.x scripts: the daemon draws for
// ten seconds after the splash is asked for, and is stopped by force after
// thirty if something goes wrong.
const DAEMON_START_WAIT: Duration = Duration::from_secs(5);
const TTY_START_WAIT: Duration = Duration::from_secs(2);
const SPLASH_TIME: Duration = Duration::from_secs(10);
const DAEMON_LIMIT: Duration = Duration::from_secs(30);
const TTY_DONE_WAITS: u32 = 15;
// How long root waits for the picture of the preview to be there.
const PICTURE_WAITS: u32 = 5;
const MAX_PICTURE_BYTES: u64 = 20 * 1024 * 1024;
const PNG_MAGIC: &[u8] = b"\x89PNG\r\n\x1a\n";

#[derive(Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "kebab-case")]
pub enum Op {
    /// Sets the default theme and rebuilds the initramfs.
    ApplyTheme { name: String },
    /// Unpacks an archive and installs the theme it holds.
    InstallTheme { archive: String },
    /// Deletes an installed theme.
    RemoveTheme { name: String },
    /// Shows a theme in the nested X display the user opened, for the themes
    /// that draw on a screen. `picture` is where the user leaves the picture
    /// of the splash, which is installed as the preview.png of the theme.
    PreviewX11 { theme: String, display: String, xdg_runtime: String, picture: String },
    /// Shows a theme in the terminal the user opened (its path comes as an
    /// argument), for the themes that use the text console. `mark` is created
    /// when the splash is on screen and `done` by the user once the picture is
    /// taken.
    PreviewTty { theme: String, display: String, xauthority: String, xdg_runtime: String, mark: String, done: String, picture: String },
}

// ---------------------------------------------------------------------------
// Checks of a request that need no disk

/// A theme name is made of letters, digits and . _ + - only: it becomes a folder
/// and an argument, so nothing else is accepted.
pub fn valid_theme_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && !name.starts_with(['.', '-'])
        && name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '+' | '-'))
}

fn valid_display(display: &str) -> bool {
    display.strip_prefix(':').map(|n| !n.is_empty() && n.len() <= 3 && n.chars().all(|c| c.is_ascii_digit())).unwrap_or(false)
}

fn absolute_without_parent(raw: &str) -> bool {
    raw.len() <= 4096 && !raw.contains('\0') && Path::new(raw).is_absolute() && !Path::new(raw).components().any(|c| matches!(c, std::path::Component::ParentDir))
}

fn valid_runtime_dir_shape(dir: &str) -> bool {
    absolute_without_parent(dir) && dir.starts_with("/run/user/") && dir.matches('/').count() == 3
}

// A file the preview shares with root: it lives directly in the runtime folder
// of the user and has a name of this program.
fn valid_runtime_file(path: &str, runtime: &str) -> bool {
    absolute_without_parent(path)
        && Path::new(path).parent() == Some(Path::new(runtime))
        && Path::new(path).file_name().map(|n| n.to_string_lossy().starts_with("soplos-plymouth-")).unwrap_or(false)
}

// The picture of a preview is a shared file that ends in .png.
fn valid_picture_file(path: &str, runtime: &str) -> bool {
    valid_runtime_file(path, runtime) && path.ends_with(".png")
}

fn valid_tty(tty: &str) -> bool {
    tty.strip_prefix("/dev/pts/").map(|n| !n.is_empty() && n.len() <= 6 && n.chars().all(|c| c.is_ascii_digit())).unwrap_or(false)
}

fn archive_extension(path: &str) -> Option<&'static str> {
    let lower = path.to_lowercase();
    ARCHIVE_EXTENSIONS.iter().copied().find(|ext| lower.ends_with(ext))
}

/// User-side check: the shape of a request.
pub fn validate(op: &Op) -> Result<(), String> {
    match op {
        Op::ApplyTheme { name } | Op::RemoveTheme { name } => {
            if valid_theme_name(name) {
                Ok(())
            } else {
                Err(format!("Not a valid theme name: {name}"))
            }
        }
        Op::InstallTheme { archive } => {
            if absolute_without_parent(archive) && archive_extension(archive).is_some() {
                Ok(())
            } else {
                Err("Not a theme archive".into())
            }
        }
        Op::PreviewX11 { theme, display, xdg_runtime, picture } => {
            if valid_theme_name(theme) && valid_display(display) && valid_runtime_dir_shape(xdg_runtime) && valid_picture_file(picture, xdg_runtime) {
                Ok(())
            } else {
                Err("Not a valid preview request".into())
            }
        }
        Op::PreviewTty { theme, display, xauthority, xdg_runtime, mark, done, picture } => {
            if valid_theme_name(theme)
                && valid_display(display)
                && valid_runtime_dir_shape(xdg_runtime)
                && valid_runtime_file(xauthority, xdg_runtime)
                && valid_runtime_file(mark, xdg_runtime)
                && valid_runtime_file(done, xdg_runtime)
                && valid_picture_file(picture, xdg_runtime)
            {
                Ok(())
            } else {
                Err("Not a valid preview request".into())
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers of the root side

fn is_real_dir(path: &Path) -> bool {
    std::fs::symlink_metadata(path).map(|m| m.is_dir()).unwrap_or(false)
}

fn is_real_file(path: &Path) -> bool {
    std::fs::symlink_metadata(path).map(|m| m.is_file()).unwrap_or(false)
}

fn tool(names: &[&str]) -> Option<PathBuf> {
    ["/usr/sbin", "/usr/bin", "/sbin", "/bin"].iter().flat_map(|dir| names.iter().map(move |name| Path::new(dir).join(name))).find(|p| p.is_file())
}

fn caller_uid() -> Option<u32> {
    std::env::var("PKEXEC_UID").ok()?.parse().ok()
}

/// A folder only root can read, removed when it goes out of scope.
struct PrivateDir(PathBuf);

impl PrivateDir {
    fn new(label: &str) -> std::io::Result<PrivateDir> {
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
        let path = Path::new("/var/tmp").join(format!("soplos-plymouth-manager-{label}-{}-{nanos}", std::process::id()));
        std::fs::DirBuilder::new().mode(0o700).create(&path)?;
        Ok(PrivateDir(path))
    }
}

impl Drop for PrivateDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn run(program: &Path, args: &[&str]) -> bool {
    match Command::new(program).args(args).status() {
        Ok(status) => status.success(),
        Err(e) => {
            eprintln!("soplos-plymouth-manager: could not run {}: {e}", program.display());
            false
        }
    }
}

/// The folder of an installed theme: one that holds a .plymouth file.
fn installed_theme(name: &str) -> Option<PathBuf> {
    THEME_DIRS.iter().map(|base| Path::new(base).join(name)).find(|dir| is_real_dir(dir) && plymouth_file_stem(dir).is_some())
}

/// The name of the .plymouth file of a folder, without its extension.
fn plymouth_file_stem(dir: &Path) -> Option<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .filter(|entry| std::fs::symlink_metadata(entry.path()).map(|m| m.is_file()).unwrap_or(false))
        .filter_map(|entry| entry.file_name().to_string_lossy().strip_suffix(".plymouth").map(str::to_string))
        .collect();
    names.sort();
    names.into_iter().next()
}

/// The theme the system uses now, as the system says it.
fn current_theme(set: &Path) -> Option<String> {
    let output = Command::new(set).stdin(Stdio::null()).stderr(Stdio::null()).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (!text.is_empty()).then_some(text)
}

/// Puts the theme that was the default back when the preview ends, however it
/// ends: the preview changes the default for a moment and must leave it as it
/// was.
struct ThemeGuard {
    set: PathBuf,
    previous: Option<String>,
}

impl Drop for ThemeGuard {
    fn drop(&mut self) {
        if let Some(previous) = &self.previous {
            let _ = Command::new(&self.set).arg(previous).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).status();
        }
    }
}

/// Stops every plymouthd running, as the 2.x scripts did before a preview: a
/// daemon left by an earlier preview would keep a new one from starting.
fn kill_plymouthd() {
    let Ok(entries) = std::fs::read_dir("/proc") else { return };
    for entry in entries.flatten() {
        let Some(pid) = entry.file_name().to_str().and_then(|name| name.parse::<i32>().ok()) else { continue };
        let Ok(raw) = std::fs::read(format!("/proc/{pid}/cmdline")) else { continue };
        let first = raw.split(|byte| *byte == 0).next().unwrap_or(&[]);
        if first.ends_with(b"plymouthd") {
            // SAFETY: kill(2) with a process id read from /proc and a signal
            // number; it touches no memory of this process.
            unsafe {
                libc::kill(pid, libc::SIGKILL);
            }
        }
    }
}

/// Stops the daemon by force after `after`, unless the preview has ended
/// before: the safeguard of the 2.x scripts, `(sleep 30; kill -9 $P) &`.
fn watchdog(pid: u32, finished: Arc<AtomicBool>, after: Duration) {
    std::thread::spawn(move || {
        std::thread::sleep(after);
        if !finished.load(Ordering::SeqCst) {
            // SAFETY: as in kill_plymouthd.
            unsafe {
                libc::kill(pid as i32, libc::SIGKILL);
            }
        }
    });
}

// ---------------------------------------------------------------------------
// The operations

/// The tool that rebuilds the initramfs, as the window shows it, with the
/// program and its arguments: dracut is what Soplos uses, initramfs-tools is
/// what Debian uses.
fn initramfs_command() -> Option<(&'static str, PathBuf, Vec<&'static str>)> {
    if let Some(program) = tool(&["dracut"]) {
        return Some(("dracut", program, vec!["--regenerate-all", "--force"]));
    }
    if let Some(program) = tool(&["update-initramfs"]) {
        return Some(("initramfs-tools", program, vec!["-u"]));
    }
    tool(&["mkinitcpio"]).map(|program| ("mkinitcpio", program, vec!["-P"]))
}

fn apply_theme(name: &str) -> bool {
    let Some(set) = tool(&["plymouth-set-default-theme"]) else {
        println!("@@step missing plymouth-set-default-theme");
        return false;
    };
    println!("@@step setTheme {name}");
    if !run(&set, &[name]) {
        return false;
    }
    match initramfs_command() {
        Some((label, program, args)) => {
            println!("@@step initramfs {label}");
            let _ = std::io::stdout().flush();
            if !run(&program, &args) {
                eprintln!("soplos-plymouth-manager: the initramfs could not be rebuilt");
                return false;
            }
        }
        None => println!("@@step noInitramfs"),
    }
    true
}

fn remove_theme(name: &str) -> bool {
    let Some(path) = installed_theme(name) else {
        eprintln!("soplos-plymouth-manager: {name} is not an installed theme");
        return false;
    };
    println!("@@step removing {name}");
    match std::fs::remove_dir_all(&path) {
        Ok(()) => true,
        Err(e) => {
            eprintln!("soplos-plymouth-manager: could not remove {name}: {e}");
            false
        }
    }
}

/// A name made only of letters, digits and . _ + -; anything else becomes _.
fn sanitize_name(raw: &str) -> String {
    let cleaned: String = raw.chars().map(|c| if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '+' | '-') { c } else { '_' }).collect();
    let trimmed = cleaned.trim_start_matches(['.', '-']);
    trimmed.chars().take(64).collect()
}

// What an unpacked theme is made of, or why it is refused: only regular files
// and folders, a bounded size and count.
fn check_tree(root: &Path) -> Result<(u64, usize), String> {
    fn walk(dir: &Path, total: &mut u64, count: &mut usize) -> Result<(), String> {
        for entry in std::fs::read_dir(dir).map_err(|e| e.to_string())?.flatten() {
            let meta = std::fs::symlink_metadata(entry.path()).map_err(|e| e.to_string())?;
            if meta.is_dir() {
                walk(&entry.path(), total, count)?;
            } else if meta.is_file() {
                *total += meta.len();
                *count += 1;
            } else {
                return Err(format!("The archive holds something that is not a plain file: {}", entry.path().display()));
            }
            if *total > MAX_THEME_BYTES || *count > MAX_THEME_FILES {
                return Err("The archive is too large for a theme".to_string());
            }
        }
        Ok(())
    }
    let mut total = 0;
    let mut count = 0;
    walk(root, &mut total, &mut count)?;
    Ok((total, count))
}

// The folder with the .plymouth file closest to the root of the unpacked
// archive, searched down to four levels as the 2.x installer did.
fn find_theme_dir(root: &Path) -> Option<(PathBuf, String)> {
    let mut level = vec![root.to_path_buf()];
    for _ in 0..5 {
        let mut next = Vec::new();
        for dir in &level {
            if let Some(stem) = plymouth_file_stem(dir) {
                return Some((dir.clone(), stem));
            }
            if let Ok(entries) = std::fs::read_dir(dir) {
                let mut dirs: Vec<PathBuf> = entries.flatten().map(|e| e.path()).filter(|p| is_real_dir(p)).collect();
                dirs.sort();
                next.extend(dirs);
            }
        }
        if next.is_empty() {
            break;
        }
        level = next;
    }
    None
}

fn copy_tree(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    std::fs::set_permissions(to, std::fs::Permissions::from_mode(0o755))?;
    for entry in std::fs::read_dir(from)?.flatten() {
        let source = entry.path();
        let target = to.join(entry.file_name());
        let meta = std::fs::symlink_metadata(&source)?;
        if meta.is_dir() {
            copy_tree(&source, &target)?;
        } else if meta.is_file() {
            std::fs::copy(&source, &target)?;
            std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o644))?;
        }
    }
    Ok(())
}

fn install_theme(archive: &str) -> bool {
    let archive_path = Path::new(archive);
    let Ok(meta) = std::fs::metadata(archive_path) else {
        eprintln!("soplos-plymouth-manager: cannot read the archive");
        return false;
    };
    if !meta.is_file() || meta.len() > MAX_ARCHIVE_BYTES {
        eprintln!("soplos-plymouth-manager: the archive is not a file or is too large");
        return false;
    }
    let Some(extension) = archive_extension(archive) else { return false };
    let Ok(work) = PrivateDir::new("theme") else {
        eprintln!("soplos-plymouth-manager: could not create a private folder");
        return false;
    };
    let unpacked = work.0.join("unpacked");
    if std::fs::create_dir(&unpacked).is_err() {
        return false;
    }
    let unpacked_text = unpacked.to_string_lossy().into_owned();
    let Some(timeout) = tool(&["timeout"]) else { return false };
    let extracted = if extension == ".zip" {
        match tool(&["unzip"]) {
            Some(unzip) => run(&timeout, &["120", &unzip.to_string_lossy(), "-q", "-o", archive, "-d", &unpacked_text]),
            None => {
                println!("@@step missing unzip");
                false
            }
        }
    } else {
        match tool(&["tar"]) {
            Some(tar) => run(&timeout, &["120", &tar.to_string_lossy(), "-xf", archive, "-C", &unpacked_text, "--no-same-owner", "--no-same-permissions"]),
            None => {
                println!("@@step missing tar");
                false
            }
        }
    };
    if !extracted {
        eprintln!("soplos-plymouth-manager: the archive could not be unpacked");
        return false;
    }
    if let Err(e) = check_tree(&unpacked) {
        eprintln!("soplos-plymouth-manager: {e}");
        return false;
    }
    let Some((theme_dir, stem)) = find_theme_dir(&unpacked) else {
        eprintln!("soplos-plymouth-manager: no .plymouth file found in the archive");
        return false;
    };
    // The folder of a theme is named after its .plymouth file: that is the
    // name plymouth-set-default-theme knows it by.
    let name = sanitize_name(&stem);
    if name.is_empty() {
        eprintln!("soplos-plymouth-manager: the theme has no usable name");
        return false;
    }
    if std::fs::create_dir_all(INSTALL_DIR).is_err() {
        return false;
    }
    let destination = Path::new(INSTALL_DIR).join(&name);
    if std::fs::symlink_metadata(&destination).is_ok() && (!is_real_dir(&destination) || std::fs::remove_dir_all(&destination).is_err()) {
        eprintln!("soplos-plymouth-manager: could not replace the installed theme {name}");
        return false;
    }
    if let Err(e) = copy_tree(&theme_dir, &destination) {
        let _ = std::fs::remove_dir_all(&destination);
        eprintln!("soplos-plymouth-manager: could not copy the theme: {e}");
        return false;
    }
    println!("@@step installed {name}");
    true
}

/// Starts plymouthd in preview mode, drawing on the display it is given.
fn start_daemon(plymouthd: &Path, extra: &[String], envs: &[(&str, &str)]) -> Option<Child> {
    let mut command = Command::new(plymouthd);
    command.args(["--mode=boot", "--no-daemon"]).args(extra).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    for (key, value) in envs {
        command.env(key, value);
    }
    match command.spawn() {
        Ok(child) => Some(child),
        Err(e) => {
            eprintln!("soplos-plymouth-manager: could not start plymouthd: {e}");
            None
        }
    }
}

fn show_splash() {
    if let Some(plymouth) = tool(&["plymouth"]) {
        let _ = Command::new(plymouth).arg("--show-splash").stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).status();
    }
}

/// Installs the picture the user took of the splash as the preview.png of the
/// theme, which is where the 2.x scripts left it: it goes away with the theme.
/// The file comes from the user, so it is opened without following links, must
/// be theirs, a PNG and of a bounded size, and it replaces the old preview in
/// one step.
fn install_picture(theme: &str, picture: &str) -> bool {
    let Some(uid) = caller_uid() else {
        eprintln!("soplos-plymouth-manager: not run through pkexec");
        return false;
    };
    let Some(dir) = installed_theme(theme) else {
        eprintln!("soplos-plymouth-manager: {theme} is not an installed theme");
        return false;
    };
    // The user takes the picture while the splash is on screen: it may still be
    // on its way when the daemon ends.
    for _ in 0..PICTURE_WAITS {
        if is_real_file(Path::new(picture)) {
            break;
        }
        std::thread::sleep(Duration::from_secs(1));
    }
    let mut file = match std::fs::OpenOptions::new().read(true).custom_flags(libc::O_NOFOLLOW).open(picture) {
        Ok(file) => file,
        Err(_) => {
            eprintln!("soplos-plymouth-manager: the picture of the preview was not made");
            return false;
        }
    };
    let Ok(meta) = file.metadata() else { return false };
    if !meta.is_file() || meta.uid() != uid || meta.len() == 0 || meta.len() > MAX_PICTURE_BYTES {
        eprintln!("soplos-plymouth-manager: the picture of the preview is not acceptable");
        return false;
    }
    let mut bytes = Vec::new();
    if std::io::Read::read_to_end(&mut std::io::Read::take(&mut file, MAX_PICTURE_BYTES + 1), &mut bytes).is_err() || !bytes.starts_with(PNG_MAGIC) {
        eprintln!("soplos-plymouth-manager: the picture of the preview is not a PNG");
        return false;
    }
    let temporary = dir.join(".preview.png.new");
    let target = dir.join("preview.png");
    let _ = std::fs::remove_file(&temporary);
    let written = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o644)
        .open(&temporary)
        .and_then(|mut out| out.write_all(&bytes))
        .and_then(|()| std::fs::rename(&temporary, &target));
    if let Err(e) = written {
        let _ = std::fs::remove_file(&temporary);
        eprintln!("soplos-plymouth-manager: could not save the preview in the theme: {e}");
        return false;
    }
    true
}

/// The preview of a theme in the nested X display, the 2.x script
/// (`killall plymouthd`, set the theme, `plymouthd --renderer=x11`, five
/// seconds, `plymouth --show-splash`, ten seconds, stop it, put the old theme
/// back) with one difference: it says `@@shown` when the splash is on screen,
/// so the picture is taken from then, not after a fixed time that a slow
/// password would have used up.
fn preview_x11(theme: &str, display: &str, runtime: &str, picture: &str) -> bool {
    let (Some(set), Some(plymouthd)) = (tool(&["plymouth-set-default-theme"]), tool(&["plymouthd"])) else {
        println!("@@step missing plymouthd");
        return false;
    };
    kill_plymouthd();
    let _guard = ThemeGuard { previous: current_theme(&set), set: set.clone() };
    if !run(&set, &[theme]) {
        return false;
    }
    let extra = vec!["--renderer=x11".to_string()];
    let Some(mut daemon) = start_daemon(&plymouthd, &extra, &[("DISPLAY", display), ("XDG_RUNTIME_DIR", runtime)]) else { return false };
    let finished = Arc::new(AtomicBool::new(false));
    watchdog(daemon.id(), finished.clone(), DAEMON_LIMIT);
    std::thread::sleep(DAEMON_START_WAIT);
    show_splash();
    println!("@@shown");
    let _ = std::io::stdout().flush();
    std::thread::sleep(SPLASH_TIME);
    let _ = daemon.kill();
    let _ = daemon.wait();
    finished.store(true, Ordering::SeqCst);
    install_picture(theme, picture)
}

/// The preview of a theme that uses the text console, the 2.x script: the
/// daemon runs on the terminal of the xterm the user opened. The file `mark` is
/// created when the splash is on screen, and the preview lasts until the user
/// creates `done` (once the picture is taken) or fifteen seconds pass.
fn preview_tty(theme: &str, display: &str, xauthority: &str, runtime: &str, mark: &str, done: &str, picture: &str, tty: &str) -> bool {
    let (Some(set), Some(plymouthd)) = (tool(&["plymouth-set-default-theme"]), tool(&["plymouthd"])) else {
        println!("@@step missing plymouthd");
        return false;
    };
    kill_plymouthd();
    let _guard = ThemeGuard { previous: current_theme(&set), set: set.clone() };
    if !run(&set, &[theme]) {
        return false;
    }
    let extra = vec!["--no-udev".to_string(), "--renderer=x11".to_string(), format!("--tty={tty}")];
    let envs = [("DISPLAY", display), ("XAUTHORITY", xauthority), ("XDG_RUNTIME_DIR", runtime), ("PATH", "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin")];
    let Some(mut daemon) = start_daemon(&plymouthd, &extra, &envs) else { return false };
    let finished = Arc::new(AtomicBool::new(false));
    watchdog(daemon.id(), finished.clone(), DAEMON_LIMIT);
    std::thread::sleep(TTY_START_WAIT);
    show_splash();
    if let Err(e) = std::fs::write(mark, b"") {
        eprintln!("soplos-plymouth-manager: could not create the mark file: {e}");
    }
    for _ in 0..TTY_DONE_WAITS {
        if Path::new(done).exists() {
            let _ = std::fs::remove_file(done);
            break;
        }
        std::thread::sleep(Duration::from_secs(1));
    }
    let _ = daemon.kill();
    let _ = daemon.wait();
    finished.store(true, Ordering::SeqCst);
    // The daemon leaves the terminal in a state of its own.
    let _ = Command::new("stty").arg("sane").status();
    install_picture(theme, picture)
}

// ---------------------------------------------------------------------------
// The root side

/// Root-side check, against the live system: the request is a file that
/// someone could have written, so what it names must be real.
fn validate_system(op: &Op, tty: Option<&str>) -> Result<(), String> {
    let uid = caller_uid();
    let runtime_ok = |dir: &str| match uid {
        Some(uid) => dir == format!("/run/user/{uid}"),
        None => false,
    };
    let display_ok = |display: &str| {
        let number = display.trim_start_matches(':');
        Path::new(&format!("/tmp/.X11-unix/X{number}")).exists()
    };
    match op {
        Op::ApplyTheme { name } => installed_theme(name).map(|_| ()).ok_or_else(|| format!("{name} is not an installed theme")),
        Op::InstallTheme { archive } => {
            let meta = std::fs::metadata(archive).map_err(|e| e.to_string())?;
            if meta.is_file() {
                Ok(())
            } else {
                Err("Not an archive".into())
            }
        }
        Op::RemoveTheme { name } => {
            installed_theme(name).ok_or_else(|| format!("{name} is not an installed theme"))?;
            if CRITICAL_THEMES.contains(&name.as_str()) {
                return Err(format!("{name} is a theme the system needs"));
            }
            if let Some(set) = tool(&["plymouth-set-default-theme"]) {
                if current_theme(&set).as_deref() == Some(name.as_str()) {
                    return Err(format!("{name} is the theme in use"));
                }
            }
            Ok(())
        }
        Op::PreviewX11 { theme, display, xdg_runtime, picture: _ } => {
            installed_theme(theme).ok_or_else(|| format!("{theme} is not an installed theme"))?;
            if !runtime_ok(xdg_runtime) {
                return Err("The runtime folder is not the one of the user".into());
            }
            if !display_ok(display) {
                return Err("The display does not exist".into());
            }
            Ok(())
        }
        Op::PreviewTty { theme, display, xauthority, xdg_runtime, mark, done, picture: _ } => {
            installed_theme(theme).ok_or_else(|| format!("{theme} is not an installed theme"))?;
            if !runtime_ok(xdg_runtime) {
                return Err("The runtime folder is not the one of the user".into());
            }
            if !display_ok(display) {
                return Err("The display does not exist".into());
            }
            let _ = (mark, done);
            let uid = uid.ok_or("Not run through pkexec")?;
            let owner_ok = |path: &str| std::fs::symlink_metadata(path).map(|m| m.is_file() && m.uid() == uid).unwrap_or(false);
            if !owner_ok(xauthority) {
                return Err("The X authority file is not the user's".into());
            }
            match tty {
                Some(tty) if valid_tty(tty) && std::fs::metadata(tty).map(|m| m.uid() == uid).unwrap_or(false) => Ok(()),
                _ => Err("Not a terminal of the user".into()),
            }
        }
    }
}

fn execute(op: &Op, tty: Option<&str>) -> bool {
    match op {
        Op::ApplyTheme { name } => apply_theme(name),
        Op::InstallTheme { archive } => install_theme(archive),
        Op::RemoveTheme { name } => remove_theme(name),
        Op::PreviewX11 { theme, display, xdg_runtime, picture } => preview_x11(theme, display, xdg_runtime, picture),
        Op::PreviewTty { theme, display, xauthority, xdg_runtime, mark, done, picture } => match tty {
            Some(tty) => preview_tty(theme, display, xauthority, xdg_runtime, mark, done, picture, tty),
            None => false,
        },
    }
}

/// Root side, called from main() before Tauri starts when pkexec re-invokes
/// this binary as `--run-ops <file> [tty]`. Returns the process exit code: 0
/// only when every operation succeeded.
pub fn run_ops_file(path: &str, tty: Option<&str>) -> i32 {
    let meta = match std::fs::symlink_metadata(path) {
        Ok(meta) => meta,
        Err(e) => {
            eprintln!("soplos-plymouth-manager: cannot read the operations file: {e}");
            return 1;
        }
    };
    // Only a regular file written by the user who authenticated is honoured.
    if !meta.file_type().is_file() || meta.len() > MAX_OPS_FILE_BYTES {
        eprintln!("soplos-plymouth-manager: refusing the operations file");
        return 1;
    }
    if let Ok(uid) = std::env::var("PKEXEC_UID") {
        if meta.uid().to_string() != uid {
            eprintln!("soplos-plymouth-manager: the operations file does not belong to the requesting user");
            return 1;
        }
    }
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) => {
            eprintln!("soplos-plymouth-manager: cannot read the operations file: {e}");
            return 1;
        }
    };
    let _ = std::fs::remove_file(path);

    let ops: Vec<Op> = match serde_json::from_str(&text) {
        Ok(ops) => ops,
        Err(e) => {
            eprintln!("soplos-plymouth-manager: invalid operations: {e}");
            return 1;
        }
    };

    for (index, op) in ops.iter().enumerate() {
        let allowed = validate(op).and_then(|()| validate_system(op, tty));
        if let Err(e) = allowed {
            eprintln!("soplos-plymouth-manager: refused operation {index}: {e}");
            return 1;
        }
        // A failed step stops the batch: the later ones were meant to follow it.
        if !execute(op, tty) {
            return 1;
        }
    }
    0
}

// ---------------------------------------------------------------------------
// The user's side: the window asks for operations

static NEXT_FILE_ID: AtomicU64 = AtomicU64::new(1);

// The request file lives in the user's runtime directory when there is one
// (private to the user), and is created with create_new and mode 0600 so
// nobody can pre-create or read it.
pub fn write_ops_file(json: &str) -> std::io::Result<PathBuf> {
    let dir = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    let id = NEXT_FILE_ID.fetch_add(1, Ordering::SeqCst);
    let path = dir.join(format!("soplos-plymouth-manager-ops-{}-{id}-{nanos}.json", std::process::id()));
    let mut file = std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(&path)?;
    file.write_all(json.as_bytes())?;
    Ok(path)
}

/// Validates the operations and writes the request file; the program to run
/// under pkexec is this one.
pub fn prepare_request(ops: &[Op]) -> Result<(String, PathBuf), String> {
    if ops.is_empty() {
        return Err("No operations given".to_string());
    }
    if let Some(error) = ops.iter().find_map(|op| validate(op).err()) {
        return Err(error);
    }
    let json = serde_json::to_string(ops).map_err(|e| format!("Could not encode the operations: {e}"))?;
    let file = write_ops_file(&json).map_err(|e| format!("Could not write the request: {e}"))?;
    let exe = std::env::current_exe().map(|p| p.to_string_lossy().into_owned()).unwrap_or_else(|_| "soplos-plymouth-manager".to_string());
    Ok((exe, file))
}

/// Runs the operations as administrator in one go (one password), as a job.
pub fn run_privileged_ops(ops: Vec<Op>) -> u64 {
    match prepare_request(&ops) {
        Ok((exe, file)) => start_job("pkexec", vec![exe, "--run-ops".to_string(), file.to_string_lossy().into_owned()]),
        Err(error) => failed_job(error),
    }
}

/// Starts the helper under pkexec and gives back the process, with its output
/// piped, for the preview, which has to follow it while it runs.
pub fn spawn_helper(ops: &[Op], tty_argument: Option<&str>) -> Result<Child, String> {
    let (exe, file) = prepare_request(ops)?;
    let mut command = Command::new("pkexec");
    command.arg(exe).arg("--run-ops").arg(&file);
    if let Some(tty) = tty_argument {
        command.arg(tty);
    }
    command.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().map_err(|e| {
        let _ = std::fs::remove_file(&file);
        format!("Could not run pkexec: {e}")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_theme_name_can_only_be_what_can_name_a_folder() {
        for name in ["soplos", "soplos-linux", "bgrt", "fade-in", "my_theme.2", "a+b"] {
            assert!(valid_theme_name(name), "{name}");
        }
        for name in ["", ".hidden", "-x", "a b", "a/b", "../x", "a;rm", "a\"b", "a$b", "a`b", &"x".repeat(65)] {
            assert!(!valid_theme_name(name), "{name}");
        }
    }

    #[test]
    fn a_display_is_a_colon_and_up_to_three_digits() {
        for display in [":1", ":10", ":199"] {
            assert!(valid_display(display), "{display}");
        }
        for display in ["", ":", "1", ":1.0", ":abc", ":1000", "host:1", ":1; rm"] {
            assert!(!valid_display(display), "{display}");
        }
    }

    #[test]
    fn the_shared_files_live_in_the_runtime_folder_with_a_name_of_the_program() {
        let runtime = "/run/user/1000";
        assert!(valid_runtime_dir_shape(runtime));
        assert!(!valid_runtime_dir_shape("/run/user/1000/x"));
        assert!(!valid_runtime_dir_shape("/tmp"));
        assert!(!valid_runtime_dir_shape("/run/user/../etc"));
        assert!(valid_runtime_file("/run/user/1000/soplos-plymouth-xauth-1", runtime));
        assert!(!valid_runtime_file("/run/user/1000/other", runtime));
        assert!(!valid_runtime_file("/run/user/1000/sub/soplos-plymouth-x", runtime));
        assert!(!valid_runtime_file("/tmp/soplos-plymouth-x", runtime));
        assert!(!valid_runtime_file("/run/user/1000/../soplos-plymouth-x", runtime));
    }

    #[test]
    fn a_terminal_is_a_pseudo_terminal_path() {
        assert!(valid_tty("/dev/pts/3"));
        assert!(!valid_tty("/dev/pts/"));
        assert!(!valid_tty("/dev/tty1"));
        assert!(!valid_tty("/dev/pts/3/../1"));
        assert!(!valid_tty("/dev/pts/a"));
    }

    #[test]
    fn only_known_archives_are_installed() {
        assert_eq!(archive_extension("/home/a/theme.TAR.GZ"), Some(".tar.gz"));
        assert_eq!(archive_extension("/home/a/theme.zip"), Some(".zip"));
        assert_eq!(archive_extension("/home/a/theme.rar"), None);
        assert!(validate(&Op::InstallTheme { archive: "/home/a/t.zip".into() }).is_ok());
        assert!(validate(&Op::InstallTheme { archive: "t.zip".into() }).is_err());
        assert!(validate(&Op::InstallTheme { archive: "/home/../etc/t.zip".into() }).is_err());
    }

    #[test]
    fn a_request_is_checked_before_it_leaves_the_window() {
        assert!(validate(&Op::ApplyTheme { name: "soplos".into() }).is_ok());
        assert!(validate(&Op::ApplyTheme { name: "so plos".into() }).is_err());
        assert!(validate(&Op::RemoveTheme { name: "../etc".into() }).is_err());
        let picture = "/run/user/1000/soplos-plymouth-picture-1.png";
        assert!(validate(&Op::PreviewX11 { theme: "soplos".into(), display: ":2".into(), xdg_runtime: "/run/user/1000".into(), picture: picture.into() }).is_ok());
        assert!(validate(&Op::PreviewX11 { theme: "soplos".into(), display: ":2".into(), xdg_runtime: "/tmp".into(), picture: picture.into() }).is_err());
        // The picture has to be a PNG of the program, in the runtime folder.
        assert!(validate(&Op::PreviewX11 { theme: "soplos".into(), display: ":2".into(), xdg_runtime: "/run/user/1000".into(), picture: "/etc/passwd".into() }).is_err());
        assert!(validate(&Op::PreviewX11 { theme: "soplos".into(), display: ":2".into(), xdg_runtime: "/run/user/1000".into(), picture: "/run/user/1000/soplos-plymouth-picture-1.txt".into() }).is_err());
    }

    #[test]
    fn the_names_of_an_archive_become_safe_names() {
        assert_eq!(sanitize_name("my theme (1)"), "my_theme__1_");
        assert_eq!(sanitize_name("..hidden"), "hidden");
        assert_eq!(sanitize_name("---"), "");
        assert_eq!(sanitize_name(&"x".repeat(100)).len(), 64);
    }

    #[test]
    fn the_operations_travel_as_json_and_come_back() {
        let ops = vec![Op::ApplyTheme { name: "soplos".into() }, Op::PreviewX11 { theme: "x".into(), display: ":2".into(), xdg_runtime: "/run/user/1".into(), picture: "/run/user/1/soplos-plymouth-picture-1.png".into() }];
        let json = serde_json::to_string(&ops).unwrap();
        assert!(json.contains("\"op\":\"apply-theme\""));
        assert!(json.contains("\"op\":\"preview-x11\""));
        let back: Vec<Op> = serde_json::from_str(&json).unwrap();
        assert_eq!(back.len(), 2);
    }

    #[test]
    fn a_tree_with_a_link_is_refused_and_a_plain_one_is_counted() {
        let dir = std::env::temp_dir().join(format!("soplos-plymouth-tree-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::write(dir.join("a.plymouth"), b"x").unwrap();
        std::fs::write(dir.join("sub").join("b.png"), b"xy").unwrap();
        assert_eq!(check_tree(&dir).unwrap(), (3, 2));
        let (found, stem) = find_theme_dir(&dir).unwrap();
        assert_eq!((found, stem), (dir.clone(), "a".to_string()));
        std::os::unix::fs::symlink("/etc/passwd", dir.join("link")).unwrap();
        assert!(check_tree(&dir).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
