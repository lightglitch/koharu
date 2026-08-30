use std::time::Duration;

use anyhow::Result;
use async_trait::async_trait;

use crate::Stage;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RunStatus {
    Completed,
    /// Every page was attempted, but at least one failed and was abandoned.
    CompletedWithFailures,
    Stopped,
}

/// A page abandoned mid-run because one of its stages failed.
#[derive(Clone, Debug)]
pub struct PageFailure {
    pub page: koharu_scene::EntityId,
    pub stage: Option<Stage>,
    pub message: String,
}

#[derive(Debug)]
pub struct Report {
    pub status: RunStatus,
    /// Pages abandoned during the run, in the order they failed.
    pub failures: Vec<PageFailure>,
    pub base: koharu_scene::Revision,
    pub final_revision: koharu_scene::Revision,
    pub completed: usize,
    pub total: usize,
    pub elapsed: Duration,
}

#[derive(Debug)]
pub struct StageOutput {
    pub page: koharu_scene::EntityId,
    pub stage: Stage,
    pub patch: koharu_scene::Patch,
}

#[async_trait]
pub trait Committer: Send {
    async fn commit(&mut self, output: StageOutput) -> Result<koharu_scene::Snapshot>;
}
