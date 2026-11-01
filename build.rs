//! Embeds the Windows resources (UAC manifest, icon, version info) into
//! RustCleanup.exe at link time.

fn main() {
    println!("cargo:rerun-if-changed=app.manifest");
    println!("cargo:rerun-if-changed=resources.rc");
    println!("cargo:rerun-if-changed=assets/icon.ico");

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        // `manifest_required` fails the build if rc.exe is missing, rather than
        // silently producing an exe that would run without admin rights.
        embed_resource::compile("resources.rc", embed_resource::NONE)
            .manifest_required()
            .expect("failed to embed Windows resources (is the Windows SDK installed?)");
    }
}
