//! Validation: each translated text of a page measured as it's drawn, and
//! flagged where a reader would trip on it: too small or too big beside the
//! page's other dialogue, past its box, over another text, off the page or
//! out of its balloon, or hard to tell from what's behind it. A model shown
//! the page then fixes or accepts each flagged text, and the page is measured
//! again, up to `max_iterations` times; what's still flagged goes out as an
//! event, for the reader.
// Fork addition: upstream has no validation stage.

mod fix;

use std::collections::HashSet;

use base64::Engine as _;
use image::{DynamicImage, ImageFormat, Rgba, RgbaImage, imageops};
use koharu_renderer::{Frame as RendererFrame, Raster, Renderer};
use koharu_scene::{Content, Frame, Layer, LayerId, Op, Project, Role, Text};
use kurbo::{Affine, BezPath, Point, Rect, Shape as _};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tracing::info_span;

use self::fix::Holder;
use super::{Member, Task};
use crate::{Event, PipelineConfig, Stage};

/// The longest side of the page the model sees, in pixels, as the
/// translator's.
const SIGHT: u32 = 1280;

/// `[pipeline.validation]`: when validation flags a text. Sizes are the
/// size a text is drawn at, after fitting, in page pixels.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
#[derive(specta::Type)]
pub struct ValidationConfig {
    /// The `vendor/model` that fixes what's flagged; the translator's when
    /// empty. It must read images and be hosted, as an OpenAI-compatible
    /// server is.
    pub model: String,
    /// How many times a page is fixed and measured again; 0 only measures.
    pub max_iterations: u32,
    /// The smallest dialogue may be drawn, as a share of the page's height.
    pub min_size: f64,
    /// Dialogue drawn below this share of the page's typical dialogue size
    /// is too small.
    pub small: f64,
    /// Dialogue drawn above this share of the page's typical dialogue size
    /// is too big. A sound effect is never too small or too big; only one
    /// over dialogue is flagged.
    pub big: f64,
    /// Two texts overlap when this share of the smaller one's lines lies
    /// under the other's.
    pub overlap: f64,
    /// A text without an outline needs one below this contrast ratio, as
    /// WCAG measures it, against what's behind its letters.
    pub contrast: f64,
    /// A text without an outline needs one when more than this share of
    /// the page under and right around its letters, its translations left
    /// out, is nearly as dark as the text, or as light: art through or
    /// against the letters. A clean balloon measures 0.
    pub busy: f64,
}

impl Default for ValidationConfig {
    fn default() -> Self {
        Self {
            model: String::new(),
            max_iterations: 3,
            min_size: 0.007,
            small: 0.6,
            big: 1.6,
            overlap: 0.05,
            contrast: 3.0,
            busy: 0.01,
        }
    }
}

/// What's wrong with a text.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum FlagKind {
    TooSmall,
    TooBig,
    /// It doesn't fit its box, even shrunk.
    Overflow,
    /// A sound effect lies over this dialogue.
    SfxOverlap,
    /// Another dialogue lies over this one.
    Overlap,
    OffPage,
    OutsideBalloon,
    LowContrast,
    BusyBackground,
    /// Its translation repeats a word over and over, or runs many times its
    /// source's length: the model looped. Only retranslating fixes it.
    Runaway,
}

/// A text validation flagged, with what it measured.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, specta::Type)]
pub struct Flag {
    pub block: LayerId,
    pub kind: FlagKind,
    /// The start of its translation, to find it by.
    pub text: String,
    /// The size it's drawn at, in page pixels.
    pub size: f32,
    /// The page's typical dialogue size, the median of what it's drawn at,
    /// on a page with enough dialogue to have one.
    pub typical: Option<f32>,
    /// The text it overlaps.
    pub other: Option<LayerId>,
    /// Its contrast ratio against what's behind it.
    pub contrast: Option<f32>,
}

/// What's behind a text's letters, as the page is drawn.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Behind {
    /// The mean relative luminance, as WCAG defines it.
    luminance: f64,
    /// The share of it nearly as dark as the text, or as light: the
    /// lines of artwork, under text that has no outline to stand apart.
    similar: f64,
}

/// A translated text as it's drawn.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Measured {
    block: LayerId,
    role: Role,
    text: String,
    size: f32,
    /// Each line's letters, as a quadrilateral on the page.
    quads: Vec<[Point; 4]>,
    overflowed: bool,
    outlined: bool,
    /// Its fill's relative luminance.
    fill: f64,
    /// Its balloon's outline, when it's in one.
    balloon: Option<BezPath>,
    behind: Option<Behind>,
    /// Its whole text, for the model.
    full: String,
    /// Its source's whole text, which a runaway translation outgrows.
    source: String,
    /// The frame it may fill, on the page.
    room: Option<Rect>,
    holder: Holder,
    /// Its own coordinates to the page's.
    transform: Affine,
}

