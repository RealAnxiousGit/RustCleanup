//! Tests that open real windows on the desktop: every System Tool and the
//! Settings storage pages. They are ignored by default; run them by hand
//! (elevated, to test the real case) and close what they open:
//!
//! `cargo test desktop_ -- --ignored --test-threads=1`

use super::{disk, storage_page, tools};

#[test]
#[ignore = "opens windows on the desktop"]
fn desktop_every_tool_launches() {
    for tool in &tools::TOOLS {
        println!("  {}", tool.label);
        tool.launch().unwrap();
        std::thread::sleep(std::time::Duration::from_secs(2));
    }
}

#[test]
#[ignore = "opens Windows Settings"]
fn desktop_storage_page_for_each_other_drive() {
    let system = std::env::var("SystemDrive").unwrap();
    for drive in disk::list_drives() {
        let volume = drive.volume();
        if volume.eq_ignore_ascii_case(&system) {
            continue;
        }
        println!("  {volume}");
        storage_page::open_drive(volume).unwrap();
        std::thread::sleep(std::time::Duration::from_secs(2));
    }
}

#[test]
#[ignore = "opens a Command Prompt"]
fn desktop_standard_prompt_opens() {
    let tool = tools::TOOLS
        .iter()
        .find(|t| t.label == "Command Prompt")
        .unwrap();
    tool.launch().unwrap();
}
