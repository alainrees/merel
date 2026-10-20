//! Records which commit the app was built from, so "Over Mailcraft" can say exactly which build
//! runs. Outside a git checkout (a source archive) the values stay empty and only the version shows.

use std::path::Path;
use std::process::Command;

fn git(args: &[&str]) -> String {
    Command::new("git")
        .args(args)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .unwrap_or_default()
}

fn main() {
    // Build again when HEAD moves: a checkout changes HEAD, a commit changes the branch it names.
    let git_dir = git(&["rev-parse", "--git-dir"]);
    if !git_dir.is_empty() {
        let dir = Path::new(&git_dir);
        let head = dir.join("HEAD");
        println!("cargo:rerun-if-changed={}", head.display());
        if let Ok(text) = std::fs::read_to_string(&head)
            && let Some(name) = text.trim().strip_prefix("ref: ")
        {
            println!("cargo:rerun-if-changed={}", dir.join(name).display());
        }
        println!("cargo:rerun-if-changed={}", dir.join("packed-refs").display());
    }
    println!("cargo:rustc-env=MAILCRAFT_COMMIT={}", git(&["rev-parse", "--short=7", "HEAD"]));
    println!("cargo:rustc-env=MAILCRAFT_COMMIT_DATE={}", git(&["log", "-1", "--format=%cs"]));
}
