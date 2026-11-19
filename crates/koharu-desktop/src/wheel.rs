//! Koharu's own tools on the Windows wheel, the dial a Surface Dial or a
//! laptop's built-in one is: each an entry in the wheel's menu beside
//! Windows' own scroll, volume, and zoom, which while chosen turns and
//! presses for Koharu. The interface names them in its language and acts on
//! what they tell it, as it does on its keys.
// Fork addition: upstream has no wheel tools.

use serde::{Deserialize, Serialize};
use specta::Type;

/// A tool in the wheel's menu.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum WheelTool {
    /// Turns through the pages; a press asks for one to go to.
    Pages,
    /// Zooms the canvas; a press fits the page.
    Zoom,
    /// Sizes the brush; a press swaps the Brush and the Eraser.
    Brush,
}

/// What the chosen tool did: a turn of `steps` notches, clockwise positive,
/// or a press.
#[derive(Clone, Debug, Serialize, Type)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum WheelInput {
    Turn { tool: WheelTool, steps: i32 },
    Press { tool: WheelTool },
}

/// Each tool's name in the wheel's menu, in the interface's language.
#[derive(Clone, Debug, Deserialize, Type)]
pub struct WheelLabels {
    pub pages: String,
    pub zoom: String,
    pub brush: String,
}

#[cfg(windows)]
pub use self::radial::show;

/// Nothing to show where Windows' wheel isn't.
#[cfg(not(windows))]
pub fn show(
    _: &tauri::AppHandle<tauri_runtime_cef::CefRuntime>,
    _: &WheelLabels,
) -> anyhow::Result<()> {
    Ok(())
}

#[cfg(windows)]
mod radial {
    use std::{
        cell::RefCell,
        sync::{
            Arc,
            atomic::{AtomicU8, Ordering},
        },
    };

    use anyhow::{Context as _, Result};
    use tauri::{AppHandle, Manager as _};
    use tauri_runtime_cef::CefRuntime;
    use windows::{
        Foundation::TypedEventHandler,
        UI::Input::{
            RadialController, RadialControllerButtonClickedEventArgs, RadialControllerMenuItem,
            RadialControllerMenuKnownIcon, RadialControllerRotationChangedEventArgs,
        },
        Win32::{Foundation::HWND, UI::Input::Radial::IRadialControllerInterop},
        core::{HSTRING, IInspectable, factory},
    };

    use super::{WheelInput, WheelLabels, WheelTool};
    use crate::{events::WHEEL, screen::MAIN};

    /// The tools in the menu's order, each told by its place.
    const TOOLS: [WheelTool; 3] = [WheelTool::Pages, WheelTool::Zoom, WheelTool::Brush];
    /// A notch: the wheel clicks and tells a step every this many degrees.
    const NOTCH: f64 = 15.0;
    /// No tool of Koharu's chosen yet.
    const NONE: u8 = u8::MAX;

    /// The wheel's controller for the main window, made on the main thread,
    /// which its events arrive on, and the place of the tool chosen last.
    struct Wheel {
        controller: RadialController,
        chosen: Arc<AtomicU8>,
    }

    thread_local! {
        static CONTROLLER: RefCell<Option<Wheel>> = const { RefCell::new(None) };
    }

    /// Puts Koharu's tools in the wheel's menu, named `labels`, in place of
    /// those put there before. Runs on the main thread.
    pub fn show(handle: &AppHandle<CefRuntime>, labels: &WheelLabels) -> Result<()> {
        CONTROLLER.with_borrow_mut(|wheel| {
            let wheel = match wheel {
                Some(wheel) => wheel,
                None => wheel.insert(Wheel::new(handle)?),
            };
            wheel.menu(labels)
        })
    }

    impl Wheel {
        fn new(handle: &AppHandle<CefRuntime>) -> Result<Self> {
            let window = handle.get_window(MAIN).context("the main window is gone")?;
            let hwnd = HWND(window.hwnd()?.0 as _);
            let interop = factory::<RadialController, IRadialControllerInterop>()?;
            // SAFETY: `hwnd` is the main window's, which outlives the app's
            // use of the wheel.
            let controller: RadialController = unsafe { interop.CreateForWindow(hwnd)? };
            controller.SetRotationResolutionInDegrees(NOTCH)?;
            controller.SetUseAutomaticHapticFeedback(true)?;
            let chosen = Arc::new(AtomicU8::new(NONE));
            let tool = {
                let chosen = Arc::clone(&chosen);
                move || {
                    TOOLS
                        .get(usize::from(chosen.load(Ordering::Relaxed)))
                        .copied()
                }
            };
            controller.RotationChanged(&TypedEventHandler::<
                RadialController,
                RadialControllerRotationChangedEventArgs,
            >::new({
                let handle = handle.clone();
                let tool = tool.clone();
                move |_, args| {
                    let steps = (args.ok()?.RotationDeltaInDegrees()? / NOTCH).round() as i32;
                    if let Some(tool) = tool()
                        && steps != 0
                    {
                        WHEEL.tell(&handle, WheelInput::Turn { tool, steps });
                    }
                    Ok(())
                }
            }))?;
            controller.ButtonClicked(&TypedEventHandler::<
                RadialController,
                RadialControllerButtonClickedEventArgs,
            >::new({
                let handle = handle.clone();
                move |_, _| {
                    if let Some(tool) = tool() {
                        WHEEL.tell(&handle, WheelInput::Press { tool });
                    }
                    Ok(())
                }
            }))?;
            Ok(Self { controller, chosen })
        }

        fn menu(&self, labels: &WheelLabels) -> Result<()> {
            let items = self.controller.Menu()?.Items()?;
            items.Clear()?;
            let entries = [
                (
                    &labels.pages,
                    RadialControllerMenuKnownIcon::NextPreviousTrack,
                ),
                (&labels.zoom, RadialControllerMenuKnownIcon::Zoom),
                (&labels.brush, RadialControllerMenuKnownIcon::InkThickness),
            ];
            for (place, (label, icon)) in (0u8..).zip(entries) {
                let item = RadialControllerMenuItem::CreateFromKnownIcon(
                    &HSTRING::from(label.as_str()),
                    icon,
                )?;
                let chosen = Arc::clone(&self.chosen);
                item.Invoked(
                    &TypedEventHandler::<RadialControllerMenuItem, IInspectable>::new(
                        move |_, _| {
                            chosen.store(place, Ordering::Relaxed);
                            Ok(())
                        },
                    ),
                )?;
                items.Append(&item)?;
            }
            Ok(())
        }
    }
}
