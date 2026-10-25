// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // Privileged-helper mode: this same binary, re-invoked by pkexec under
    // the polkit action declared in org.soplos.plymouthmanager.policy, instead
    // of a separate helper executable. Must be checked before Tauri starts,
    // since this path never opens a window: it validates and runs the named
    // operations in the request file and exits. The text preview adds the
    // terminal it runs in as a last argument. See commands::privileged.
    let args: Vec<String> = std::env::args().collect();
    if args.len() >= 3 && args[1] == "--run-ops" {
        std::process::exit(soplos_plymouth_manager_lib::run_ops_file(&args[2], args.get(3).map(String::as_str)));
    }

    // WebKitGTK's DMA-BUF renderer causes protocol errors on Wayland with
    // NVIDIA/Nouveau drivers on physical hardware, as well as blank windows on
    // virtual GPUs. It is disabled by default on all hardware, as in Soplos
    // Welcome, unless the environment already says otherwise.
    if std::env::var_os("WEBKIT_DISABLE_DMABUF_RENDERER").is_none() {
        std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1");
    }

    soplos_plymouth_manager_lib::run()
}
