//! Validation's fixes: a model shown the page as it's drawn, with its texts
//! and what measuring flagged, answers a fix for each flagged text or
//! accepts it, from a closed set held to a schema; each fix is checked
//! against the page before it's written.
// Fork addition: upstream has no validation stage.

use std::{collections::BTreeMap, future::IntoFuture};

use anyhow::{Context as _, Result};
use koharu_llm::{
    Size,
    rig::{
        completion::{
            GenerationOptions, Message, Reasoning,
            message::{DocumentSourceKind, Image, ImageMediaType, UserContent},
        },
        driver::DynModel,
        operation::Completion,
    },
};
use koharu_scene::{LayerId, Op, Role};
use kurbo::{Affine, Rect};
use serde::Deserialize;
use serde_json::{Map, Value, json};
use tokio_util::sync::CancellationToken;

use super::{Flag, FlagKind, ValidationConfig};
use crate::stages::translate::turn::remembered;

/// The model `reference` names, which reads the page.
pub(super) async fn connect(reference: &str) -> Result<DynModel<Completion>> {
    koharu_llm::connect(
        reference,
        Size {
            sequences: 1,
            context: 32_768,
            images: true,
        },
    )
    .await
}

/// How a text's frame holds it, which a size fix keeps.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Holder {
    Box,
    Path,
    Point,
}

/// A translated text as the model sees it.
#[derive(Clone, Debug)]
pub(super) struct Text {
    pub(super) key: String,
    pub(super) block: LayerId,
    pub(super) role: Role,
    pub(super) text: String,
    /// The size its letters are drawn at, in page pixels.
    pub(super) size: f32,
    /// Where its letters are, on the page.
    pub(super) letters: Rect,
    /// The frame it may fill, on the page; none for point text.
    pub(super) room: Option<Rect>,
    /// Where its balloon safely holds text, when it's in one.
    pub(super) balloon: Option<Rect>,
    /// Its fill's relative luminance, which an outline is unlike.
    pub(super) fill: f64,
    pub(super) holder: Holder,
    /// Its own coordinates to the page's.
    pub(super) transform: Affine,
}

/// A page as the model sees it.
#[derive(Clone, Debug)]
pub(super) struct Page {
    /// The page as it's drawn, as a JPEG in base 64.
    pub(super) image: String,
    /// The image's pixels to the page's.
    pub(super) scale: f64,
    pub(super) width: f64,
    pub(super) height: f64,
    pub(super) typical: Option<f32>,
    pub(super) texts: Vec<Text>,
}

