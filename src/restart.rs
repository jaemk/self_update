use std::process::Command;

pub fn restart() {
    // Get the current executable path
    let current_exe = std::env::current_exe().expect("Failed to get current executable path");

    // Launch a new instance of the application but do not wait for the command to complete!
    #[allow(clippy::zombie_processes)]
    let _ = Command::new(current_exe).spawn();
}
