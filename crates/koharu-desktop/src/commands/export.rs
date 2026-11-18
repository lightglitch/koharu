//! Export: every page as PNG or PSD files in a folder, or as a CBZ of PNG,
//! JPEG, or WebP pages, telling its progress as it goes until stopped.

use std::{
    io::{Cursor, Write as _},
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use anyhow::Context as _;
use fs_err::File;
use image::{ImageFormat, RgbImage, RgbaImage, codecs::jpeg::JpegEncoder};
use koharu_psd::Labels;
use koharu_renderer::Frame;
use koharu_scene::PageId;
use serde::{Deserialize, Deserializer, Serialize, de::Error as _};
use specta::Type;
use tauri::{State, Window, ipc::Channel};
use tauri_runtime_cef::CefRuntime;

use super::Result;
use crate::app::App;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum ExportFormat {
    Png,
    Psd,
    Cbz,
}

// Fork addition: upstream writes a CBZ's pages as PNG only, and an export
// can't be followed or stopped.

/// How a CBZ's pages are encoded. JPEG has no alpha, so its pages are
/// flattened onto white.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum PageEncoding {
    Png,
    Jpeg,
    Webp,
}

impl PageEncoding {
    fn extension(self) -> &'static str {
        match self {
            Self::Png => "png",
            Self::Jpeg => "jpg",
            Self::Webp => "webp",
        }
    }
}

/// `[export]`: the lossy encodings' quality, from 1 to 100. A quality out of
/// that range fails to parse.
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

/// Stops the export going on before its next page; a CBZ it leaves
/// unfinished is deleted.
#[tauri::command]
#[specta::specta]
pub fn stop_export() {
    STOPPING.store(true, Ordering::Relaxed);
}

/// Picks where the open project's pages go, and writes them there. `labels`
/// names a PSD's layers the scene names nothing, in the interface's language;
/// `encoding` is a CBZ's pages'. `on_progress` hears each page written.
#[tauri::command]
#[specta::specta]
pub async fn export(
    app: State<'_, Arc<App>>,
    window: Window<CefRuntime>,
    format: ExportFormat,
    labels: Labels,
    encoding: PageEncoding,
    on_progress: Channel<ExportProgress>,
) -> Result<()> {
    let name = app
        .project
        .lock()
        .as_ref()
        .map(|project| project.name.clone())
        .context("no project is open")?;
    let dialog = rfd::AsyncFileDialog::new().set_parent(&window);
    let destination = match format {
        ExportFormat::Png | ExportFormat::Psd => dialog.pick_folder().await,
        ExportFormat::Cbz => {
            dialog
                .set_file_name(format!("{name}.cbz"))
                .add_filter("CBZ", &["cbz"])
                .save_file()
                .await
        }
    };
    let Some(destination) = destination.map(|picked| picked.path().to_owned()) else {
        return Ok(());
    };
    let config: ExportConfig = koharu_config::get()?;
    let app = Arc::clone(&app);
    STOPPING.store(false, Ordering::Relaxed);
    Ok(tokio::task::spawn_blocking(move || {
        let progress = |done, total| {
            let _ = on_progress.send(ExportProgress { done, total });
        };
        let written = write(
            &app,
            format,
            &labels,
            encoding,
            &config,
            &destination,
            progress,
        );
        // A CBZ left unfinished, stopped or failed, is no archive anyone
        // could read; a folder export's files stay as written.
        if format == ExportFormat::Cbz && !matches!(written, Ok(true)) {
            let _ = fs_err::remove_file(&destination);
        }
        written.map(|_| ())
    })
    .await??)
}

