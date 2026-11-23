use std::{env, fs, path::PathBuf};

fn main() {
    println!("cargo:rerun-if-changed=icons");
    println!("cargo:rerun-if-env-changed=SPACEBAR_RELEASE_NOTES_FILE");
    let notes = env::var_os("SPACEBAR_RELEASE_NOTES_FILE").map_or_else(String::new, |path| {
        let path = PathBuf::from(path);
        println!("cargo:rerun-if-changed={}", path.display());
        fs::read_to_string(path).expect("Read the release notes as UTF-8")
    });
    let output = PathBuf::from(env::var_os("OUT_DIR").expect("Cargo output directory"));
    fs::write(output.join("release-notes.md"), notes).expect("Bundle the release notes");
    tauri_build::build();
}
