//! Opening a drive's own page in Settings, under System > Storage >
//! Storage used on other drives.
//!
//! That page has no `ms-settings:` link, so Settings is opened at Storage
//! and walked there with UI Automation. Only automation IDs are matched,
//! never visible text, so this works in every display language.

use std::time::{Duration, Instant};

use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance};
use windows::Win32::System::Variant::VARIANT;
use windows::Win32::UI::Accessibility::{
    CUIAutomation, ExpandCollapseState_Collapsed, IUIAutomation, IUIAutomationElement,
    IUIAutomationExpandCollapsePattern, IUIAutomationInvokePattern, TreeScope_Children,
    TreeScope_Descendants, UIA_AutomationIdPropertyId, UIA_ButtonControlTypeId,
    UIA_ClassNamePropertyId, UIA_ControlTypePropertyId, UIA_ExpandCollapsePatternId,
    UIA_InvokePatternId,
};

use super::com::{self, Apartment};
use super::shell;

/// The "Storage used on other drives" row on the Storage page.
const VOLUME_LIST_LINK: &str = "SystemSettings_StorageSense_VolumeListLink_ButtonEntityItem";
/// Drive rows on that page are `SystemSettings_StorageSense_Volume_<label> (F:)_Button`.
const VOLUME_PREFIX: &str = "SystemSettings_StorageSense_Volume_";
/// The "Show more settings" expanders on the Storage page.
const EXPANDER: &str = "EntityItemButton";
/// Settings runs inside an app frame window.
const FRAME_CLASS: &str = "ApplicationFrameWindow";
/// How long Settings gets to open and draw each page.
const PAGE_TIMEOUT: Duration = Duration::from_secs(12);

/// Opens Settings at the drive's storage page, for example `F:`.
///
/// Blocks for a few seconds while Settings loads, so call it off the UI
/// thread. If a step fails, Settings is left open at Storage.
pub fn open_drive(volume: &str) -> Result<(), String> {
    shell::open_unelevated("ms-settings:storagesense")?;
    let _com = Apartment::enter()?;
    // SAFETY: plain COM call on an initialized apartment.
    let uia: IUIAutomation =
        unsafe { CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER) }
            .map_err(|e| com::message(&e))?;
    let suffix = format!("({})_Button", volume.to_ascii_uppercase());

    // 1. The Storage page, with "Storage used on other drives" on it. That
    //    row is inside "Advanced storage settings", which may be collapsed.
    let link = wait_for(&uia, |window| {
        if let Some(found) = find_by_id(&uia, window, VOLUME_LIST_LINK) {
            return Some(found);
        }
        expand_all(&uia, window);
        None
    })
    .ok_or("Storage page did not open in Settings")?;
    let button = first_button(&uia, &link).ok_or("Storage used on other drives not found")?;
    invoke(&button)?;

    // 2. The drive list. Pick the row whose ID ends in "(F:)_Button".
    let drive = wait_for(&uia, |window| {
        all_buttons(&uia, window).into_iter().find(|b| {
            automation_id(b)
                .is_some_and(|id| id.starts_with(VOLUME_PREFIX) && id.ends_with(&suffix))
        })
    })
    .ok_or_else(|| format!("{volume} is not in Storage used on other drives"))?;
    invoke(&drive)
}

/// Polls every Settings window with `find` until it returns something.
fn wait_for<T>(
    uia: &IUIAutomation,
    mut find: impl FnMut(&IUIAutomationElement) -> Option<T>,
) -> Option<T> {
    let start = Instant::now();
    while start.elapsed() < PAGE_TIMEOUT {
        for window in frame_windows(uia) {
            if let Some(found) = find(&window) {
                return Some(found);
            }
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    None
}

fn frame_windows(uia: &IUIAutomation) -> Vec<IUIAutomationElement> {
    // SAFETY: plain COM calls; every result is checked.
    unsafe {
        let Ok(root) = uia.GetRootElement() else {
            return Vec::new();
        };
        let Ok(cond) =
            uia.CreatePropertyCondition(UIA_ClassNamePropertyId, &VARIANT::from(FRAME_CLASS))
        else {
            return Vec::new();
        };
        collect(root.FindAll(TreeScope_Children, &cond).ok())
    }
}

fn find_by_id(
    uia: &IUIAutomation,
    within: &IUIAutomationElement,
    id: &str,
) -> Option<IUIAutomationElement> {
    // SAFETY: plain COM calls; every result is checked.
    unsafe {
        let cond = uia
            .CreatePropertyCondition(UIA_AutomationIdPropertyId, &VARIANT::from(id))
            .ok()?;
        within.FindFirst(TreeScope_Descendants, &cond).ok()
    }
}

fn all_buttons(uia: &IUIAutomation, within: &IUIAutomationElement) -> Vec<IUIAutomationElement> {
    // SAFETY: plain COM calls; every result is checked.
    unsafe {
        let Ok(cond) = uia.CreatePropertyCondition(
            UIA_ControlTypePropertyId,
            &VARIANT::from(UIA_ButtonControlTypeId.0),
        ) else {
            return Vec::new();
        };
        collect(within.FindAll(TreeScope_Descendants, &cond).ok())
    }
}

fn first_button(
    uia: &IUIAutomation,
    within: &IUIAutomationElement,
) -> Option<IUIAutomationElement> {
    all_buttons(uia, within).into_iter().next()
}

/// Opens every collapsed "Show more settings" expander on the page.
fn expand_all(uia: &IUIAutomation, window: &IUIAutomationElement) {
    for button in all_buttons(uia, window) {
        if automation_id(&button).as_deref() != Some(EXPANDER) {
            continue;
        }
        // SAFETY: plain COM calls; a button without the pattern is skipped.
        unsafe {
            if let Ok(pattern) = button.GetCurrentPatternAs::<IUIAutomationExpandCollapsePattern>(
                UIA_ExpandCollapsePatternId,
            ) && pattern.CurrentExpandCollapseState() == Ok(ExpandCollapseState_Collapsed)
            {
                let _ = pattern.Expand();
            }
        }
    }
}

fn invoke(element: &IUIAutomationElement) -> Result<(), String> {
    // SAFETY: plain COM calls.
    unsafe {
        element
            .GetCurrentPatternAs::<IUIAutomationInvokePattern>(UIA_InvokePatternId)
            .and_then(|p| p.Invoke())
            .map_err(|e| com::message(&e))
    }
}

fn automation_id(element: &IUIAutomationElement) -> Option<String> {
    // SAFETY: plain COM call.
    unsafe { element.CurrentAutomationId() }
        .ok()
        .map(|id| id.to_string())
}

fn collect(
    array: Option<windows::Win32::UI::Accessibility::IUIAutomationElementArray>,
) -> Vec<IUIAutomationElement> {
    let Some(array) = array else {
        return Vec::new();
    };
    // SAFETY: indexes stay below Length.
    unsafe {
        let len = array.Length().unwrap_or(0);
        (0..len).filter_map(|i| array.GetElement(i).ok()).collect()
    }
}
