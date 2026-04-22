fn main() {
    let version = std::env::var("WT_VERSION")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .map(|v| v.trim().trim_start_matches('v').trim_start_matches('V').to_string())
        .unwrap_or_else(|| format!("{}-dev", env!("CARGO_PKG_VERSION")));

    println!("cargo:rustc-env=WT_VERSION={version}");
    println!("cargo:rerun-if-env-changed=WT_VERSION");
}