/// A fix the model answered.
#[derive(Clone, Debug, Deserialize)]
pub(super) struct Fix {
    /// The text's key; a model that slips names it by its words.
    #[serde(alias = "text")]
    key: String,
    action: Action,
    #[serde(default)]
    size: Option<f64>,
    #[serde(default, rename = "box")]
    area: Option<[f64; 4]>,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum Action {
    Accept,
    Size,
    Box,
    Outline,
}

#[derive(Deserialize)]
struct Answer {
    fixes: Vec<Fix>,
}

const SYSTEM: &str = "You check the lettering of a translated manga page and fix it. \
The image is the page as it's drawn now, translations included. \
You get the page's translated texts as JSON: each with a key, its role (dialogue or a sound effect), its text, \
the size its letters are drawn at, the area its letters cover (\"letters\": [x, y, width, height]), \
the area it may fill (\"room\"), and what measuring flagged, all in page pixels. \
For each flagged text, answer one fix in \"fixes\", naming the text by its \"key\", such as \"t1\":\n\
- \"accept\" when it's right as it is: a shout, a title, or a sound effect may be big on purpose, and a whisper small.\n\
- \"size\" with \"size\": the size in page pixels its letters should be drawn at.\n\
- \"box\" with \"box\": [x, y, width, height], the area in page pixels it should fill: for a text in a \
balloon, inside its \"balloon\", the area its balloon safely holds text in, and off the other texts; anywhere \
on the art for a sound effect without one.\n\
- \"outline\", for text hard to tell from the art behind it: it gets an outline unlike its letters.\n\
A text shrinks to fit its room, so a bigger size only shows where its room holds it: \
for small text in a tight spot, answer a bigger \"box\" instead. \
After your fixes the page is drawn and measured again; when what's still flagged comes back, \
try another fix rather than the same one, or accept it. \
Fix only flagged texts. Keep dialogue near the page's typical size. \
Don't cover faces or other texts. Set the fields an action doesn't use to null.";

/// What the model answers for `page`'s `flags`, after the page's earlier
/// rounds in `history`, which this round joins, and `note`, what it hears of
/// its last round first; `None` when the run stopped first.
pub(super) async fn ask(
    model: &DynModel<Completion>,
    page: &Page,
    flags: &[Flag],
    note: Option<&str>,
    history: &mut Vec<Message>,
    reasoning: Option<Reasoning>,
    cancel: &CancellationToken,
) -> Result<Option<Vec<Fix>>> {
    let texts: Vec<Value> = page
        .texts
        .iter()
        .map(|text| {
            let flagged: Vec<FlagKind> = flags
                .iter()
                .filter(|flag| flag.block == text.block)
                .map(|flag| flag.kind)
                .collect();
            json!({
                "key": text.key,
                "role": match text.role { Role::Text => "dialogue", Role::Sfx => "sound effect" },
                "text": text.text,
                "size": text.size.round(),
                "letters": rounded(text.letters),
                "room": text.room.map(rounded),
                "balloon": text.balloon.map(rounded),
                "flagged": flagged,
            })
        })
        .collect();
    let brief = json!({
        "page": {"width": page.width.round(), "height": page.height.round()},
        "image_scale": (1.0 / page.scale * 1000.0).round() / 1000.0,
        "typical_dialogue_size": page.typical.map(f32::round),
        "texts": texts,
    });
    let mut content = Vec::new();
    if model.spec().is_some_and(|spec| spec.input.image) {
        content.push(UserContent::Image(Image {
            data: DocumentSourceKind::Base64(page.image.clone()),
            media_type: Some(ImageMediaType::JPEG),
            detail: None,
            native: None,
        }));
    }
    if let Some(note) = note {
        content.push(UserContent::text(note.to_owned()));
    }
    content.push(UserContent::text(brief.to_string()));
    let mut options = GenerationOptions::default();
    if let Some(reasoning) = reasoning {
        options = options.reasoning(reasoning);
    }
    let keys: Vec<&str> = page.texts.iter().map(|text| text.key.as_str()).collect();
    let request = koharu_llm::answers(model, schema(&keys)?)
        .preamble(SYSTEM)
        .options(options)
        .build()
        .prompt(Message::from(content))
        .history(history.clone());
    let response = tokio::select! {
        response = request.into_future() => response?,
        () = cancel.cancelled() => return Ok(None),
    };
    let fixes = read(&response.output())?;
    // The image served this round alone, as a translation's page does.
    history.extend(remembered(response.messages));
    Ok(Some(fixes))
}

/// The fixes in `answer`, which may come fenced or wrapped in prose.
fn read(answer: &str) -> Result<Vec<Fix>> {
    let json = match (answer.find('{'), answer.rfind('}')) {
        (Some(start), Some(end)) if start < end => &answer[start..=end],
        _ => answer,
    };
    let answer: Answer = serde_json::from_str(json).with_context(|| {
        format!(
            "the validation model's answer isn't its fixes: {}",
            answer.replace('\n', " ")
        )
    })?;
    Ok(answer.fixes)
}

/// The answer's schema, naming a text by one of `keys`. Every property is
/// required, the unused ones null, so rig's rewrite toward strict mode
/// leaves it as it is.
fn schema(keys: &[&str]) -> Result<schemars::Schema> {
    schemars::Schema::try_from(json!({
        "type": "object",
        "properties": {
            "fixes": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {
                        "key": {"type": "string", "enum": keys},
                        "action": {"type": "string", "enum": ["accept", "size", "box", "outline"]},
                        "size": {"type": ["number", "null"]},
                        "box": {"type": ["array", "null"], "items": {"type": "number"}},
                    },
                    "required": ["key", "action", "size", "box"],
                    "additionalProperties": false,
                },
            },
        },
        "required": ["fixes"],
        "additionalProperties": false,
    }))
    .context("the validation answer's schema isn't valid")
}

