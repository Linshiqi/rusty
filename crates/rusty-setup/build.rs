//! The installer carries rusty's icon, where Explorer and a download list
//! show it, and its window (`ui/setup.slint`) — on Windows, the one system
//! it installs on.

fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        slint_build::compile("ui/setup.slint").expect("the installer's window compiles");
        println!("cargo:rerun-if-changed=setup.rc");
        println!("cargo:rerun-if-changed=../rusty-desk/icons/rusty.ico");
        embed_resource::compile("setup.rc", embed_resource::NONE)
            .manifest_optional()
            .expect("the icon resource compiles");
    }
}