/// Validates the task's page: measures it, and while something's flagged
/// and rounds are left, has the model fix or accept it and measures again.
pub(crate) async fn validate(task: &Task, config: &PipelineConfig) -> anyhow::Result<()> {
    let member = task.members[0].clone();
    task.started(&[&member]);
    let settings = config.validation.clone();
    let reference = match settings.model.trim() {
        "" => config.translator.model.clone(),
        model => model.to_owned(),
    };
    // A model of Koharu's own runs on the GPU thread, which validation
    // doesn't take a turn on.
    let local = reference
        .split_once('/')
        .is_none_or(|(vendor, _)| vendor == koharu_llm::KOHARU);
    if local && settings.max_iterations > 0 {
        tracing::info!(%reference, "validation only measures: its fixes need a hosted model");
    }
    let mut accepted: HashSet<(LayerId, FlagKind)> = HashSet::new();
    let mut model = None;
    // The page's rounds so far, so a fix that didn't help isn't tried again.
    let mut history = Vec::new();
    // The round just written, which the next measure checks.
    let mut last: Option<Round> = None;
    // What the model hears first next round: the fixes undone.
    let mut note: Option<String> = None;
    let mut round = 0;
    let mut remaining = loop {
        let fixing = !local && round < settings.max_iterations;
        let (flags, page) = look(&member, &settings, &accepted, fixing).await?;
        tracing::info!(
            page = member.index + 1,
            round,
            flags = ?flags.iter().map(|flag| (flag.kind, &flag.text)).collect::<Vec<_>>(),
            "validation measured"
        );
        // A fix is kept only if it made nothing worse: one that brought a
        // problem its text, or a text beside it, didn't have is undone.
        if let Some(written) = last.take() {
            let worse = written.worse(&flags);
            if !worse.is_empty() {
                note = Some(written.told(&worse));
                let undo: Vec<Op> = written
                    .undo
                    .into_iter()
                    .filter(|(block, _)| worse.iter().any(|(culprit, ..)| culprit == block))
                    .map(|(_, op)| op)
                    .collect();
                tracing::info!(
                    page = member.index + 1,
                    round,
                    undone = undo.len(),
                    "validation undid fixes"
                );
                member.apply(Stage::Validate, move |_| Ok(undo)).await?;
                continue;
            }
            // A round that changed no flag won't be helped by another: a
            // balloon too small for its text stays so whatever's asked.
            if written.same(&flags) {
                break flags;
            }
            let idle = written.idle(&flags);
            if !idle.is_empty() {
                note = Some(written.idled(&idle));
            }
        }
        // Only the model's to fix: a runaway translation needs retranslating.
        let fixable: Vec<Flag> = flags
            .iter()
            .filter(|flag| flag.kind != FlagKind::Runaway)
            .cloned()
            .collect();
        let Some(page) = page.filter(|_| !fixable.is_empty()) else {
            break flags;
        };
        let model = match &model {
            Some(model) => model,
            None => model.insert(fix::connect(&reference).await?),
        };
        let asked = fix::ask(
            model,
            &page,
            &fixable,
            note.take().as_deref(),
            &mut history,
            config.translator.reasoning,
            &member.cancel,
        )
        .await;
        let fixes = match asked {
            Ok(Some(fixes)) => fixes,
            Ok(None) => return Ok(()),
            Err(error) => {
                report(task, &member, flags);
                return Err(error);
            }
        };
        let (ops, accepts) = fix::ops(&page, &fixes, &fixable, &settings);
        tracing::info!(
            page = member.index + 1,
            round,
            fixed = ops.len(),
            accepted = accepts.len(),
            "validation fixes"
        );
        let before = accepted.len();
        accepted.extend(accepts);
        if ops.is_empty() && accepted.len() == before {
            break flags;
        }
        if !ops.is_empty() {
            let changed: Vec<LayerId> = ops.iter().map(|(block, _)| *block).collect();
            let undo = {
                let project = member.session.lock().project();
                changed
                    .iter()
                    .filter_map(|&block| Some((block, undo(&project, block)?)))
                    .collect()
            };
            last = Some(Round {
                flags,
                undo,
                keys: page
                    .texts
                    .iter()
                    .map(|text| (text.block, text.key.clone()))
                    .collect(),
            });
            member
                .apply(Stage::Validate, move |_| {
                    Ok(ops.into_iter().map(|(_, op)| op).collect())
                })
                .await?;
        }
        round += 1;
    };
    remaining.retain(|flag| !accepted.contains(&(flag.block, flag.kind)));
    report(task, &member, remaining);
    Ok(())
}

/// A round of fixes as written: the flags before it, and how to undo each
/// changed text.
struct Round {
    flags: Vec<Flag>,
    undo: Vec<(LayerId, Op)>,
    /// Each text's key, as the model named it.
    keys: Vec<(LayerId, String)>,
}

impl Round {
    /// The changed texts that `now` shows made something worse: each with
    /// the text the new problem is on and its kind.
    fn worse(&self, now: &[Flag]) -> Vec<(LayerId, LayerId, FlagKind)> {
        let changed = |block: &LayerId| self.undo.iter().any(|(changed, _)| changed == block);
        now.iter()
            .filter(|flag| {
                !self
                    .flags
                    .iter()
                    .any(|before| before.block == flag.block && before.kind == flag.kind)
            })
            .filter_map(|flag| {
                let culprit = if changed(&flag.block) {
                    flag.block
                } else {
                    flag.other.filter(changed)?
                };
                Some((culprit, flag.block, flag.kind))
            })
            .collect()
    }