fn rounded(rect: Rect) -> [f64; 4] {
    [rect.x0, rect.y0, rect.width(), rect.height()].map(f64::round)
}

/// A round's ops, each with the text it changes, and the flags it accepts.
type Fixed = (Vec<(LayerId, Op)>, Vec<(LayerId, FlagKind)>);

/// The ops `fixes` make of `page`, each checked against it, one per text
/// with the text it changes, and the flags they accept as they are.
pub(super) fn ops(page: &Page, fixes: &[Fix], flags: &[Flag], config: &ValidationConfig) -> Fixed {
    let mut patches: BTreeMap<LayerId, Map<String, Value>> = BTreeMap::new();
    let mut accepted = Vec::new();
    let bounds = Rect::new(0.0, 0.0, page.width, page.height);
    let smallest = page.height * config.min_size;
    for fix in fixes {
        let named = |text: &&Text| text.key == fix.key || text.text == fix.key;
        let Some(text) = page.texts.iter().find(named) else {
            tracing::info!(key = %fix.key, "a validation fix names no text of the page");
            continue;
        };
        // Its own units to page pixels.
        let scale = text.transform.determinant().abs().sqrt().max(f64::EPSILON);
        let patch = patches.entry(text.block).or_default();
        match fix.action {
            Action::Accept => accepted.extend(
                flags
                    .iter()
                    .filter(|flag| flag.block == text.block)
                    .map(|flag| (flag.block, flag.kind)),
            ),
            Action::Size => {
                let Some(size) = fix.size.filter(|size| size.is_finite()) else {
                    continue;
                };
                // Up to a page's headline for a sound effect, and a big shout
                // for dialogue.
                let largest = match (text.role, page.typical) {
                    (Role::Text, Some(typical)) => f64::from(typical) * config.big.max(1.0) * 1.5,
                    (Role::Text, None) => page.height * 0.06,
                    (Role::Sfx, _) => page.height * 0.2,
                };
                let size = size.clamp(smallest, largest.max(smallest));
                merge(patch, json!({"style": {"chars": {"size": size / scale}}}));
                // The size set is the size drawn, shrunk only to fit.
                match text.holder {
                    Holder::Box => merge(patch, json!({"frame": {"box": {"fit": "shrink"}}})),
                    Holder::Path => merge(patch, json!({"frame": {"path": {"fit": "shrink"}}})),
                    Holder::Point => {}
                }
            }
            Action::Box => {
                let Some([x, y, width, height]) = fix.area else {
                    continue;
                };
                if ![x, y, width, height].iter().all(|value| value.is_finite())
                    || width < 8.0
                    || height < 8.0
                {
                    continue;
                }
                let mut area = Rect::new(x, y, x + width, y + height).intersect(bounds);
                // A text in a balloon, dialogue or a sound effect, stays
                // where its balloon holds it, whatever the model asked.
                if let Some(balloon) = text.balloon {
                    area = area.intersect(balloon);
                }
                if area.width() < 8.0 || area.height() < 8.0 {
                    continue;
                }
                let own = text.transform.inverse().transform_rect_bbox(area);
                merge(
                    patch,
                    json!({"frame": {
                        "box": {
                            "bounds": {"x": own.x0, "y": own.y0, "width": own.width(), "height": own.height()},
                            "fit": "shrink",
                            "anchor": "center",
                        },
                        "path": null,
                    }}),
                );
            }
            Action::Outline => {
                // Unlike its letters, as a sound effect's outline is placed:
                // black round light letters, white round any other. The
                // model only says that it needs one; one it coloured black
                // round black letters only made them bold.
                let color = if text.fill > 0.5 {
                    [0.0, 0.0, 0.0, 1.0]
                } else {
                    [1.0, 1.0, 1.0, 1.0]
                };
                // An eighth of its size: over art, a sound effect's
                // sixteenth leaves the letters touching the lines.
                let width = f64::from(text.size) / scale / 8.0;
                merge(
                    patch,
                    json!({"style": {"chars": {"stroke": color, "stroke_width": width}}}),
                );
            }
        }
    }
    let ops = patches
        .into_iter()
        .filter(|(_, patch)| !patch.is_empty())
        .map(|(block, patch)| {
            let op = Op::UpdateLayer {
                layer: block.into(),
                patch: json!({"translation": Value::Object(patch)}),
            };
            (block, op)
        })
        .collect();
    (ops, accepted)
}

