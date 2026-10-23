fn main() {
    println!("cargo:rerun-if-changed=folder.rc");
    println!("cargo:rerun-if-changed=folder.exe.manifest");
    println!("cargo:rerun-if-changed=folder.ico");
    // Version de l'exécutable (propriétés du fichier) : celle de Cargo.toml, complétée dans
    // une copie du modèle folder.rc.
    let version = std::env::var("CARGO_PKG_VERSION").expect("CARGO_PKG_VERSION");
    let numbers: Vec<&str> = version
        .split(['.', '-', '+'])
        .take(3)
        .chain(["0"; 4])
        .take(4)
        .map(|n| {
            if n.chars().all(|c| c.is_ascii_digit()) {
                n
            } else {
                "0"
            }
        })
        .collect();
    let dir = std::env::var("CARGO_MANIFEST_DIR")
        .expect("CARGO_MANIFEST_DIR")
        .replace('\\', "/");
    let rc = std::fs::read_to_string("folder.rc")
        .expect("folder.rc")
        .replace("@VERSION_COMMA@", &numbers.join(","))
        .replace("@VERSION@", &version)
        .replace("@DIR@", &dir);
    let out = std::path::Path::new(&std::env::var("OUT_DIR").expect("OUT_DIR")).join("folder.rc");
    std::fs::write(&out, rc).expect("écriture de folder.rc");
    // Manifeste : DPI par moniteur, chemins longs, UTF-8, Segment Heap, Common Controls v6.
    embed_resource::compile(&out, embed_resource::NONE)
        .manifest_required()
        .expect("compilation des ressources Windows (rc.exe du Windows SDK requis)");
}