    /// Whether `now` flags just what the round started with.
    fn same(&self, now: &[Flag]) -> bool {
        let kinds = |flags: &[Flag]| -> HashSet<(LayerId, FlagKind)> {
            flags.iter().map(|flag| (flag.block, flag.kind)).collect()
        };
        kinds(&self.flags) == kinds(now)
    }

    /// The changed texts `now` flags just as before: fixes with no effect.
    fn idle(&self, now: &[Flag]) -> Vec<LayerId> {
        let kinds = |flags: &[Flag], block: LayerId| -> HashSet<FlagKind> {
            flags
                .iter()
                .filter(|flag| flag.block == block)
                .map(|flag| flag.kind)
                .collect()
        };
        self.undo
            .iter()
            .map(|(block, _)| *block)
            .filter(|&block| {
                let before = kinds(&self.flags, block);
                !before.is_empty() && before == kinds(now, block)
            })
            .collect()
    }

    /// What the model hears of the fixes with no effect on `idle`.
    fn idled(&self, idle: &[LayerId]) -> String {
        let keys: Vec<&str> = idle.iter().map(|block| self.key(block)).collect();
        format!(
            "Your fixes of {} changed nothing: accept them, or fix them another way.",
            keys.join(", ")
        )
    }

    fn key(&self, block: &LayerId) -> &str {
        self.keys
            .iter()
            .find(|(text, _)| text == block)
            .map_or("?", |(_, key)| key.as_str())
    }

    /// What the model hears of the fixes undone for `worse`.
    fn told(&self, worse: &[(LayerId, LayerId, FlagKind)]) -> String {
        let key = |block: &LayerId| self.key(block);
        let undone: Vec<String> = worse
            .iter()
            .map(|(culprit, on, kind)| {
                let kind = serde_json::to_value(kind)
                    .ok()
                    .and_then(|kind| kind.as_str().map(str::to_owned))
                    .unwrap_or_default();
                format!(
                    "your fix of {} was undone: it caused {kind} on {}",
                    key(culprit),
                    key(on)
                )
            })
            .collect();
        format!(
            "{}. Fix those texts another way, or accept them.",
            undone.join("; ")
        )
    }
}

/// The op putting `block`'s translation's frame, transform, and the styles a
/// fix sets back as `project` has them; `None` when it has none.
fn undo(project: &Project, block: LayerId) -> Option<Op> {
    let Content::Block(found) = &project.layer(block).ok()?.content else {
        return None;
    };
    let old = serde_json::to_value(found.translation.as_ref()?).ok()?;
    // A merge patch keeps what it doesn't name: the frame kind the fix wrote
    // and the styles it added go by name.
    let mut frame = old["frame"].clone();
    if let Value::Object(kinds) = &mut frame {
        for kind in ["box", "path"] {
            kinds.entry(kind).or_insert(Value::Null);
        }
    }
    let mut chars = match &old["style"]["chars"] {
        Value::Object(chars) => chars.clone(),
        _ => serde_json::Map::new(),
    };
    for style in ["size", "stroke", "stroke_width"] {
        chars.entry(style).or_insert(Value::Null);
    }
    Some(Op::UpdateLayer {
        layer: block.into(),
        patch: json!({"translation": {
            "frame": frame,
            "transform": old["transform"],
            "style": {"chars": chars},
        }}),
    })
}

/// Tells the app what's still flagged on `member`'s page.
fn report(task: &Task, member: &Member, flags: Vec<Flag>) {
    if !flags.is_empty() {
        task.events.send(Event::Flagged {
            run: member.run,
            page: member.page,
            flags,
        });
    }
}

/// `member`'s page measured as it's drawn now: what's flagged and not
/// `accepted`, and when `fixing` with something flagged, the page as the
/// model sees it.
async fn look(
    member: &Member,
    settings: &ValidationConfig,
    accepted: &HashSet<(LayerId, FlagKind)>,
    fixing: bool,
) -> anyhow::Result<(Vec<Flag>, Option<fix::Page>)> {
    let (read, settings, accepted) = (member.clone(), settings.clone(), accepted.clone());
    tokio::task::spawn_blocking(move || {
        let _span =
            info_span!("measure", stage = %Stage::Validate, page = read.index + 1).entered();
        let renderer = Renderer::get();
        let (project, frame) = {
            let session = read.session.lock();
            (session.project(), renderer.render(&session, read.page)?)
        };
        let drawn = frame.rasterize()?.image;
        let size = project.page(read.page)?.size;
        let (width, height) = (f64::from(size.width), f64::from(size.height));
        let measured = measure(&project, read.page, renderer, &frame)?;
        let mut flags = flags(&measured, Rect::new(0.0, 0.0, width, height), &settings);
        // A run on some blocks measures the whole page, for what's beside
        // them, and tells and fixes only what concerns them.
        if let Some(only) = &read.blocks {
            flags.retain(|flag| {
                only.contains(&flag.block) || flag.other.is_some_and(|other| only.contains(&other))
            });
        }
        flags.retain(|flag| !accepted.contains(&(flag.block, flag.kind)));
        let page = if fixing && !flags.is_empty() {
            Some(sight(&measured, &drawn, width, height)?)
        } else {
            None
        };
        Ok((flags, page))
    })
    .await?
}