/// `from` merged into `into`, objects key by key.
fn merge(into: &mut Map<String, Value>, from: Value) {
    let Value::Object(from) = from else {
        return;
    };
    for (key, value) in from {
        match (into.get_mut(&key), value) {
            (Some(Value::Object(existing)), Value::Object(value)) => {
                merge(existing, Value::Object(value));
            }
            (_, value) => {
                into.insert(key, value);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(n: u64) -> LayerId {
        format!("00000000-0000-0000-0000-{n:012}")
            .parse()
            .expect("an id")
    }

    fn page() -> Page {
        let text = |n: u64, role| Text {
            key: format!("t{n}"),
            block: id(n),
            role,
            text: if n == 2 { "Sfx!" } else { "Hey" }.to_owned(),
            size: 30.0,
            letters: Rect::new(100.0, 100.0, 200.0, 130.0),
            room: Some(Rect::new(90.0, 90.0, 210.0, 200.0)),
            balloon: None,
            fill: 0.0,
            holder: Holder::Path,
            transform: Affine::IDENTITY,
        };
        Page {
            image: String::new(),
            scale: 1.0,
            width: 1000.0,
            height: 1500.0,
            typical: Some(20.0),
            texts: vec![text(1, Role::Text), text(2, Role::Sfx)],
        }
    }

    fn flag(n: u64, kind: FlagKind) -> Flag {
        Flag {
            block: id(n),
            kind,
            text: String::new(),
            size: 30.0,
            typical: Some(20.0),
            other: None,
            contrast: None,
        }
    }

    #[test]
    fn an_answer_reads_fenced_and_its_fixes_become_checked_patches() -> Result<()> {
        let answer = "```json\n{\"fixes\": [\
            {\"key\": \"t1\", \"action\": \"size\", \"size\": 500, \"box\": null},\
            {\"key\": \"t2\", \"action\": \"box\", \"size\": null, \"box\": [900, 10, 300, 50]},\
            {\"text\": \"Sfx!\", \"action\": \"outline\", \"size\": null, \"box\": null, \"outline\": \"black\"},\
            {\"key\": \"t9\", \"action\": \"size\", \"size\": 20, \"box\": null}\
        ]}\n```";
        let fixes = read(answer)?;
        let (ops, accepted) = ops(&page(), &fixes, &[], &ValidationConfig::default());
        assert!(accepted.is_empty());
        let patches: Vec<Value> = ops
            .iter()
            .map(|(_, op)| match op {
                Op::UpdateLayer { patch, .. } => patch.clone(),
                _ => panic!("an update"),
            })
            .collect();
        // A huge size is held to a shout's, 20 × 1.6 × 1.5.
        assert_eq!(
            patches[0]["translation"]["style"]["chars"]["size"],
            json!(48.0)
        );
        assert_eq!(
            patches[0]["translation"]["frame"]["path"]["fit"],
            json!("shrink")
        );
        // A box past the page is cut to it, and both fixes of one text are one
        // patch.
        let bounds = &patches[1]["translation"]["frame"]["box"]["bounds"];
        assert_eq!(
            (bounds["x"].clone(), bounds["width"].clone()),
            (json!(900.0), json!(100.0))
        );
        assert_eq!(patches[1]["translation"]["frame"]["path"], Value::Null);
        assert_eq!(
            patches[1]["translation"]["style"]["chars"]["stroke"],
            json!([1.0, 1.0, 1.0, 1.0])
        );
        assert_eq!(ops.len(), 2);
        Ok(())
    }

    #[test]
    fn accepting_a_text_accepts_its_flags() {
        let fixes = vec![Fix {
            key: "t1".to_owned(),
            action: Action::Accept,
            size: None,
            area: None,
        }];
        let flags = [flag(1, FlagKind::TooBig), flag(2, FlagKind::Overflow)];
        let (ops, accepted) = ops(&page(), &fixes, &flags, &ValidationConfig::default());
        assert!(ops.is_empty());
        assert_eq!(accepted, [(id(1), FlagKind::TooBig)]);
    }
}