/// Writes every page; `false` when stopped first.
fn write(
    app: &App,
    format: ExportFormat,
    labels: &Labels,
    encoding: PageEncoding,
    config: &ExportConfig,
    destination: &Path,
    progress: impl Fn(u32, u32),
) -> anyhow::Result<bool> {
    let session = app.session()?;
    let pages: Vec<PageId> = session
        .lock()
        .project()
        .pages()
        .map(|page| page.id)
        .collect();
    let digits = pages.len().to_string().len().max(3);
    let mut archive = match format {
        ExportFormat::Cbz => Some(::zip::ZipWriter::new(File::create(destination)?)),
        ExportFormat::Png | ExportFormat::Psd => None,
    };
    let total = pages.len() as u32;
    progress(0, total);
    for (index, &page) in pages.iter().enumerate() {
        if STOPPING.load(Ordering::Relaxed) {
            return Ok(false);
        }
        let number = format!("{:0digits$}", index + 1);
        let frame = app.renderer.render(&session.lock(), page)?;
        match (format, &mut archive) {
            (ExportFormat::Psd, _) => {
                let psd = koharu_psd::export(&session.lock(), &app.renderer, page, labels)?;
                fs_err::write(destination.join(format!("{number}.psd")), psd)?;
            }
            (_, Some(archive)) => {
                // Already-compressed pages gain nothing from deflate.
                let options = ::zip::write::SimpleFileOptions::default()
                    .compression_method(::zip::CompressionMethod::Stored);
                archive.start_file(format!("{number}.{}", encoding.extension()), options)?;
                archive.write_all(&encoded(&frame.rasterize()?.image, encoding, config)?)?;
            }
            (_, None) => {
                fs_err::write(destination.join(format!("{number}.png")), png(&frame)?)?;
            }
        }
        progress(index as u32 + 1, total);
    }
    if let Some(archive) = archive {
        archive.finish()?;
    }
    Ok(true)
}

fn png(frame: &Frame) -> anyhow::Result<Vec<u8>> {
    let raster = frame.rasterize()?;
    let mut png = Cursor::new(Vec::new());
    raster.image.write_to(&mut png, ImageFormat::Png)?;
    Ok(png.into_inner())
}

/// `image` as a CBZ page in `encoding`, at `config`'s quality.
fn encoded(
    image: &RgbaImage,
    encoding: PageEncoding,
    config: &ExportConfig,
) -> anyhow::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    match encoding {
        PageEncoding::Png => image.write_to(&mut Cursor::new(&mut bytes), ImageFormat::Png)?,
        PageEncoding::Jpeg => JpegEncoder::new_with_quality(&mut bytes, config.jpeg_quality)
            .encode_image(&on_white(image))?,
        // libwebp, as `image` encodes WebP only losslessly.
        PageEncoding::Webp => {
            bytes = webp::Encoder::from_rgba(image.as_raw(), image.width(), image.height())
                .encode(f32::from(config.webp_quality))
                .to_vec();
        }
    }
    Ok(bytes)
}

/// `image` composed over white, its alpha dropped.
fn on_white(image: &RgbaImage) -> RgbImage {
    RgbImage::from_fn(image.width(), image.height(), |x, y| {
        let [red, green, blue, alpha] = image.get_pixel(x, y).0;
        let alpha = u16::from(alpha);
        let over =
            |channel: u8| ((u16::from(channel) * alpha + 255 * (255 - alpha) + 127) / 255) as u8;
        image::Rgb([over(red), over(green), over(blue)])
    })
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
    fn each_encoding_reads_back_and_jpeg_is_on_white() -> anyhow::Result<()> {
        let mut image = RgbaImage::from_pixel(8, 8, image::Rgba([0, 0, 0, 0]));
        image.put_pixel(0, 0, image::Rgba([255, 0, 0, 255]));
        let config = ExportConfig::default();
        for (encoding, format) in [
            (PageEncoding::Png, ImageFormat::Png),
            (PageEncoding::Jpeg, ImageFormat::Jpeg),
            (PageEncoding::Webp, ImageFormat::WebP),
        ] {
            let bytes = encoded(&image, encoding, &config)?;
            assert_eq!(image::guess_format(&bytes)?, format);
            let read = image::load_from_memory(&bytes)?.to_rgba8();
            assert_eq!(read.dimensions(), (8, 8));
            if encoding == PageEncoding::Jpeg {
                assert!(
                    read.get_pixel(7, 7).0[..3]
                        .iter()
                        .all(|&channel| channel > 240)
                );
            }
        }
        Ok(())
    }
}
