mod command_names {
    include!("src/command_names.rs");
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    // A direct Cargo build does not run the Tauri CLI's beforeBuildCommand.
    // Read the manifest folder when the script runs: Cargo can reuse a compiled
    // build script from another checkout that shares the target folder.
    let manifest =
        std::env::var_os("CARGO_MANIFEST_DIR").ok_or("Cargo CARGO_MANIFEST_DIR missing")?;
    let root = std::path::Path::new(&manifest).join("../..");
    let build = std::process::Command::new("node")
        .arg(root.join("scripts/build-frontend.mjs"))
        .current_dir(&root)
        .status()?;
    if !build.success() {
        return Err("Frontend build failed; run npm run build:frontend".into());
    }
    // Generate menus from the renderer's single key table rather than keeping a
    // second shortcut table in Rust.
    let generate_keymap = r#"
        import {KEYMAP, acceleratorFor} from './desktop/keymap.js';
        const schemes = ['mac', 'windows'].map(scheme => {
          const entries = KEYMAP.map(row => [row.id, acceleratorFor(row.id, scheme) ?? null]);
          return [scheme, Object.fromEntries(entries)];
        });
        console.log(JSON.stringify(Object.fromEntries(schemes)));
    "#;
    let output = std::process::Command::new("node")
        .args(["--input-type=module", "-e", generate_keymap])
        .current_dir(&root)
        .output()?;
    if !output.status.success() {
        return Err("Cannot read renderer keymap".into());
    }
    let destination =
        std::path::PathBuf::from(std::env::var_os("OUT_DIR").ok_or("Cargo OUT_DIR missing")?);
    std::fs::write(destination.join("keymap.json"), output.stdout)?;
    println!("cargo:rerun-if-changed=../../desktop");
    println!("cargo:rerun-if-changed=../../scripts/build-frontend.mjs");
    println!("cargo:rerun-if-changed=../../node_modules/@rhwp/core");
    tauri_build::try_build(
        tauri_build::Attributes::new()
            .app_manifest(tauri_build::AppManifest::new().commands(command_names::COMMAND_NAMES)),
    )?;
    Ok(())
}
