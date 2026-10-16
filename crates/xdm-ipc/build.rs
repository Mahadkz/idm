use std::process::Command;

fn main() {
    let commit = Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()
        .filter(|out| out.status.success())
        .and_then(|out| String::from_utf8(out.stdout).ok())
        .map(|text| text.trim().to_owned())
        .filter(|text| !text.is_empty())
        .unwrap_or_else(|| "unknown".into());
    println!("cargo:rustc-env=XDM_GIT_COMMIT={commit}");
    println!("cargo:rerun-if-changed=build.rs");
    // Re-embed when the checked-out commit changes (works in worktrees too).
    if let Ok(out) = Command::new("git").args(["rev-parse", "--git-path", "HEAD"]).output() {
        if let Ok(path) = String::from_utf8(out.stdout) {
            println!("cargo:rerun-if-changed={}", path.trim());
        }
    }
    // HEAD only names a branch; the commit lives in the branch ref (or packed-refs), so watch those too.
    if let Ok(out) = Command::new("git").args(["symbolic-ref", "-q", "HEAD"]).output()
        && out.status.success()
        && let Ok(name) = String::from_utf8(out.stdout)
    {
        for git_path in [name.trim().to_owned(), "packed-refs".to_owned()] {
            if let Ok(out) = Command::new("git").args(["rev-parse", "--git-path", &git_path]).output()
                && let Ok(path) = String::from_utf8(out.stdout)
            {
                println!("cargo:rerun-if-changed={}", path.trim());
            }
        }
    }
    println!("cargo:rerun-if-env-changed=XDM_LOG");
}
