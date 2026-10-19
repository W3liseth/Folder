fn main() {
    println!("cargo:rerun-if-changed=folder.rc");
    println!("cargo:rerun-if-changed=folder.exe.manifest");
    println!("cargo:rerun-if-changed=folder.ico");
    // Manifeste : DPI par moniteur, chemins longs, UTF-8, Segment Heap, Common Controls v6.
    embed_resource::compile("folder.rc", embed_resource::NONE)
        .manifest_required()
        .expect("compilation des ressources Windows (rc.exe du Windows SDK requis)");
}
