#![allow(dead_code)]

use std::fs;
use std::path::Path;
use std::process::{Command, Output};

/// Runs a command isolated from the user's git configuration.
pub fn isolated(mut command: Command) -> Output {
    command
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_AUTHOR_NAME", "Test")
        .env("GIT_AUTHOR_EMAIL", "test@example.com")
        .env("GIT_COMMITTER_NAME", "Test")
        .env("GIT_COMMITTER_EMAIL", "test@example.com")
        .output()
        .expect("command runs")
}

pub fn git(dir: &Path, args: &[&str]) -> String {
    let mut command = Command::new("git");
    command.arg("-C").arg(dir).args(args);
    let output = isolated(command);
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

pub fn write(dir: &Path, path: &str, text: &str) {
    let full = dir.join(path);
    if let Some(parent) = full.parent() {
        fs::create_dir_all(parent).expect("create fixture dir");
    }
    fs::write(full, text).expect("write fixture");
}
