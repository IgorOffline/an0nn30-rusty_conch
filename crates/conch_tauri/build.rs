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

    // Embed git commit hash and build date for the About dialog.
    if let Ok(output) = std::process::Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
    {
        let hash = String::from_utf8_lossy(&output.stdout).trim().to_string();
        println!("cargo:rustc-env=CONCH_GIT_HASH={hash}");
    }
    if let Ok(output) = std::process::Command::new("date")
        .args(["-u", "+%B %d, %Y"])
        .output()
    {
        let date = String::from_utf8_lossy(&output.stdout).trim().to_string();
        println!("cargo:rustc-env=CONCH_BUILD_DATE={date}");
    }

    tauri_build::build();
}
