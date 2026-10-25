// The commands the window calls to change something: apply, install, remove
// and preview. Each one starts a job and answers with its id at once; the
// window follows it with poll_job. What changes the system runs as root,
// through the helper (privileged.rs); the preview has steps on the user's side
// (preview.rs).

use super::jobs::{failed_job, start_task};
use super::preview;
use super::privileged::{run_privileged_ops, Op};

#[tauri::command]
pub fn apply_theme(name: String) -> u64 {
    run_privileged_ops(vec![Op::ApplyTheme { name }])
}

#[tauri::command]
pub fn install_theme(archive: String) -> u64 {
    run_privileged_ops(vec![Op::InstallTheme { archive }])
}

/// Removes several themes with one password.
#[tauri::command]
pub fn remove_themes(names: Vec<String>) -> u64 {
    if names.is_empty() {
        return failed_job("No themes given".to_string());
    }
    run_privileged_ops(names.into_iter().map(|name| Op::RemoveTheme { name }).collect())
}

#[tauri::command]
pub fn preview_theme(name: String) -> u64 {
    start_task(move |log| preview::run(name, log))
}
