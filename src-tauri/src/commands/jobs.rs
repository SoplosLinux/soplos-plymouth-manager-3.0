// Shared job tracking: a job is work running on a background thread whose
// output is collected line by line, so the frontend can poll the live output
// and the final status. Two kinds: a command (the privileged helper, run under
// pkexec) and a task written in Rust (the preview, which has steps on the
// user's side and a step that needs root).
//
// Lines that start with `@@step ` are the program talking to the window: the
// word after it names a message the window translates, and the rest is its
// argument. Any other line is what a tool printed, shown as it is.

use std::collections::HashMap;
use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use serde::Serialize;

static NEXT_JOB_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JobStatus {
    pub running: bool,
    pub success: bool,
    pub output: String,
}

static JOBS: Mutex<Option<HashMap<u64, JobStatus>>> = Mutex::new(None);

fn jobs() -> std::sync::MutexGuard<'static, Option<HashMap<u64, JobStatus>>> {
    let mut guard = JOBS.lock().unwrap();
    if guard.is_none() {
        *guard = Some(HashMap::new());
    }
    guard
}

fn append_output_line(id: u64, line: &str) {
    if let Some(job) = jobs().as_mut().unwrap().get_mut(&id) {
        job.output.push_str(line);
        job.output.push('\n');
    }
}

fn finish(id: u64, success: bool) {
    if let Some(job) = jobs().as_mut().unwrap().get_mut(&id) {
        job.running = false;
        job.success = success;
    }
}

fn register() -> u64 {
    let id = NEXT_JOB_ID.fetch_add(1, Ordering::SeqCst);
    jobs().as_mut().unwrap().insert(id, JobStatus { running: true, success: false, output: String::new() });
    id
}

/// Where a task writes what it has to say.
#[derive(Clone, Copy)]
pub struct JobLog {
    id: u64,
}

impl JobLog {
    /// A line of output, as it is.
    pub fn line(&self, text: &str) {
        append_output_line(self.id, text);
    }

    /// A message for the window, such as `capturing` or `showing soplos`.
    pub fn step(&self, text: &str) {
        append_output_line(self.id, &format!("@@step {text}"));
    }
}

/// Runs `task` on a plain OS thread and returns the id of the job at once; the
/// task says whether it worked.
pub fn start_task(task: impl FnOnce(JobLog) -> bool + Send + 'static) -> u64 {
    let id = register();
    std::thread::spawn(move || {
        let success = task(JobLog { id });
        finish(id, success);
    });
    id
}

/// Runs a command on a plain OS thread, its output (stdout and stderr) going
/// into the job line by line as it arrives.
pub(crate) fn start_job(program: &str, args: Vec<String>) -> u64 {
    let id = register();
    let program = program.to_string();
    std::thread::spawn(move || {
        let child = Command::new(&program).args(&args).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn();
        let mut child = match child {
            Ok(child) => child,
            Err(e) => {
                append_output_line(id, &e.to_string());
                finish(id, false);
                return;
            }
        };

        let stdout = child.stdout.take().unwrap();
        let stderr = child.stderr.take().unwrap();
        let stdout_reader = std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines().flatten() {
                append_output_line(id, &line);
            }
        });
        let stderr_reader = std::thread::spawn(move || {
            for line in BufReader::new(stderr).lines().flatten() {
                append_output_line(id, &line);
            }
        });

        let exit_status = child.wait();
        let _ = stdout_reader.join();
        let _ = stderr_reader.join();
        finish(id, exit_status.map(|status| status.success()).unwrap_or(false));
    });
    id
}

/// A job that has already failed, with the reason as its output.
pub(crate) fn failed_job(message: String) -> u64 {
    let id = NEXT_JOB_ID.fetch_add(1, Ordering::SeqCst);
    jobs().as_mut().unwrap().insert(id, JobStatus { running: false, success: false, output: message });
    id
}

#[tauri::command]
pub fn poll_job(job_id: u64) -> JobStatus {
    let mut guard = jobs();
    let map = guard.as_mut().unwrap();
    match map.get(&job_id) {
        // The frontend stops polling as soon as it sees `running: false`, so
        // this is the one and only time it reads a finished job's status:
        // safe to drop it here instead of leaving it in the map forever.
        Some(status) if !status.running => map.remove(&job_id).unwrap(),
        Some(status) => status.clone(),
        None => JobStatus { running: false, success: false, output: "Unknown job".to_string() },
    }
}
