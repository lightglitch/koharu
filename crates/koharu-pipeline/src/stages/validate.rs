//! Validation: each translated text of a page measured as it's drawn, and
//! flagged where a reader would trip on it: too small or too big beside the
//! page's other dialogue, past its box, over another text, off the page or
//! out of its balloon, or hard to tell from what's behind it. It changes
//! nothing; what it flags goes out as an event, for the reader or a later
//! fix.
// Fork addition: upstream has no validation stage.

use image::RgbaImage;
use koharu_renderer::Renderer;
use koharu_scene::{Content, Layer, LayerId, Project, Role, Text};
use kurbo::{Affine, BezPath, Point, Rect, Shape as _};
use serde::{Deserialize, Serialize};
use tracing::info_span;

use super::Task;
use crate::{Event, PipelineConfig, Stage};

/// `[pipeline.validation]`: when validation flags a text. Sizes are the
/// size a text is drawn at, after fitting, in page pixels.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
#[derive(specta::Type)]
pub struct ValidationConfig {
    /// The smallest any text may be drawn, as a share of the page's height.
    pub min_size: f64,
    /// Dialogue drawn below this share of the page's typical dialogue size
    /// is too small.
    pub small: f64,
    /// Dialogue drawn above this share of the page's typical dialogue size
    /// is too big; a sound effect never is.
    pub big: f64,
    /// Two texts overlap when this share of the smaller one's lines lies
    /// under the other's.
    pub overlap: f64,
    /// A text without an outline needs one below this contrast ratio, as
    /// WCAG measures it, against what's behind its letters.
    pub contrast: f64,
    /// A text without an outline needs one when more than this share of
    /// what's behind its letters, their soft edges left out, is nearly as
    /// dark as the text, or as light: the lines of artwork, not a balloon.
    pub busy: f64,
}

impl Default for ValidationConfig {
    fn default() -> Self {
        Self {
            min_size: 0.007,
            small: 0.6,
            big: 1.6,
            overlap: 0.05,
            contrast: 3.0,
            busy: 0.15,
        }
    }
}

/// What's wrong with a text.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
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
    /// Its balloon's bounds, when it's in one.
    balloon: Option<Rect>,
    behind: Option<Behind>,
}

/// Validates the task's page.
pub(crate) async fn validate(task: &Task, config: &PipelineConfig) -> anyhow::Result<()> {
    let member = task.members[0].clone();
    task.started(&[&member]);
    let settings = config.validation;
    let read = member.clone();
    let flags = tokio::task::spawn_blocking(move || -> anyhow::Result<Vec<Flag>> {
        let _span =
            info_span!("measure", stage = %Stage::Validate, page = read.index + 1).entered();
        let renderer = Renderer::get();
        let (project, page) = {
            let session = read.session.lock();
            let page = renderer.render(&session, read.page)?.rasterize()?;
            (session.project(), page)
        };
        let size = project.page(read.page)?.size;
        let measured = measure(&project, read.page, renderer, &page.image)?;
        let mut flags = flags(
            &measured,
            Rect::new(0.0, 0.0, f64::from(size.width), f64::from(size.height)),
            &settings,
        );
        // A run on some blocks measures the whole page, for what's beside
        // them, and tells only what concerns them.
        if let Some(only) = &read.blocks {
            flags.retain(|flag| {
                only.contains(&flag.block) || flag.other.is_some_and(|other| only.contains(&other))
            });
        }
        Ok(flags)
    })
    .await??;
    if !flags.is_empty() {
        task.events.send(Event::Flagged {
            run: member.run,
            page: member.page,
            flags,
        });
    }
    Ok(())
}

