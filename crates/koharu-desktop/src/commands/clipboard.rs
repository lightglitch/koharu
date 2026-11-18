//! Copying the interface's text, such as a pipeline error, to the system
//! clipboard the way the canvas's own copy does, rather than depending on the
//! webview granting its clipboard API to the app's page.
// Fork addition: upstream has no copy button in the interface.

use super::Result;

/// Puts `text` on the system clipboard.
#[tauri::command]
#[specta::specta]
pub fn copy_text(text: String) -> Result<()> {
    Ok(arboard::Clipboard::new().and_then(|mut clipboard| clipboard.set_text(text))?)
}