/// The page as the model sees it: `drawn` over white at `SIGHT`, as a JPEG,
/// and its texts as `measured`.
fn sight(
    measured: &[Measured],
    drawn: &RgbaImage,
    width: f64,
    height: f64,
) -> anyhow::Result<fix::Page> {
    let mut page = RgbaImage::from_pixel(drawn.width(), drawn.height(), Rgba([255; 4]));
    imageops::overlay(&mut page, drawn, 0, 0);
    let page = DynamicImage::ImageRgba8(page)
        .thumbnail(SIGHT, SIGHT)
        .to_rgb8();
    let mut bytes = Vec::new();
    page.write_to(&mut std::io::Cursor::new(&mut bytes), ImageFormat::Jpeg)?;
    let texts = measured
        .iter()
        .enumerate()
        .map(|(at, text)| fix::Text {
            key: format!("t{}", at + 1),
            block: text.block,
            role: text.role,
            text: text.full.clone(),
            size: text.size,
            letters: letters(&text.quads),
            room: text.room,
            balloon: text
                .balloon
                .as_ref()
                .map(|outline| holds(outline.bounding_box())),
            fill: text.fill,
            holder: text.holder,
            transform: text.transform,
        })
        .collect();
    Ok(fix::Page {
        image: base64::engine::general_purpose::STANDARD.encode(bytes),
        scale: f64::from(page.width()) / width,
        width,
        height,
        typical: typical(measured),
        texts,
    })
}

/// Where `quads` are, together, on the page.
fn letters(quads: &[[Point; 4]]) -> Rect {
    quads
        .iter()
        .flatten()
        .fold(None, |bounds: Option<Rect>, &corner| {
            Some(bounds.map_or(Rect::from_points(corner, corner), |b| b.union_pt(corner)))
        })
        .unwrap_or(Rect::ZERO)
}

