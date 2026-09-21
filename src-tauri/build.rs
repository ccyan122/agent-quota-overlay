fn main() {
    // `quota-probe` is a read-only diagnostic and does not need Tauri's
    // generated application manifest.  Keeping this escape hatch makes it
    // possible to validate locally stored sign-ins even when desktop bundle
    // assets are unavailable during an early bootstrap.
    if std::env::var_os("CARGO_FEATURE_DESKTOP").is_some() && std::env::var_os("LLM_QUOTA_PROBE_BUILD").is_none() {
        tauri_build::build()
    }
}
