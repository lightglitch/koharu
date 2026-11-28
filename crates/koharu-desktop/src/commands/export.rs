//! Export: every page, or some of them, as PNG, JPEG, WebP, or PSD files in a
//! folder, as a CBZ of images, or as a PDF, telling its progress as it goes
//! until stopped.

use std::{
    io::Write as _,
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use anyhow::{Context as _, anyhow, ensure};
use fs_err::File;
use image::{
    DynamicImage, Rgba, RgbaImage,
    codecs::{
        jpeg::JpegEncoder,
        png::{CompressionType, FilterType, PngEncoder},
    },
    imageops,
};
use koharu_psd::Labels;
use koharu_renderer::Renderer;
use koharu_scene::{PageId, Session};
use krilla::{Document, geom::Size, image::Image, page::PageSettings};
use parking_lot::Mutex;
use serde::{Deserialize, Deserializer, Serialize, de::Error as _};
use specta::Type;
use tauri::{State, Window, ipc::Channel};
use tauri_runtime_cef::CefRuntime;

use super::{Result, import::PDF_SCALE};
use crate::app::App;

/// What the pages go out as: a folder of images or of PSDs, a CBZ of
/// images, or a PDF.
#[derive(Clone, Debug, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum ExportFormat {
    Images(ImageFormat),
    /// With the names of the layers the scene names nothing, in the
    /// interface's language.
    Psd(Labels),
    Cbz(ImageFormat),
    Pdf,
}

/// How a flattened page is written: PNG's without loss, JPEG's and WebP's at
/// a quality from 1 to 100.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum ImageFormat {
    Png,
    Jpeg { quality: u8 },
    Webp { quality: u8 },
}

impl ImageFormat {
    fn extension(self) -> &'static str {
        match self {
            Self::Png => "png",
            Self::Jpeg { .. } => "jpg",
            Self::Webp { .. } => "webp",
        }
    }

    fn encode(self, image: RgbaImage) -> anyhow::Result<Vec<u8>> {
        let mut bytes = Vec::new();
        match self {
            // The png crate's own default compression: image's, the fastest,
            // leaves a page in near-grays twice the size.
            Self::Png => {
                let encoder = PngEncoder::new_with_quality(
                    &mut bytes,
                    CompressionType::Default,
                    FilterType::Adaptive,
                );
                if image.pixels().all(|pixel| pixel[3] == u8::MAX) {
                    opaque(image).write_with_encoder(encoder)?
                } else {
                    image.write_with_encoder(encoder)?
                }
            }
            // JPEG has no transparency, so the page is drawn over white, as
            // Photoshop's JPEG export does. Overlay's rounding can leave that
            // white's alpha at 254.
            Self::Jpeg { quality } => {
                let mut white =
                    RgbaImage::from_pixel(image.width(), image.height(), Rgba([255; 4]));
                imageops::overlay(&mut white, &image, 0, 0);
                opaque(white)
                    .write_with_encoder(JpegEncoder::new_with_quality(&mut bytes, quality))?
            }
            // libwebp leaves out the alpha of a page with no transparency.
            Self::Webp { quality } => {
                let (width, height) = image.dimensions();
                // libwebp's `WEBP_MAX_DIMENSION`.
                ensure!(
                    width.max(height) <= 16383,
                    "a WebP image is at most 16383 pixels a side, and the page is {width}×{height}"
                );
                bytes = webp::Encoder::from_rgba(image.as_raw(), width, height)
                    .encode_simple(false, quality.into())
                    .map_err(|error| anyhow!("libwebp failed with {error:?}"))?
                    .to_vec();
            }
        }
        Ok(bytes)
    }
}

/// An opaque `image`, its alpha dropped unread, in the fewest channels that
/// hold it exactly: gray when every pixel is a gray, RGB otherwise.
fn opaque(image: RgbaImage) -> DynamicImage {
    let gray = image.pixels().all(|Rgba([r, g, b, _])| r == g && g == b);
    let image = DynamicImage::ImageRgba8(image);
    if gray {
        DynamicImage::ImageLuma8(image.to_luma8())
    } else {
        DynamicImage::ImageRgb8(image.to_rgb8())
    }
}

