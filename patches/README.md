# Patched crates

## eframe 0.37.0

A copy of eframe 0.37.0 from crates.io with one change, in
`src/native/glow_integration.rs`, `GlutinWindowContext::remove_viewports_not_in`.

When a detached window (Task list, Settings, Scan Disk, ...) was closed,
eframe dropped its window and device context while the shared OpenGL context
was still current on it. The next context switch then called `wglMakeCurrent`
with the freed DC, and Intel's driver (`igxelpicd64.dll`) crashed with an
access violation (0xC0000005). The patch makes the context not current before
the window is dropped.

Search for "RustCleanup patch" to find it. Remove this folder and the
`[patch.crates-io]` entry in `Cargo.toml` once eframe ships the fix.