/// Whether `translation` ran away from `source`: a word said `REPEATS`
/// times in a row, or a long text many times its source's length. A sound
/// effect's few repeats, "clap clap clap", stay well under.
fn runaway(translation: &str, source: &str) -> bool {
    const REPEATS: usize = 8;
    let words: Vec<String> = translation
        .split(|c: char| !c.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .map(str::to_lowercase)
        .collect();
    let (mut run, mut longest) = (1, 1);
    for pair in words.windows(2) {
        run = if pair[0] == pair[1] { run + 1 } else { 1 };
        longest = longest.max(run);
    }
    let length = translation.chars().count();
    longest >= REPEATS || (length > 80 && length > source.chars().count().max(1) * 10)
}

/// The share of `quads` outside `outline`, sampled on a grid across each.
fn outside(quads: &[[Point; 4]], outline: &BezPath) -> f64 {
    const STEPS: usize = 8;
    let (mut out, mut all) = (0usize, 0usize);
    for quad in quads {
        for across in 0..STEPS {
            for down in 0..STEPS {
                let u = (across as f64 + 0.5) / STEPS as f64;
                let v = (down as f64 + 0.5) / STEPS as f64;
                let point = quad[0].lerp(quad[1], u).lerp(quad[3].lerp(quad[2], u), v);
                all += 1;
                if !outline.contains(point) {
                    out += 1;
                }
            }
        }
    }
    if all == 0 {
        0.0
    } else {
        out as f64 / all as f64
    }
}

/// The rectangle a balloon of `bounds` safely holds text in: about its
/// centre, as large as an oval of those bounds holds, 1/√2 of each side.
fn holds(bounds: Rect) -> Rect {
    let scale = std::f64::consts::FRAC_1_SQRT_2;
    Rect::from_center_size(
        bounds.center(),
        (bounds.width() * scale, bounds.height() * scale),
    )
}

/// The page's typical dialogue size: the median of what its dialogue is
/// drawn at, when it has a few texts to stand for it.
fn typical(measured: &[Measured]) -> Option<f32> {
    let mut dialogue: Vec<f32> = measured
        .iter()
        .filter(|text| text.role == Role::Text)
        .map(|text| text.size)
        .collect();
    dialogue.sort_by(f32::total_cmp);
    (dialogue.len() >= 3).then(|| dialogue[dialogue.len() / 2])
}

/// The page's visible translated texts as `renderer` lays them out and
/// `frame` draws them.
fn measure(
    project: &Project,
    page: koharu_scene::PageId,
    renderer: &Renderer,
    frame: &RendererFrame,
) -> anyhow::Result<Vec<Measured>> {
    // The page without its translations: what's behind each text's letters,
    // the clean-up included, whatever its colour.
    let translated: Vec<LayerId> = project
        .blocks(page)?
        .into_iter()
        .filter(|(_, block)| block.translation.is_some())
        .map(|(id, _)| id)
        .collect();
    let bare = frame.without(&translated).rasterize()?;
    let mut measured = Vec::new();
    for (id, block) in project.blocks(page)? {
        let Some(translation) = &block.translation else {
            continue;
        };
        if !project.layer(id)?.visible {
            continue;
        }
        let Some(resolved) = renderer.layout(project, id, translation)? else {
            continue;
        };
        let transform = Affine::from(translation.transform);
        let scale = transform.determinant().abs().sqrt() as f32;
        let size = resolved.runs.iter().map(|run| run.size).fold(0.0, f32::max) * scale;
        let quads: Vec<[Point; 4]> = resolved
            .lines
            .iter()
            .map(|strip| {
                [
                    Point::new(strip.x0, strip.y0),
                    Point::new(strip.x1, strip.y0),
                    Point::new(strip.x1, strip.y1),
                    Point::new(strip.x0, strip.y1),
                ]
                .map(|corner| transform * corner)
            })
            .collect();
        let outlined = resolved.runs.iter().any(|run| run.stroke.is_some());
        let fill = resolved
            .runs
            .first()
            .map(|run| run.fill.components)
            .unwrap_or([0.0, 0.0, 0.0, 1.0]);
        let balloon = match project.parent(id)?.1 {
            Some(Layer {
                content: Content::Balloon(balloon),
                ..
            }) => Some(BezPath::from(&balloon.outline)),
            _ => None,
        };
        let behind = match frame.translation(id).filter(|_| !outlined) {
            Some(drawn) => behind(&bare, &drawn.cropped().rasterize()?, fill),
            None => None,
        };
        tracing::debug!(text = %excerpt(translation), ?behind, outlined, "validation behind");
        let holder = match translation.frame {
            Frame::Box { .. } => Holder::Box,
            Frame::Path { .. } => Holder::Path,
            Frame::Point => Holder::Point,
        };
        measured.push(Measured {
            block: id,
            role: block.role,
            text: excerpt(translation),
            full: whole(translation),
            source: whole(&block.source),
            room: translation.bounds(),
            holder,
            transform,
            size,
            quads,
            overflowed: resolved.overflowed,
            outlined,
            fill: luminance(fill.map(f64::from)),
            balloon,
            behind,
        });
    }
    Ok(measured)
}

/// What validation flags among `measured`, on a page of `page` bounds.
pub(crate) fn flags(measured: &[Measured], page: Rect, config: &ValidationConfig) -> Vec<Flag> {
    let typical = typical(measured);
    let minimum = (page.height() * config.min_size) as f32;
    let mut flags = Vec::new();
    for text in measured {
        let flag = |kind| Flag {
            block: text.block,
            kind,
            text: text.text.clone(),
            size: text.size,
            typical,
            other: None,
            contrast: None,
        };
        // A sound effect's size is the artist's to choose, small or big;
        // only one over dialogue is flagged, below.
        let dialogue = text.role == Role::Text;
        let below_typical =
            typical.is_some_and(|typical| f64::from(text.size) < f64::from(typical) * config.small);
        if dialogue && (text.size < minimum || below_typical) {
            flags.push(flag(FlagKind::TooSmall));
        }
        if dialogue
            && typical.is_some_and(|typical| f64::from(text.size) > f64::from(typical) * config.big)
        {
            flags.push(flag(FlagKind::TooBig));
        }
        if text.overflowed {
            flags.push(flag(FlagKind::Overflow));
        }
        if runaway(&text.full, &text.source) {
            flags.push(flag(FlagKind::Runaway));
        }
        let area: f64 = text.quads.iter().map(|quad| polygon_area(quad)).sum();
        if area > 0.0 {
            let inside = |bounds: Rect| -> f64 {
                text.quads
                    .iter()
                    .map(|quad| polygon_area(&clip(quad, &rect_quad(bounds))))
                    .sum()
            };
            if (area - inside(page)) / area > 0.02 {
                flags.push(flag(FlagKind::OffPage));
            }
            // Against the outline itself: an oval's corners aren't inside it.
            if let Some(balloon) = &text.balloon
                && outside(&text.quads, balloon) > 0.05
            {
                flags.push(flag(FlagKind::OutsideBalloon));
            }
        }
        if !text.outlined
            && let Some(behind) = text.behind
        {
            let ratio = contrast(text.fill, behind.luminance);
            if ratio < config.contrast {
                flags.push(Flag {
                    contrast: Some(ratio as f32),
                    ..flag(FlagKind::LowContrast)
                });
            } else if behind.similar > config.busy {
                flags.push(Flag {
                    contrast: Some(ratio as f32),
                    ..flag(FlagKind::BusyBackground)
                });
            }
        }
    }
    // Overlaps, told on the dialogue a sound effect covers, or the first of
    // two dialogues; sound effects may cross each other.
    for (at, text) in measured.iter().enumerate() {
        for other in &measured[at + 1..] {
            let (kind, flagged, by) = match (text.role, other.role) {
                (Role::Sfx, Role::Sfx) => continue,
                (Role::Sfx, Role::Text) => (FlagKind::SfxOverlap, other, text),
                (Role::Text, Role::Sfx) => (FlagKind::SfxOverlap, text, other),
                (Role::Text, Role::Text) => (FlagKind::Overlap, text, other),
            };
            let smaller = [text, other]
                .map(|text| {
                    text.quads
                        .iter()
                        .map(|quad| polygon_area(quad))
                        .sum::<f64>()
                })
                .into_iter()
                .fold(f64::INFINITY, f64::min);
            if smaller <= 0.0 {
                continue;
            }
            let shared: f64 = text
                .quads
                .iter()
                .flat_map(|a| other.quads.iter().map(move |b| polygon_area(&clip(a, b))))
                .sum();
            if shared / smaller > config.overlap {
                flags.push(Flag {
                    block: flagged.block,
                    kind,
                    text: flagged.text.clone(),
                    size: flagged.size,
                    typical,
                    other: Some(by.block),
                    contrast: None,
                });
            }
        }
    }
    flags
}

/// What's under and right around a text's letters on `bare`, the page
/// without its translations: `letters` is the text drawn alone, whose opaque
/// pixels are its letters, and `RING` pixels round them count, as art that
/// touches a letter muddles it as much as art under it. `None` with too few
/// to tell.
fn behind(bare: &Raster, letters: &Raster, fill: [f32; 4]) -> Option<Behind> {
    /// How far round a letter art still muddles it, in pixels.
    const RING: i64 = 3;
    let text = luminance(fill.map(f64::from));
    let (width, height) = (
        i64::from(letters.image.width()),
        i64::from(letters.image.height()),
    );
    let ink = |x: i64, y: i64| {
        (0..width).contains(&x)
            && (0..height).contains(&y)
            && letters.image.get_pixel(x as u32, y as u32).0[3] > 128
    };
    let (mut luminances, mut similar) = (Vec::new(), 0usize);
    for y in -RING..height + RING {
        for x in -RING..width + RING {
            let near = (-RING..=RING).any(|dy| (-RING..=RING).any(|dx| ink(x + dx, y + dy)));
            if !near {
                continue;
            }
            // The same place on the bare page.
            let px = i64::from(letters.offset.x) + x - i64::from(bare.offset.x);
            let py = i64::from(letters.offset.y) + y - i64::from(bare.offset.y);
            if !(0..i64::from(bare.image.width())).contains(&px)
                || !(0..i64::from(bare.image.height())).contains(&py)
            {
                continue;
            }
            // Over white, as the page shows where nothing's drawn.
            let [r, g, b, a] = bare
                .image
                .get_pixel(px as u32, py as u32)
                .0
                .map(|channel| f64::from(channel) / 255.0);
            let over = |channel: f64| channel * a + (1.0 - a);
            let under = luminance([over(r), over(g), over(b), 1.0]);
            if contrast(text, under) < 2.0 {
                similar += 1;
            }
            luminances.push(under);
        }
    }
    if luminances.len() < 32 {
        return None;
    }
    let count = luminances.len() as f64;
    Some(Behind {
        luminance: luminances.iter().sum::<f64>() / count,
        similar: similar as f64 / count,
    })
}

/// The contrast ratio of two relative luminances, as WCAG 2 defines it.
fn contrast(a: f64, b: f64) -> f64 {
    (a.max(b) + 0.05) / (a.min(b) + 0.05)
}

/// An sRGB colour's relative luminance, as WCAG 2 defines it.
fn luminance([r, g, b, _]: [f64; 4]) -> f64 {
    let linear = |channel: f64| {
        if channel <= 0.04045 {
            channel / 12.92
        } else {
            ((channel + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * linear(r) + 0.7152 * linear(g) + 0.0722 * linear(b)
}

/// `text`'s content on one line.
fn whole(text: &Text) -> String {
    let content: String = text
        .content
        .0
        .iter()
        .map(|span| span.insert.as_str())
        .collect();
    content.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The start of `text`'s content, on one line.
fn excerpt(text: &Text) -> String {
    let line = whole(text);
    match line.char_indices().nth(40) {
        Some((at, _)) => format!("{}…", &line[..at]),
        None => line,
    }
}

fn rect_quad(rect: Rect) -> [Point; 4] {
    [
        Point::new(rect.x0, rect.y0),
        Point::new(rect.x1, rect.y0),
        Point::new(rect.x1, rect.y1),
        Point::new(rect.x0, rect.y1),
    ]
}

/// The signed area's size, by the shoelace formula.
fn polygon_area(points: &[Point]) -> f64 {
    let n = points.len();
    (0..n)
        .map(|at| {
            let (a, b) = (points[at], points[(at + 1) % n]);
            a.x * b.y - b.x * a.y
        })
        .sum::<f64>()
        .abs()
        / 2.0
}

/// Which side of the edge `a`→`b` `point` is on: positive on its left.
fn side(a: Point, b: Point, point: Point) -> f64 {
    (b.x - a.x) * (point.y - a.y) - (b.y - a.y) * (point.x - a.x)
}

/// `subject` cut to the convex `window`, by Sutherland and Hodgman's
/// clipping.
fn clip(subject: &[Point], window: &[Point; 4]) -> Vec<Point> {
    // The window's inside is on whichever side its winding puts it.
    let winding = side(window[0], window[1], window[2]).signum();
    let mut output = subject.to_vec();
    for at in 0..4 {
        let (a, b) = (window[at], window[(at + 1) % 4]);
        let inside = |point: Point| side(a, b, point) * winding >= 0.0;
        let input = std::mem::take(&mut output);
        for (index, &current) in input.iter().enumerate() {
            let previous = input[(index + input.len() - 1) % input.len()];
            let crossing = |p: Point, q: Point| {
                let (sp, sq) = (side(a, b, p), side(a, b, q));
                p + (q - p) * (sp / (sp - sq))
            };
            match (inside(current), inside(previous)) {
                (true, true) => output.push(current),
                (true, false) => {
                    output.push(crossing(previous, current));
                    output.push(current);
                }
                (false, true) => output.push(crossing(previous, current)),
                (false, false) => {}
            }
        }
        if output.is_empty() {
            break;
        }
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(n: u64) -> LayerId {
        format!("00000000-0000-0000-0000-{n:012}")
            .parse()
            .expect("an id")
    }

    fn text(block: u64, role: Role, size: f32, rect: Rect) -> Measured {
        Measured {
            block: id(block),
            role,
            text: String::new(),
            size,
            quads: vec![rect_quad(rect)],
            overflowed: false,
            outlined: false,
            fill: 0.0,
            balloon: None,
            behind: None,
            full: String::new(),
            source: String::new(),
            room: None,
            holder: Holder::Box,
            transform: Affine::IDENTITY,
        }
    }

    fn kinds(flags: &[Flag]) -> Vec<(LayerId, FlagKind)> {
        flags.iter().map(|flag| (flag.block, flag.kind)).collect()
    }

    const PAGE: Rect = Rect::new(0.0, 0.0, 1000.0, 1500.0);

    #[test]
    fn dialogue_far_from_the_pages_size_is_flagged_but_a_sound_effect_never_is() {
        let texts = [
            text(1, Role::Text, 30.0, Rect::new(0.0, 0.0, 100.0, 40.0)),
            text(2, Role::Text, 30.0, Rect::new(0.0, 100.0, 100.0, 140.0)),
            text(3, Role::Text, 14.0, Rect::new(0.0, 200.0, 100.0, 240.0)),
            text(4, Role::Text, 60.0, Rect::new(0.0, 300.0, 100.0, 340.0)),
            text(5, Role::Sfx, 120.0, Rect::new(500.0, 300.0, 900.0, 500.0)),
            // Below the page's smallest, which dialogue may not be.
            text(6, Role::Sfx, 6.0, Rect::new(500.0, 600.0, 520.0, 606.0)),
        ];
        let flags = flags(&texts, PAGE, &ValidationConfig::default());
        assert_eq!(
            kinds(&flags),
            [(id(3), FlagKind::TooSmall), (id(4), FlagKind::TooBig)]
        );
        assert_eq!(flags[0].typical, Some(30.0));
    }

    #[test]
    fn boxes_that_meet_without_their_letters_meeting_dont_overlap() {
        // Two short lines side by side, apart.
        let apart = [
            text(1, Role::Text, 30.0, Rect::new(100.0, 100.0, 200.0, 130.0)),
            text(2, Role::Sfx, 30.0, Rect::new(210.0, 100.0, 300.0, 130.0)),
        ];
        assert!(flags(&apart, PAGE, &ValidationConfig::default()).is_empty());
        // A sound effect across the dialogue's line.
        let across = [
            text(1, Role::Text, 30.0, Rect::new(100.0, 100.0, 200.0, 130.0)),
            text(2, Role::Sfx, 30.0, Rect::new(150.0, 90.0, 260.0, 140.0)),
        ];
        let flagged = flags(&across, PAGE, &ValidationConfig::default());
        assert_eq!(kinds(&flagged), [(id(1), FlagKind::SfxOverlap)]);
        assert_eq!(flagged[0].other, Some(id(2)));
    }

    #[test]
    fn a_turned_line_is_measured_as_turned() {
        // A diamond whose bounding box covers the other text, but whose
        // letters don't.
        let mut turned = text(1, Role::Sfx, 30.0, Rect::ZERO);
        turned.quads = vec![[
            Point::new(100.0, 0.0),
            Point::new(200.0, 100.0),
            Point::new(100.0, 200.0),
            Point::new(0.0, 100.0),
        ]];
        let corner = text(2, Role::Text, 30.0, Rect::new(160.0, 160.0, 200.0, 200.0));
        assert!(flags(&[turned, corner], PAGE, &ValidationConfig::default()).is_empty());
    }

    #[test]
    fn text_off_the_page_or_out_of_its_balloon_is_flagged() {
        let off = text(1, Role::Text, 30.0, Rect::new(950.0, 100.0, 1100.0, 130.0));
        let mut out = text(2, Role::Text, 30.0, Rect::new(100.0, 500.0, 300.0, 530.0));
        out.balloon = Some(Rect::new(120.0, 480.0, 240.0, 560.0).to_path(0.1));
        // Inside a round balloon's rectangle, but over its curve.
        let mut corner = text(3, Role::Text, 30.0, Rect::new(400.0, 400.0, 440.0, 420.0));
        corner.balloon =
            Some(kurbo::Ellipse::from_rect(Rect::new(400.0, 400.0, 600.0, 600.0)).to_path(0.1));
        let mut centred = text(4, Role::Text, 30.0, Rect::new(460.0, 490.0, 540.0, 510.0));
        centred.balloon = corner.balloon.clone();
        assert_eq!(
            kinds(&flags(
                &[off, out, corner, centred],
                PAGE,
                &ValidationConfig::default()
            )),
            [
                (id(1), FlagKind::OffPage),
                (id(2), FlagKind::OutsideBalloon),
                (id(3), FlagKind::OutsideBalloon)
            ]
        );
    }

    #[test]
    fn text_without_an_outline_needs_contrast_and_a_calm_background() {
        let mut grey_on_grey = text(1, Role::Sfx, 30.0, Rect::new(0.0, 0.0, 10.0, 10.0));
        grey_on_grey.fill = 0.2;
        grey_on_grey.behind = Some(Behind {
            luminance: 0.25,
            similar: 0.0,
        });
        let mut busy = text(2, Role::Sfx, 30.0, Rect::new(0.0, 100.0, 10.0, 110.0));
        busy.behind = Some(Behind {
            luminance: 0.6,
            similar: 0.3,
        });
        let mut outlined = busy.clone();
        outlined.block = id(3);
        outlined.outlined = true;
        assert_eq!(
            kinds(&flags(
                &[grey_on_grey, busy, outlined],
                PAGE,
                &ValidationConfig::default()
            )),
            [
                (id(1), FlagKind::LowContrast),
                (id(2), FlagKind::BusyBackground)
            ]
        );
    }

    #[test]
    fn a_looping_translation_runs_away_but_a_sound_effects_repeats_dont() {
        let splat = format!("*{}splat*", "splat ".repeat(59));
        assert!(runaway(&splat, "ぐちゅ"));
        assert!(!runaway("*clap clap clap clap clap clap*", "パンパン"));
        assert!(!runaway(
            "Even if the person next to you isn't me,",
            "隣にいるのが私じゃなくても"
        ));
        // A long answer to a word.
        assert!(runaway(&"and then she said that ".repeat(5), "あ"));
    }

    #[test]
    fn a_fix_is_worse_when_it_brings_its_text_or_a_neighbour_a_new_problem() {
        let flag = |n: u64, kind, other: Option<u64>| Flag {
            block: id(n),
            kind,
            text: String::new(),
            size: 20.0,
            typical: None,
            other: other.map(id),
            contrast: None,
        };
        let written = Round {
            flags: vec![flag(1, FlagKind::TooSmall, None)],
            undo: vec![(
                id(1),
                Op::UpdateLayer {
                    layer: id(1).into(),
                    patch: json!({}),
                },
            )],
            keys: vec![(id(1), "t1".to_owned()), (id(2), "t2".to_owned())],
        };
        // Still too small isn't worse; busy behind it now is, and so is the
        // text it moved onto.
        assert!(
            written
                .worse(&[flag(1, FlagKind::TooSmall, None)])
                .is_empty()
        );
        let worse = written.worse(&[
            flag(1, FlagKind::BusyBackground, None),
            flag(2, FlagKind::Overlap, Some(1)),
            flag(2, FlagKind::TooBig, None),
        ]);
        assert_eq!(
            worse,
            [
                (id(1), id(1), FlagKind::BusyBackground),
                (id(1), id(2), FlagKind::Overlap)
            ]
        );
        assert!(
            written
                .told(&worse)
                .contains("t1 was undone: it caused overlap on t2")
        );
        // A fix that left its text flagged as before had no effect, and a
        // round that changed no flag at all is the last.
        let still = [flag(1, FlagKind::TooSmall, None)];
        assert!(written.same(&still));
        assert_eq!(written.idle(&still), [id(1)]);
        assert!(!written.same(&[]));
        assert!(written.idle(&[]).is_empty());
    }

    #[test]
    fn an_undone_fix_puts_the_translation_back_as_it_was() -> anyhow::Result<()> {
        use koharu_scene::{Created, Edit, Session};

        let dir = tempfile::tempdir()?;
        let mut session = Session::create(dir.path().join("test.khr"), koharu_scene::Meta::default())?;
        let block = json!({"kind": "block", "role": "text",
        "source": {"frame": {"box": {"bounds": {"x": 10.0, "y": 10.0, "width": 40.0, "height": 120.0}}}},
        "translation": {
            "content": "Hey",
            "frame": {"path": {"path": "M0,0 L100,0 L100,100 L0,100 Z", "fit": "best"}},
            "style": {"chars": {"size": 20.0}},
        }});
        let change = session.apply(Edit {
            label: "test".to_owned(),
            ops: [
                json!({"op": "add_page", "id": "$p", "index": 0, "size": {"width": 400, "height": 400}}),
                json!({"op": "add_layer", "id": "$b", "page": "$p", "index": 0, "layer": block}),
            ]
            .into_iter()
            .map(serde_json::from_value)
            .collect::<Result<_, _>>()?,
        })?;
        let block = change
            .created
            .iter()
            .find_map(|created| match created {
                Created::Layer {
                    id,
                    label: Some(label),
                } if label == "b" => Some(*id),
                _ => None,
            })
            .expect("the block");
        let translation =
            |session: &Session| match &session.project().layer(block).expect("the block").content {
                Content::Block(found) => found.translation.clone(),
                _ => None,
            };
        let before = translation(&session);
        let undo = undo(&session.project(), block).expect("an undo");
        // Every kind of fix at once: a box, a size, and an outline.
        session.apply(Edit {
            label: "validate".to_owned(),
            ops: vec![Op::UpdateLayer {
                layer: block.into(),
                patch: json!({"translation": {
                    "frame": {"box": {"bounds": {"x": 5.0, "y": 5.0, "width": 90.0, "height": 50.0}, "fit": "shrink"}, "path": null},
                    "style": {"chars": {"size": 30.0, "stroke": [1.0, 1.0, 1.0, 1.0], "stroke_width": 3.0}},
                }}),
            }],
        })?;
        assert_ne!(translation(&session), before);
        session.apply(Edit {
            label: "validate".to_owned(),
            ops: vec![undo],
        })?;
        assert_eq!(translation(&session), before);
        Ok(())
    }
}