// Fork addition: upstream's export can't be followed or stopped, and its
// dialog starts a JPEG's or WebP's quality at 90 each time the app starts.

/// `[export]`: the quality the export dialog starts a JPEG or WebP at, and
/// the command line's, from 1 to 100. A quality out of that range fails to
/// parse.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(default)]
pub struct ExportConfig {
    #[serde(deserialize_with = "quality")]
    #[specta(type = u8)]
    pub jpeg_quality: u8,
    #[serde(deserialize_with = "quality")]
    #[specta(type = u8)]
    pub webp_quality: u8,
}

impl Default for ExportConfig {
    fn default() -> Self {
        Self {
            jpeg_quality: 90,
            webp_quality: 85,
        }
    }
}

impl koharu_config::Section for ExportConfig {
    const NAME: &'static str = "export";
}

fn quality<'de, D: Deserializer<'de>>(deserializer: D) -> std::result::Result<u8, D::Error> {
    let quality = u8::deserialize(deserializer)?;
    match quality {
        1..=100 => Ok(quality),
        _ => Err(D::Error::custom(format!(
            "a quality is from 1 to 100, not {quality}"
        ))),
    }
}

/// The pages written so far, of `total`.
#[derive(Clone, Copy, Debug, Serialize, Type)]
pub struct ExportProgress {
    pub done: u32,
    pub total: u32,
}

/// Whether `stop_export` asked the export going on to stop.
static STOPPING: AtomicBool = AtomicBool::new(false);

/// Stops the export going on before its next page; a CBZ or PDF it leaves
/// unfinished is deleted.
#[tauri::command]
#[specta::specta]
pub fn stop_export() {
    STOPPING.store(true, Ordering::Relaxed);
}

/// Picks where the open project's pages go, and writes them there: `pages`
/// alone, or every page when it's `None`, in the project's order.
/// `on_progress` hears each page written.
#[tauri::command]
#[specta::specta]
pub async fn export(
    app: State<'_, Arc<App>>,
    window: Window<CefRuntime>,
    pages: Option<Vec<PageId>>,
    format: ExportFormat,
    on_progress: Channel<ExportProgress>,
) -> Result<()> {
    let name = app
        .project
        .lock()
        .as_ref()
        .map(|project| project.name.clone())
        .context("no project is open")?;
    let dialog = rfd::AsyncFileDialog::new().set_parent(&window);
    let extension = match format {
        ExportFormat::Images(_) | ExportFormat::Psd(_) => None,
        ExportFormat::Cbz(_) => Some("cbz"),
        ExportFormat::Pdf => Some("pdf"),
    };
    let destination = match extension {
        None => dialog.pick_folder().await,
        Some(extension) => {
            dialog
                .set_file_name(format!("{name}.{extension}"))
                .add_filter(extension.to_uppercase(), &[extension])
                .save_file()
                .await
        }
    };
    let Some(destination) = destination.map(|picked| picked.path().to_owned()) else {
        return Ok(());
    };
    let app = Arc::clone(&app);
    STOPPING.store(false, Ordering::Relaxed);
    Ok(tokio::task::spawn_blocking(move || {
        let progress = |done, total| {
            let _ = on_progress.send(ExportProgress { done, total });
        };
        let written = app.session().and_then(|session| {
            write(
                &session,
                &app.renderer,
                pages.as_deref(),
                &format,
                &destination,
                progress,
            )
        });
        // A CBZ or PDF left unfinished, stopped or failed, is no file anyone
        // could read; a folder export's files stay as written.
        if extension.is_some() && !matches!(written, Ok(true)) {
            let _ = fs_err::remove_file(&destination);
        }
        written.map(|_| ())
    })
    .await??)
}

