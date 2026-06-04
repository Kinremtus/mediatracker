use std::process::Command;

fn main() {
    // Write short git commit hash to $OUT_DIR/static_version.txt so the
    // binary can include_str! it and use as a cache-bust query string
    // for /static/* assets.
    let version = Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()
        .and_then(|o| if o.status.success() {
            String::from_utf8(o.stdout).ok()
        } else {
            None
        })
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "dev".to_string());

    let dest = std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap())
        .join("static_version.txt");
    let _ = std::fs::write(&dest, &version);

    // Rebuild when HEAD changes (cheap signal; rerun-if-changed on
    // .git/HEAD would also work but git rev-parse picks up new commits
    // automatically because we re-run on every build).
    println!("cargo:rerun-if-changed=build.rs");
}