/// The page's visible translated texts as `renderer` draws them, `page` its
/// drawing in page pixels.
fn measure(
    project: &Project,
    page: koharu_scene::PageId,
    renderer: &Renderer,
    drawn: &RgbaImage,
) -> anyhow::Result<Vec<Measured>> {
    let mut measured = Vec::new();
    for (id, block) in project.blocks(page)? {
        let Some(translation) = &block.translation else {
            continue;
        };
        if !project.layer(id)?.visible {
            continue;
        }
        let Some(resolved) = renderer.layout(id, translation)? else {
            continue;
        };
        let transform = Affine::from(translation.transform);
        let scale = transform.determinant().abs().sqrt() as f32;
        let size = resolved.runs.iter().map(|run| run.size).fold(0.0, f32::max) * scale;
        let quads: Vec<[Point; 4]> = resolved
            .strips
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
            }) => Some(BezPath::from(&balloon.outline).bounding_box()),
            _ => None,
        };
        let behind = (!outlined).then(|| behind(drawn, &quads, fill)).flatten();
        measured.push(Measured {
            block: id,
            role: block.role,
            text: excerpt(translation),
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
    let mut dialogue: Vec<f32> = measured
        .iter()
        .filter(|text| text.role == Role::Text)
        .map(|text| text.size)
        .collect();
    dialogue.sort_by(f32::total_cmp);
    // A typical size needs a few texts to stand for the page.
    let typical = (dialogue.len() >= 3).then(|| dialogue[dialogue.len() / 2]);
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
        let dialogue = text.role == Role::Text;
        let below_typical = dialogue
            && typical
                .is_some_and(|typical| f64::from(text.size) < f64::from(typical) * config.small);
        if text.size < minimum || below_typical {
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
            if let Some(balloon) = text.balloon
                && (area - inside(balloon)) / area > 0.1
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

/// What's behind `quads` on `drawn`. The pixels near `fill` are taken for
/// the letters, and left out with a halo of `HALO` pixels round them, whose
/// blend of letter and background isn't the background; `None` with too few
/// left to tell.
fn behind(drawn: &RgbaImage, quads: &[[Point; 4]], fill: [f32; 4]) -> Option<Behind> {
    /// How far a letter's soft edge reaches, in pixels.
    const HALO: i64 = 2;
    let text = luminance(fill.map(f64::from));
    let fill = fill.map(|channel| f64::from(channel) * 255.0);
    let (mut luminances, mut similar) = (Vec::new(), 0usize);
    for quad in quads {
        let bounds = quad
            .iter()
            .fold(Rect::from_points(quad[0], quad[0]), |rect, &corner| {
                rect.union_pt(corner)
            });
        let (x0, y0) = (
            bounds.x0.floor().max(0.0) as u32,
            bounds.y0.floor().max(0.0) as u32,
        );
        let x1 = (bounds.x1.ceil() as u32).min(drawn.width());
        let y1 = (bounds.y1.ceil() as u32).min(drawn.height());
        if x1 <= x0 || y1 <= y0 {
            continue;
        }
        let (width, height) = ((x1 - x0) as i64, (y1 - y0) as i64);
        let at = |x: i64, y: i64| (y * width + x) as usize;
        let mut ink = vec![false; (width * height) as usize];
        for y in 0..height {
            for x in 0..width {
                let [r, g, b, _] = drawn
                    .get_pixel(x0 + x as u32, y0 + y as u32)
                    .0
                    .map(f64::from);
                ink[at(x, y)] =
                    (r - fill[0]).abs() + (g - fill[1]).abs() + (b - fill[2]).abs() < 96.0;
            }
        }
        for y in 0..height {
            for x in 0..width {
                let point = Point::new(
                    f64::from(x0) + x as f64 + 0.5,
                    f64::from(y0) + y as f64 + 0.5,
                );
                if !contains(quad, point) {
                    continue;
                }
                let near_ink = (-HALO..=HALO).any(|dy| {
                    (-HALO..=HALO).any(|dx| {
                        let (nx, ny) = (x + dx, y + dy);
                        (0..width).contains(&nx) && (0..height).contains(&ny) && ink[at(nx, ny)]
                    })
                });
                if near_ink {
                    continue;
                }
                let [r, g, b, _] = drawn
                    .get_pixel(x0 + x as u32, y0 + y as u32)
                    .0
                    .map(|channel| f64::from(channel) / 255.0);
                let behind = luminance([r, g, b, 1.0]);
                if contrast(text, behind) < 2.0 {
                    similar += 1;
                }
                luminances.push(behind);
            }
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

/// The start of `text`'s content, on one line.
fn excerpt(text: &Text) -> String {
    let content: String = text
        .content
        .0
        .iter()
        .map(|span| span.insert.as_str())
        .collect();
    let line = content.split_whitespace().collect::<Vec<_>>().join(" ");
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

/// Whether the convex `quad` holds `point`, either winding.
fn contains(quad: &[Point; 4], point: Point) -> bool {
    let sides = (0..4).map(|at| side(quad[at], quad[(at + 1) % 4], point));
    let (mut left, mut right) = (false, false);
    for value in sides {
        left |= value > 0.0;
        right |= value < 0.0;
    }
    !(left && right)
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
        }
    }

    fn kinds(flags: &[Flag]) -> Vec<(LayerId, FlagKind)> {
        flags.iter().map(|flag| (flag.block, flag.kind)).collect()
    }

    const PAGE: Rect = Rect::new(0.0, 0.0, 1000.0, 1500.0);

    #[test]
    fn dialogue_far_from_the_pages_size_is_flagged_but_a_sound_effect_isnt_too_big() {
        let texts = [
            text(1, Role::Text, 30.0, Rect::new(0.0, 0.0, 100.0, 40.0)),
            text(2, Role::Text, 30.0, Rect::new(0.0, 100.0, 100.0, 140.0)),
            text(3, Role::Text, 14.0, Rect::new(0.0, 200.0, 100.0, 240.0)),
            text(4, Role::Text, 60.0, Rect::new(0.0, 300.0, 100.0, 340.0)),
            text(5, Role::Sfx, 120.0, Rect::new(500.0, 300.0, 900.0, 500.0)),
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
        out.balloon = Some(Rect::new(120.0, 480.0, 240.0, 560.0));
        assert_eq!(
            kinds(&flags(&[off, out], PAGE, &ValidationConfig::default())),
            [
                (id(1), FlagKind::OffPage),
                (id(2), FlagKind::OutsideBalloon)
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
}