/// Writes `pages`, or every page when it's `None`, telling `progress` each
/// page written; `false` when stopped first.
pub(crate) fn write(
    session: &Mutex<Session>,
    renderer: &Renderer,
    pages: Option<&[PageId]>,
    format: &ExportFormat,
    destination: &Path,
    progress: impl Fn(u32, u32),
) -> anyhow::Result<bool> {
    let order: Vec<PageId> = session
        .lock()
        .project()
        .pages()
        .map(|page| page.id)
        .collect();
    let digits = order.len().to_string().len().max(3);
    // A page keeps its number in the project whichever pages go with it, so
    // one exported again replaces its own file.
    let pages: Vec<(String, PageId)> = order
        .iter()
        .enumerate()
        .filter(|(_, page)| pages.is_none_or(|pages| pages.contains(page)))
        .map(|(index, &page)| (format!("{:0digits$}", index + 1), page))
        .collect();
    let draw = |page| -> anyhow::Result<RgbaImage> {
        let frame = renderer.render(&session.lock(), page)?;
        Ok(frame.rasterize()?.image)
    };
    let mut archive = match format {
        ExportFormat::Cbz(_) => Some(::zip::ZipWriter::new(File::create(destination)?)),
        ExportFormat::Images(_) | ExportFormat::Psd(_) | ExportFormat::Pdf => None,
    };
    let mut document = matches!(format, ExportFormat::Pdf).then(Document::new);
    let total = pages.len() as u32;
    progress(0, total);
    for (done, (number, page)) in pages.into_iter().enumerate() {
        if STOPPING.load(Ordering::Relaxed) {
            return Ok(false);
        }
        match format {
            ExportFormat::Images(format) => fs_err::write(
                destination.join(format!("{number}.{}", format.extension())),
                format.encode(draw(page)?)?,
            )?,
            ExportFormat::Psd(labels) => {
                let psd = koharu_psd::export(&session.lock(), renderer, page, labels)?;
                fs_err::write(destination.join(format!("{number}.psd")), psd)?;
            }
            ExportFormat::Cbz(format) => {
                let archive = archive.as_mut().expect("a CBZ export has its archive");
                // Already-compressed images gain nothing from deflate.
                let options = ::zip::write::SimpleFileOptions::default()
                    .compression_method(::zip::CompressionMethod::Stored);
                archive.start_file(format!("{number}.{}", format.extension()), options)?;
                archive.write_all(&format.encode(draw(page)?)?)?;
            }
            ExportFormat::Pdf => {
                let image = draw(page)?;
                let (width, height) = image.dimensions();
                // Import's own scale, so a page read back keeps its pixels.
                let size = Size::from_wh(width as f32 / PDF_SCALE, height as f32 / PDF_SCALE)
                    .context("a page has no pixels")?;
                document
                    .as_mut()
                    .expect("a PDF export has its document")
                    .start_page_with(PageSettings::new(size))
                    .surface()
                    .draw_image(Image::from_rgba8(image.into_raw(), width, height), size);
            }
        }
        progress(done as u32 + 1, total);
    }
    if let Some(archive) = archive {
        archive.finish()?;
    }
    if let Some(document) = document {
        fs_err::write(destination, document.finish()?)?;
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_quality_out_of_range_fails_to_parse() {
        let parse = |section| serde_json::from_value::<ExportConfig>(section);
        assert_eq!(
            parse(serde_json::json!({})).expect("the defaults"),
            ExportConfig::default()
        );
        assert!(parse(serde_json::json!({"jpeg_quality": 100})).is_ok());
        assert!(parse(serde_json::json!({"jpeg_quality": 0})).is_err());
        assert!(parse(serde_json::json!({"webp_quality": 101})).is_err());
    }

    #[test]
    fn each_image_format_reads_back_and_jpeg_is_on_white() -> anyhow::Result<()> {
        let mut image = RgbaImage::from_pixel(8, 8, Rgba([0, 0, 0, 0]));
        image.put_pixel(0, 0, Rgba([255, 0, 0, 255]));
        for (format, read_as) in [
            (ImageFormat::Png, image::ImageFormat::Png),
            (ImageFormat::Jpeg { quality: 90 }, image::ImageFormat::Jpeg),
            (ImageFormat::Webp { quality: 85 }, image::ImageFormat::WebP),
        ] {
            let bytes = format.encode(image.clone())?;
            assert_eq!(image::guess_format(&bytes)?, read_as);
            let read = image::load_from_memory(&bytes)?.to_rgba8();
            assert_eq!(read.dimensions(), (8, 8));
            if matches!(format, ImageFormat::Jpeg { .. }) {
                assert!(read.get_pixel(7, 7).0[..3].iter().all(|&channel| channel > 240));
            }
        }
        Ok(())
    }
}
