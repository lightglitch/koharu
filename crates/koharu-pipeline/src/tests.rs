use super::*;

#[test]
fn configuration_ignores_unknown_fields() {
    let config = toml::from_str::<PipelineConfig>("legacy_limit = 1").unwrap();
    assert_eq!(config, PipelineConfig::default());
}

#[tokio::test]
async fn stop_is_a_successful_partial_result() {
    let pipeline = pipeline(Default::default());
    let stop = StopToken::default();
    stop.stop();
    let request = Request {
        stop,
        ..Request::default()
    };
    let mut committer = RejectCommitter;

    let report = pipeline
        .execute(
            koharu_scene::Session::memory().await.unwrap().snapshot(),
            request,
            &mut committer,
        )
        .await
        .unwrap();

    assert_eq!(report.status, RunStatus::Stopped);
    assert_eq!(report.completed, 0);
}

#[tokio::test]
async fn stop_after_a_page_keeps_completed_progress() {
    let pipeline = pipeline(skipping_translation());
    let mut session = koharu_scene::Session::memory().await.unwrap();
    let patch = session
        .snapshot()
        .patch(|edit| {
            edit.add_page(
                koharu_scene::PageDraft::new("one", 1.0, 1.0),
                koharu_scene::At::End,
            )?;
            edit.add_page(
                koharu_scene::PageDraft::new("two", 1.0, 1.0),
                koharu_scene::At::End,
            )?;
            Ok(())
        })
        .unwrap();
    session.commit(patch).await.unwrap();
    let stop = StopToken::default();
    let progress_stop = stop.clone();
    let request = Request {
        operation: Operation::Only {
            stage: Stage::Translation,
        },
        stop,
        progress: Some(std::sync::Arc::new(move |event| {
            if matches!(event, Progress::Skipped { .. }) {
                progress_stop.stop();
            }
        })),
        ..Request::default()
    };
    let mut committer = RejectCommitter;

    let report = pipeline
        .execute(session.snapshot(), request, &mut committer)
        .await
        .unwrap();

    assert_eq!(report.status, RunStatus::Stopped);
    assert_eq!(report.completed, 1);
    assert_eq!(report.total, 2);
}

#[tokio::test]
async fn a_failed_page_does_not_stop_the_run() {
    let pipeline = pipeline(Default::default());
    pipeline.fail_stage(Stage::Translation);
    let mut session = koharu_scene::Session::memory().await.unwrap();
    let patch = session
        .snapshot()
        .patch(|edit| {
            for label in ["one", "two", "three"] {
                edit.add_page(
                    koharu_scene::PageDraft::new(label, 1.0, 1.0),
                    koharu_scene::At::End,
                )?;
            }
            Ok(())
        })
        .unwrap();
    session.commit(patch).await.unwrap();
    let pages = session
        .snapshot()
        .pages()
        .map(|page| page.id())
        .collect::<Vec<_>>();

    let failed = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let observed = failed.clone();
    let request = Request {
        operation: Operation::Only {
            stage: Stage::Translation,
        },
        progress: Some(std::sync::Arc::new(move |event| {
            if let Progress::Failed { page, .. } = event {
                observed.lock().unwrap().push(page);
            }
        })),
        ..Request::default()
    };
    let mut committer = RejectCommitter;

    let report = pipeline
        .execute(session.snapshot(), request, &mut committer)
        .await
        .unwrap();

    // Every page was attempted rather than the first failure ending the run.
    assert_eq!(report.status, RunStatus::CompletedWithFailures);
    assert_eq!(report.failures.len(), pages.len());
    assert_eq!(
        report.failures.iter().map(|f| f.page).collect::<Vec<_>>(),
        pages,
    );
    assert_eq!(report.failures[0].stage, Some(Stage::Translation));
    assert!(report.failures[0].message.contains("on purpose"));

    // Each failure is announced once, in page order.
    assert_eq!(*failed.lock().unwrap(), pages);

    // Nothing succeeded, but the accounting still balances against total.
    assert_eq!(report.completed, 0);
    assert_eq!(report.total, pages.len());
}

#[tokio::test]
async fn a_run_without_failures_still_reports_completed() {
    // Control for the test above: nothing is forced to fail, so both pages
    // skip cleanly and the status stays Completed.
    let pipeline = pipeline(skipping_translation());
    let mut session = koharu_scene::Session::memory().await.unwrap();
    let patch = session
        .snapshot()
        .patch(|edit| {
            for label in ["one", "two"] {
                edit.add_page(
                    koharu_scene::PageDraft::new(label, 1.0, 1.0),
                    koharu_scene::At::End,
                )?;
            }
            Ok(())
        })
        .unwrap();
    session.commit(patch).await.unwrap();

    let request = Request {
        operation: Operation::Only {
            stage: Stage::Translation,
        },
        ..Request::default()
    };
    let mut committer = RejectCommitter;

    let report = pipeline
        .execute(session.snapshot(), request, &mut committer)
        .await
        .unwrap();

    assert_eq!(report.status, RunStatus::Completed);
    assert!(report.failures.is_empty());
    assert_eq!(report.completed, 2);
}

/// A remote provider that no test supplies credentials for. Stages using it
/// skip on an empty page rather than trying to load a local model.
fn skipping_translation() -> TranslationConfig {
    TranslationConfig {
        model: koharu_translator::ModelSelection {
            provider: koharu_translator::Provider::OpenAi,
            model: Some("gpt-5.6-luna".to_owned()),
            quantization: None,
            vision: true,
            reasoning: true,
        },
        ..Default::default()
    }
}

struct RejectCommitter;

fn pipeline(translation: TranslationConfig) -> Pipeline {
    let config = PipelineConfig {
        translation,
        ..PipelineConfig::default()
    };
    Pipeline::from_config(
        koharu_config::Config::memory(config),
        koharu_config::Config::memory(koharu_translator::ProvidersConfig::default()),
        koharu_ml::Device::cpu(),
    )
    .unwrap()
}

#[async_trait::async_trait]
impl Committer for RejectCommitter {
    async fn commit(&mut self, _output: StageOutput) -> anyhow::Result<koharu_scene::Snapshot> {
        anyhow::bail!("stopped execution must not commit")
    }
}

#[test]
fn operations_expand_to_the_supported_workflows() {
    assert_eq!(
        Operation::Through {
            stage: Stage::Translation,
        }
        .stages()
        .unwrap(),
        vec![Stage::Detection, Stage::Ocr, Stage::Translation],
    );
    assert_eq!(
        Operation::Through {
            stage: Stage::Inpainting,
        }
        .stages()
        .unwrap(),
        vec![Stage::Detection, Stage::Inpainting],
    );
    assert_eq!(
        Operation::Only {
            stage: Stage::Translation,
        }
        .stages()
        .unwrap(),
        vec![Stage::Translation],
    );
    assert_eq!(
        Operation::Stages {
            stages: vec![Stage::Translation, Stage::Detection, Stage::Translation],
        }
        .stages()
        .unwrap(),
        vec![Stage::Detection, Stage::Translation],
    );
}
