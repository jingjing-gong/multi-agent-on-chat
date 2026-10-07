use std::process::Command;
fn git(args: &[&str]) -> String {
    Command::new("git")
        .args(args)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
        .unwrap_or_else(|| "unknown".into())
}
fn main() {
    println!("cargo:rerun-if-changed=.git/HEAD");
    println!("cargo:rerun-if-changed=.git/refs/heads");
    println!("cargo:rerun-if-changed=src");
    println!("cargo:rerun-if-changed=Cargo.toml");
    println!("cargo:rerun-if-env-changed=MAC_BUILD_RELEASE");
    let commit = git(&["rev-parse", "HEAD"]);
    let dirty = !git(&["status", "--porcelain"]).is_empty();
    let release = std::env::var("MAC_BUILD_RELEASE").unwrap_or_else(|_| {
        format!(
            "{}{}",
            &commit[..commit.len().min(12)],
            if dirty { "-dirty" } else { "" }
        )
    });
    println!("cargo:rustc-env=MAC_BUILD_COMMIT={commit}");
    println!("cargo:rustc-env=MAC_BUILD_DIRTY={dirty}");
    println!("cargo:rustc-env=MAC_BUILD_RELEASE_ID={release}");
}
