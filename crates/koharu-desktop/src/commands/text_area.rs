//! Text that detection missed, marked by hand.
// Fork addition: upstream's marked area only feeds Generative Fill.

use std::sync::Arc;

use anyhow::Context as _;
use koharu_scene::{Created, Edit, LayerId, Op};
use serde_json::json;
use tauri::State;

use super::Result;
use crate::app::App;

/// Makes the area the marquee and the lasso marked a new text block, as if
/// detection had found text there: its source is the area's box, its mask
/// the area, drawn above the page's other layers. `label` is the undo step's
/// name, in the interface's language. Answers the block, which the interface
/// then runs the pipeline's later stages on, or `None` while no area is
/// marked. The area is dropped, as Photoshop drops a selection a command
/// used.
#[tauri::command]
#[specta::specta]
pub async fn add_text_from_area(
    app: State<'_, Arc<App>>,
    label: String,
) -> Result<Option<LayerId>> {
    let Some((page, rect, pixels)) = app.screen.area().await.ok().flatten() else {
        return Ok(None);
    };
    let editing = Arc::clone(&app);
    let block = tokio::task::spawn_blocking(move || -> anyhow::Result<LayerId> {
        let above = editing.session()?.lock().project().page(page)?.layers.len();
        let source = json!({
            "frame": {"box": {"bounds": {
                "x": rect.x,
                "y": rect.y,
                "width": rect.width,
                "height": rect.height,
            }}},
            // Taller than wide reads as Japanese's vertical lines, as
            // detection guesses without typography.
            "direction": if rect.height > rect.width { "vertical" } else { "horizontal" },
        });
        let ops: Vec<Op> = vec![
            serde_json::from_value(json!({
                "op": "add_layer",
                "id": "$area",
                "page": page,
                "index": above,
                "layer": {"kind": "block", "role": "text", "source": source},
            }))?,
            Op::WritePixels {
                layer: "$area.mask"
                    .to_owned()
                    .try_into()
                    .map_err(anyhow::Error::msg)?,
                rect,
                pixels,
            },
        ];
        let change = editing.edit(Edit { label, ops })?;
        change
            .created
            .into_iter()
            .find_map(|created| match created {
                Created::Layer { id, .. } => Some(id),
                Created::Page { .. } => None,
            })
            .context("the edit made no block")
    })
    .await??;
    app.screen.deselect();
    Ok(Some(block))
}
