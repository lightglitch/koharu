//! Fork addition: `koharu --input x.cbz --output y.cbz` translates a book
//! without the window. It reads the input as the import does, runs the
//! pipeline on every page with the saved settings, and writes a CBZ as the
//! export does, in a project in the temporary folder that goes once done.

use std::{collections::BTreeSet, path::PathBuf, sync::Arc};

use anyhow::{Context as _, Result};
use koharu_pipeline::{Event, Outcome, Pipeline, PipelineConfig, Request, Stage, Target};
use koharu_renderer::{Renderer, RendererConfig};
use koharu_scene::Session;
use parking_lot::Mutex;
use tokio::sync::broadcast::error::RecvError;

use crate::commands::{
    export::{ExportConfig, ExportFormat, PageEncoding, write},
    import::pages,
};

#[derive(clap::Args, Debug)]
pub struct Args {
    /// Translate this book (an image, ZIP, CBZ, RAR, CBR, or PDF) without
    /// opening the window.
    #[arg(long, requires = "output")]
    pub input: Option<PathBuf>,
    /// The CBZ to write.
    #[arg(long, requires = "input")]
    pub output: Option<PathBuf>,
    /// The CBZ's page format; JPEG and WebP use the quality in Settings.
    #[arg(long, value_enum, default_value = "png")]
    pub format: Format,
    /// The stages to run, comma-separated; all but validation by default.
    #[arg(
        long,
        value_enum,
        value_delimiter = ',',
        default_value = "detect,ocr,inpaint,translate"
    )]
    pub stages: Vec<Step>,
}

#[derive(clap::ValueEnum, Clone, Copy, Debug)]
pub enum Format {
    Png,
    #[value(alias = "jpg")]
    Jpeg,
    Webp,
}

#[derive(clap::ValueEnum, Clone, Copy, Debug)]
pub enum Step {
    Detect,
    #[value(alias = "recognize")]
    Ocr,
    #[value(alias = "clean")]
    Inpaint,
    Translate,
    Validate,
}

/// Translates `args.input` into `args.output`; `false` when a page failed,
/// whose stages left it as they found it, and the CBZ is written anyway.
pub async fn run(args: Args) -> Result<bool> {
    let (Some(input), Some(output)) = (args.input, args.output) else {
        anyhow::bail!("--input and --output go together");
    };
    let encoding = match args.format {
        Format::Png => PageEncoding::Png,
        Format::Jpeg => PageEncoding::Jpeg,
        Format::Webp => PageEncoding::Webp,
    };
    let stages: BTreeSet<Stage> = args
        .stages
        .iter()
        .map(|step| match step {
            Step::Detect => Stage::Detect,
            Step::Ocr => Stage::Recognize,
            Step::Inpaint => Stage::Inpaint,
            Step::Translate => Stage::Translate,
            Step::Validate => Stage::Validate,
        })
        .collect();
    let export: ExportConfig = koharu_config::get()?;
    let renderer = Renderer::get().clone();
    renderer.set_config(koharu_config::get::<RendererConfig>()?);
    let pipeline = Pipeline::new(koharu_config::get::<PipelineConfig>()?);

    let path = std::env::temp_dir().join(format!(
        "koharu-convert-{}.{}",
        std::process::id(),
        koharu_scene::EXTENSION
    ));
    let session = Arc::new(Mutex::new(Session::create(&path).with_context(|| {
        format!("failed to create a project in {}", path.display())
    })?));
    let result = translate(
        &pipeline, &renderer, &session, input, &output, stages, encoding, &export,
    )
    .await;
    pipeline.close(session.lock().id());
    drop(session);
    if let Err(error) = Session::remove(&path) {
        tracing::warn!("couldn't delete {}: {error:#}", path.display());
    }
    result
}

#[allow(clippy::too_many_arguments)]
async fn translate(
    pipeline: &Pipeline,
    renderer: &Renderer,
    session: &Arc<Mutex<Session>>,
    input: PathBuf,
    output: &std::path::Path,
    stages: BTreeSet<Stage>,
    encoding: PageEncoding,
    export: &ExportConfig,
) -> Result<bool> {
    session.lock().apply(pages(&[input], 0)?)?;
    let order: Vec<_> = session
        .lock()
        .project()
        .pages()
        .map(|page| page.id)
        .collect();
    let number = |page| {
        order
            .iter()
            .position(|&id| id == page)
            .map_or(0, |index| index + 1)
    };
    eprintln!("{} pages", order.len());

    // Listening before the run starts, so none of its events is missed.
    let mut events = pipeline.events();
    let run = pipeline.run(
        Arc::clone(session),
        Request {
            target: Target::Project,
            stages,
            config: None,
        },
    )?;
    let (mut done, mut steps) = (0, 0);
    let outcome = loop {
        match events.recv().await {
            Ok(Event::Queued {
                run: id,
                steps: all,
            }) if id == run => steps = all,
            Ok(Event::Finished {
                run: id,
                page,
                stage,
            }) if id == run => {
                done += 1;
                eprintln!("[{done}/{steps}] page {}: {stage}", number(page));
            }
            Ok(Event::Failed {
                run: id,
                page,
                stage,
                reason,
            }) if id == run => eprintln!("page {} failed at {stage}: {reason}", number(page)),
            Ok(Event::Flagged {
                run: id,
                page,
                flags,
            }) if id == run => {
                for flag in flags {
                    eprintln!(
                        "page {} flagged: {:?}, {:.0} px \"{}\"",
                        number(page),
                        flag.kind,
                        flag.size,
                        flag.text
                    );
                }
            }
            Ok(Event::Ended { run: id, outcome }) if id == run => break outcome,
            Ok(_) | Err(RecvError::Lagged(_)) => {}
            Err(RecvError::Closed) => anyhow::bail!("the pipeline stopped"),
        }
    };
    if outcome == Outcome::Stopped {
        anyhow::bail!("the run was stopped");
    }

    eprintln!("writing {}", output.display());
    let session = Arc::clone(session);
    let renderer = renderer.clone();
    let (output, export) = (output.to_owned(), export.clone());
    let written = tokio::task::spawn_blocking(move || {
        // A CBZ names no layers; these are a PSD's.
        let labels = koharu_psd::Labels {
            background: String::new(),
            cleanup: String::new(),
            outline: String::new(),
        };
        let progress = |_, _| {};
        let written = write(
            &session,
            &renderer,
            ExportFormat::Cbz,
            &labels,
            encoding,
            &export,
            &output,
            progress,
        );
        if written.is_err() {
            let _ = fs_err::remove_file(&output);
        }
        written
    })
    .await??;
    anyhow::ensure!(written, "the export was stopped");
    Ok(matches!(outcome, Outcome::Done))
}
