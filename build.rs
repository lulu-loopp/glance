fn main() {
    // The program's icon, and the manifest that declares it aware of each
    // monitor's scale: Windows resources, for a Windows build.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        embed_resource::compile("glance.rc", embed_resource::NONE).manifest_required().unwrap();
    }
}
