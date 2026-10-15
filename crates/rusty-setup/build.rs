//! The installer carries rusty's icon, where Explorer and a download list
//! show it.

fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        println!("cargo:rerun-if-changed=setup.rc");
        println!("cargo:rerun-if-changed=../rusty-desk/icons/rusty.ico");
        embed_resource::compile("setup.rc", embed_resource::NONE)
            .manifest_optional()
            .expect("the icon resource compiles");
    }
}
