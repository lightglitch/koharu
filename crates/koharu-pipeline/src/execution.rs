use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
    time::Instant,
};

use anyhow::{Context as _, Result, ensure};
use futures::{StreamExt as _, stream::FuturesUnordered};
use koharu_scene::{EntityId, Snapshot};

use crate::{
    Committer, ErrorKind, PageFailure, PipelineError, Progress, ProgressSink, Report, Request,
    RunStatus, Stage, StageOutput, StopToken,
    images::ImageCache,
    progress,
    resources::ResourceMonitor,
    scheduler::Scheduler,
    scope::NormalizedScope,
    stage_runner::{StageCompletion, StageJob, StageOutcome, StageRunner},
    stages::StageInput,
};

pub(crate) struct Execution<'a> {
    runner: Arc<StageRunner>,
    resources: Arc<ResourceMonitor>,
    committer: &'a mut dyn Committer,
    stop: StopToken,
    progress: Option<ProgressSink>,
    scope: NormalizedScope,
    scheduler: Scheduler,
    scene: Snapshot,
    images: BTreeMap<EntityId, Arc<ImageCache>>,
    busy_stages: BTreeSet<Stage>,
    completed: usize,
    /// Work items abandoned because their page failed. Counted separately so
    /// the completed-versus-total invariant still balances.
    abandoned: usize,
    failures: Vec<PageFailure>,
    failed_pages: BTreeSet<EntityId>,
    base: koharu_scene::Revision,
    started: Instant,
    inpainting_mask: Option<crate::InpaintingMask>,
}

impl<'a> Execution<'a> {
    pub(crate) fn new(
        runner: Arc<StageRunner>,
        resources: Arc<ResourceMonitor>,
        snapshot: Snapshot,
        request: Request,
        committer: &'a mut dyn Committer,
    ) -> std::result::Result<Self, PipelineError> {
        let started = Instant::now();
        let base = snapshot.revision();
        let stages = request
            .operation
            .stages()
            .map_err(|error| PipelineError::new(ErrorKind::InvalidInput, None, error))?;
        let scope = NormalizedScope::new(&snapshot, &request.scope, &stages)
            .map_err(|error| PipelineError::new(ErrorKind::InvalidInput, None, error))?;
        let pages = scope.pages().to_vec();
        if let Some(mask) = request.inpainting_mask.as_ref()
            && (!pages.contains(&mask.page) || !stages.contains(&Stage::Inpainting))
        {
            return Err(PipelineError::new(
                ErrorKind::InvalidInput,
                Some(Stage::Inpainting),
                anyhow::anyhow!("the inpainting mask page is outside the inpainting scope"),
            ));
        }
        progress::emit(
            request.progress.as_ref(),
            Progress::Started {
                pages: pages.clone(),
                stages: stages.clone(),
            },
        );

        Ok(Self {
            runner,
            resources,
            committer,
            stop: request.stop,
            progress: request.progress,
            scope,
            scheduler: Scheduler::new(&pages, &stages),
            scene: snapshot,
            images: BTreeMap::new(),
            busy_stages: BTreeSet::new(),
            completed: 0,
            abandoned: 0,
            failures: Vec::new(),
            failed_pages: BTreeSet::new(),
            base,
            started,
            inpainting_mask: request.inpainting_mask,
        })
    }

    pub(crate) async fn run(mut self) -> std::result::Result<Report, PipelineError> {
        if self.stopped() {
            return Ok(self.report(RunStatus::Stopped));
        }

        self.resources.start();
        self.resources.wait_for_sample().await;

        let runner = self.runner.clone();
        let mut running = FuturesUnordered::new();
        loop {
            while let Some(job) = self.take_ready_job() {
                running.push(runner.run(job));
            }

            let Some(completion) = running.next().await else {
                break;
            };
            self.busy_stages.remove(&completion.stage);
            if self.stopped() {
                continue;
            }
            // A sibling stage of an already-abandoned page: its slot was
            // counted when the page failed, so drop the result.
            if self.failed_pages.contains(&completion.page) {
                continue;
            }
            let page = completion.page;
            if let Err(error) = self.apply_completion(completion).await {
                self.fail_page(page, error);
            }
        }

        self.finalize()
    }

    fn take_ready_job(&mut self) -> Option<StageJob> {
        if self.stopped() {
            return None;
        }
        let (page, stage) = self.scheduler.start_next(&self.busy_stages)?;
        self.busy_stages.insert(stage);
        let images = self
            .images
            .entry(page)
            .or_insert_with(|| Arc::new(ImageCache::default()))
            .clone();
        Some(StageJob::new(
            stage,
            StageInput::new(
                self.scene.clone(),
                page,
                self.scope.entities(),
                self.scope.region(page),
                images,
                self.inpainting_mask
                    .as_ref()
                    .filter(|mask| stage == Stage::Inpainting && mask.page == page)
                    .cloned(),
            ),
            self.stop.clone(),
            self.progress.clone(),
        ))
    }

