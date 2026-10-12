fn main() {
    #[cfg(windows)]
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        embed();
    }
}

#[cfg(windows)]
fn embed() {
    println!("cargo:rerun-if-changed=build/icon.ico");
    let mut res = winresource::WindowsResource::new();
    res.set_icon("build/icon.ico");
    res.set("ProductName", "lolPing");
    res.set("FileDescription", "League of Legends style ping wheel");
    res.set("LegalCopyright", "Copyright (c) 2026 Danrui Meng, SlimeSB. MIT licensed.");
    res.compile().expect("embed the Windows resources");
}
