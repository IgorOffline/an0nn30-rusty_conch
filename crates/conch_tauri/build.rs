fn main() {
    // Rebuild Java SDK JAR if make is available (needed by conch_plugin's include_bytes!).
    let java_sdk_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../java-sdk");
    if java_sdk_dir.join("Makefile").exists() {
        println!("cargo:rerun-if-changed=../../java-sdk/src");
        let status = std::process::Command::new("make")
            .arg("-C")
            .arg(&java_sdk_dir)
            .arg("build")
            .status();
        match status {
            Ok(s) if s.success() => {}
            Ok(s) => {
                eprintln!("warning: Java SDK build exited with {s} — Java plugins may not work");
            }
            Err(e) => {
                eprintln!("warning: Could not run 'make' for Java SDK: {e} — Java plugins may not work");
            }
        }
    }

    tauri_build::build();
}
