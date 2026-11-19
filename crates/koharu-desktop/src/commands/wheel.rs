//! The interface's say in the Windows wheel's menu.
// Fork addition: upstream has no wheel tools.

use tauri::{AppHandle, Manager as _};
use tauri_runtime_cef::CefRuntime;

use super::Result;
use crate::wheel::{self, WheelLabels};

/// Puts Koharu's tools in the Windows wheel's menu, named `labels` in the
/// interface's language; where there's no wheel, the menu is never shown.
#[tauri::command]
#[specta::specta]
pub fn show_wheel_tools(handle: AppHandle<CefRuntime>, labels: WheelLabels) -> Result<()> {
    let main = handle.clone();
    // The wheel's controller is made and told on the main window's thread.
    handle.run_on_main_thread(move || {
        if let Err(error) = wheel::show(&main, &labels) {
            tracing::warn!("the wheel's tools aren't in its menu: {error:#}");
        }
    })?;
    Ok(())
}
