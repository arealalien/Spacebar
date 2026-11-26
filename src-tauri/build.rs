use std::{env, fs, path::PathBuf};

fn main() {
    println!("cargo:rerun-if-changed=icons");
    println!("cargo:rerun-if-env-changed=SPACEBAR_RELEASE_NOTES_FILE");
    let bundled_notes = PathBuf::from("../release-notes").join(format!("v{}.md", env!("CARGO_PKG_VERSION")));
    println!("cargo:rerun-if-changed={}", bundled_notes.display());
    let notes = if let Some(path) = env::var_os("SPACEBAR_RELEASE_NOTES_FILE") {
        let path = PathBuf::from(path);
        println!("cargo:rerun-if-changed={}", path.display());
        fs::read_to_string(path).expect("Read the release notes as UTF-8")
    } else {
        match fs::read_to_string(bundled_notes) {
            Ok(notes) => notes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(error) => panic!("Read the release notes as UTF-8: {error}"),
        }
    };
    let output = PathBuf::from(env::var_os("OUT_DIR").expect("Cargo output directory"));
    fs::write(output.join("release-notes.md"), notes).expect("Bundle the release notes");
    tauri_build::build();
}
