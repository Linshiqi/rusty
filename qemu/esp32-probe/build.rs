fn main() {
    // esp-hal ships the memory layout; this pulls it in last so its sections
    // win. No `-Wl,` prefix anywhere: this target links with rust-lld
    // directly, which reads that whole string as one unknown argument.
    println!("cargo:rustc-link-arg=-Tlinkall.x");
}
