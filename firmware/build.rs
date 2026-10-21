fn main() {
    // Keep defmt's linker sections only in builds that enable its logger.
    if std::env::var_os("CARGO_FEATURE_LOGGING").is_some() {
        println!("cargo:rustc-link-arg=-Tdefmt.x");
    }
}
