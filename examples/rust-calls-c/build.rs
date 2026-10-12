//! Compiles the C in csrc/ into this crate, for the chip this project targets.
//!
//! `cc` shells out to a *cross* compiler, and the one it picks by default for
//! a bare-metal RISC-V target is not a name that exists on most machines. So
//! it is named here — the same binary the Toolchain panel probes for and the
//! same one the C scaffold refuses without, so what the workbench says to
//! install is what this actually runs.

fn main() {
    println!("cargo:rustc-link-arg=-Tlinkall.x");
    println!("cargo:rerun-if-changed=csrc");

    let mut build = cc::Build::new();
    // Named explicitly because `cc`'s default guess for a bare-metal RISC-V
    // target is not a binary most machines have — but only when the caller
    // has not said otherwise, so `CC_riscv32imc_unknown_none_elf=…` still
    // works the way it does for every other crate.
    if std::env::var("CC_riscv32imc_unknown_none_elf").is_err()
        && std::env::var("CC").is_err()
    {
        build.compiler("riscv32-esp-elf-gcc");
    }
    // Freestanding: no libc, no host headers, and the ABI the Rust target
    // uses. Mismatching either produces a link error about incompatible
    // objects rather than anything about C.
    build
        .flag("-march=rv32imc_zicsr_zifencei")
        .flag("-mabi=ilp32")
        .flag("-ffreestanding")
        .flag("-nostdlib");

    for entry in std::fs::read_dir("csrc").expect("csrc/ exists").flatten() {
        let path = entry.path();
        if path.extension().is_some_and(|e| e == "c") {
            build.file(path);
        }
    }
    build.compile("pattern");
}
