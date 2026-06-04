use std::process::Command;

fn main() {
    // 1) Prefer env var set by the CI / build context
    //    (GitHub Actions: --build-arg MEDIATRACKER_VERSION=$GITHUB_SHA_SHORT)
    let version = std::env::var("MEDIATRACKER_VERSION")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    // 2) Fallback: try git rev-parse (works locally when .git is present)
    let version = version.unwrap_or_else(|| {
        Command::new("git")
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
            .unwrap_or_else(|| "dev".to_string())
    });

    let dest = std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap())
        .join("static_version.txt");
    let _ = std::fs::write(&dest, &version);

    // Re-run when build.rs or env var changes.
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env=MEDIATRACKER_VERSION");
}
