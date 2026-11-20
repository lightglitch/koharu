//! Fork addition: `koharu --input x.cbz --output y.cbz` translates a book
//! without the window. It reads the input as the import does, runs the
//! pipeline on every page with the saved settings, and writes a CBZ as the
//! export does, in a project in the temporary folder that goes once done.
//! `koharu --project name` runs the stages on one of the app's projects
//! instead, which keeps what they did, as the app does, and writes a CBZ
//! when given `--output`; with `--input`, it makes that project of the book
//! first, which mustn't exist yet.

use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
    sync::Arc,
};

use anyhow::{Context as _, Result};
use koharu_pipeline::{Event, Outcome, Pipeline, PipelineConfig, Request, Stage, Target};
use koharu_renderer::{Renderer, RendererConfig};
use koharu_scene::Session;
use parking_lot::Mutex;
use tokio::sync::broadcast::error::RecvError;

use crate::{
    commands::{
        export::{ExportConfig, ExportFormat, PageEncoding, write},
        import::pages,
    },
    library::Library,
};

#[derive(clap::Args, Debug)]
pub struct Args {
    /// Translate this book (an image, ZIP, CBZ, RAR, CBR, or PDF) without
    /// opening the window; with --project, into that new project.
    #[arg(long)]
    pub input: Option<PathBuf>,
    /// Run the stages on this project of the app's, by name, keeping what
    /// they do, as validating a book translated in the app does; with
    /// --input, a new project made of the book.
    #[arg(long)]
    pub project: Option<String>,
    /// The CBZ to write: required with --input, optional with --project.
    #[arg(long)]
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

/// Runs `args.stages` on `args.input` into `args.output`, or on the project
/// `args.project` names among the app's, `identifier`'s; `false` when a page
/// failed, whose stages left it as they found it, and the CBZ is written
/// anyway.
pub async fn run(args: Args, identifier: &str) -> Result<bool> {
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
    let output = args.output.as_deref().map(|path| (path, encoding, &export));

    if let Some(name) = args.project {
        let library = Library::new(crate::projects(identifier)?)?;
        let session = match &args.input {
            // A book's pages go in once, into a project of their own.
            Some(input) => {
                anyhow::ensure!(
                    !library.list()?.contains(&name),
                    "a project named {name:?} already exists; drop --input to run on it"
                );
                let mut session = library.create(&name)?;
                session.apply(pages(std::slice::from_ref(input), 0)?)?;
                eprintln!("made the project {name:?} of {}", input.display());
                session
            }
            None => library.open(&name)?,
        };
        let session = Arc::new(Mutex::new(session));
        let result = process(&pipeline, &renderer, &session, stages, output).await;
        pipeline.close(session.lock().id());
        return result;
    }
    let Some(input) = args.input else {
        anyhow::bail!("give --project, or --input with --output");
    };
    anyhow::ensure!(
        output.is_some(),
        "--input without --project needs --output, or its work is lost"
    );
    let path = std::env::temp_dir().join(format!(
        "koharu-convert-{}.{}",
        std::process::id(),
        koharu_scene::EXTENSION
    ));
    let session = Arc::new(Mutex::new(Session::create(&path).with_context(|| {
        format!("failed to create a project in {}", path.display())
    })?));
    let result = match pages(&[input], 0).and_then(|edit| Ok(session.lock().apply(edit)?)) {
        Ok(_) => process(&pipeline, &renderer, &session, stages, output).await,
        Err(error) => Err(error),
    };
    pipeline.close(session.lock().id());
    drop(session);
    if let Err(error) = Session::remove(&path) {
        tracing::warn!("couldn't delete {}: {error:#}", path.display());
    }
    result
}

/// Runs `stages` on every page `session` holds, then writes it to `output`
/// as a CBZ of pages in its encoding, when there is one.
async fn process(
    pipeline: &Pipeline,
    renderer: &Renderer,
    session: &Arc<Mutex<Session>>,
    stages: BTreeSet<Stage>,
    output: Option<(&Path, PageEncoding, &ExportConfig)>,
) -> Result<bool> {
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
    let Some((output, encoding, export)) = output else {
        return Ok(matches!(outcome, Outcome::Done));
    };

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
