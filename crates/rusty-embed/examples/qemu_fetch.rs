//! Prove the QEMU installer end to end: download (with mirror fallback),
//! extract, and run `--version` on the result.
//!
//! ```text
//! cargo run -p rusty-embed --example qemu_fetch -- qemu-system-xtensa
//! ```

fn main() {
    let name = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "qemu-system-riscv32".to_string());

    let plan = rusty_embed::simulate::qemu_download(&name).expect("plan");
    println!("archive: {}", plan.archive.display());
    for url in &plan.urls {
        println!("candidate: {url}");
    }

    rusty_embed::simulate::download(&plan.urls, &plan.archive, |line| println!("{line}"))
        .expect("download");

    println!("$ {}", plan.extract.display);
    let session = rusty_embed::process::spawn(&plan.extract, None).expect("spawn tar");
    while let Some(line) = session.recv() {
        println!("{}", line.text);
    }
    let code = session.wait();
    assert_eq!(code, Some(0), "extraction failed");
    let _ = std::fs::remove_file(&plan.archive);

    let binary = plan
        .archive
        .parent()
        .expect("tools dir")
        .join("qemu/bin")
        .join(if cfg!(windows) {
            format!("{name}.exe")
        } else {
            name.clone()
        });
    let out = std::process::Command::new(&binary)
        .arg("--version")
        .output()
        .expect("run qemu --version");
    println!("{}", String::from_utf8_lossy(&out.stdout).lines().next().unwrap_or(""));
    assert!(out.status.success());
    println!("installer proven: {}", binary.display());
}
