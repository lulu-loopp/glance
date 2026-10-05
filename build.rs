fn main() {
    // The program's icon, and the manifest that declares it aware of each
    // monitor's scale.
    embed_resource::compile("glance.rc", embed_resource::NONE).manifest_required().unwrap();
}