    async fn apply_completion(
        &mut self,
        completion: StageCompletion,
    ) -> std::result::Result<(), PipelineError> {
        let StageCompletion {
            page,
            stage,
            model,
            elapsed,
            outcome,
        } = completion;
        match outcome? {
            StageOutcome::Stopped => {}
            StageOutcome::Skipped => {
                self.mark_complete(page, stage);
                progress::emit(self.progress.as_ref(), Progress::Skipped { page, stage });
            }
            StageOutcome::Patch(patch) => {
                if !self.commit_patch(page, stage, patch).await? {
                    return Ok(());
                }
                self.mark_complete(page, stage);
                progress::emit(
                    self.progress.as_ref(),
                    Progress::Finished {
                        page,
                        stage,
                        model,
                        elapsed,
                    },
                );
            }
        }
        Ok(())
    }

    async fn commit_patch(
        &mut self,
        page: EntityId,
        stage: Stage,
        patch: koharu_scene::Patch,
    ) -> std::result::Result<bool, PipelineError> {
        let patch = patch
            .rebase_on(&self.scene)
            .and_then(|patch| {
                patch.validate_on(&self.scene)?;
                Ok(patch.with_label(format!("Pipeline {stage} for page {page}")))
            })
            .context("failed to rebase stage output onto the latest scene")
            .map_err(|error| PipelineError::new(ErrorKind::InvalidOutput, Some(stage), error))?;
        if self.stopped() {
            return Ok(false);
        }

        let next = self
            .committer
            .commit(StageOutput { page, stage, patch })
            .await
            .with_context(|| format!("failed to commit {stage} output for page {page}"))
            .map_err(|error| PipelineError::new(ErrorKind::Commit, Some(stage), error))?;
        validate_commit(&self.scene, &next)
            .map_err(|error| PipelineError::new(ErrorKind::Commit, Some(stage), error))?;
        self.scene = next;
        Ok(true)
    }

    /// Records a page failure, abandons the rest of that page and lets the run
    /// continue. Whole-run problems are not routed here: a committer that
    /// breaks its contract is a defect, not a page that failed to translate.
    fn fail_page(&mut self, page: EntityId, error: PipelineError) {
        let stage = error.stage;
        let message = format!("{error:#}");
        tracing::warn!(%page, ?stage, %message, "abandoning page after a stage failure");
        self.failures.push(PageFailure {
            page,
            stage,
            message: message.clone(),
        });
        self.failed_pages.insert(page);
        self.abandoned += self.scheduler.fail_page(page);
        self.images.remove(&page);
        progress::emit(
            self.progress.as_ref(),
            Progress::Failed {
                page,
                stage,
                message,
            },
        );
    }

    fn mark_complete(&mut self, page: EntityId, stage: Stage) {
        if self.scheduler.complete_stage(page, stage) {
            self.images.remove(&page);
        }
        self.completed += 1;
    }

    fn stopped(&self) -> bool {
        self.stop.stopped()
    }

    fn finalize(self) -> std::result::Result<Report, PipelineError> {
        let settled = self.completed + self.abandoned;
        if !self.stopped() && settled != self.scheduler.total() {
            return Err(PipelineError::new(
                ErrorKind::InvalidOutput,
                None,
                anyhow::anyhow!(
                    "pipeline scheduler stopped after {} of {} work items",
                    settled,
                    self.scheduler.total()
                ),
            ));
        }
        let status = if self.stopped() {
            RunStatus::Stopped
        } else if self.failures.is_empty() {
            RunStatus::Completed
        } else {
            RunStatus::CompletedWithFailures
        };
        Ok(self.report(status))
    }

    fn report(&self, status: RunStatus) -> Report {
        Report {
            status,
            failures: self.failures.clone(),
            base: self.base,
            final_revision: self.scene.revision(),
            completed: self.completed,
            total: self.scheduler.total(),
            elapsed: self.started.elapsed(),
        }
    }
}

fn validate_commit(previous: &Snapshot, next: &Snapshot) -> Result<()> {
    ensure!(
        previous.project_id() == next.project_id(),
        "committer returned a snapshot from another project"
    );
    ensure!(
        next.revision() > previous.revision(),
        "committer did not advance the scene revision"
    );
    Ok(())
}
