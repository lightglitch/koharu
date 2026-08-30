use std::collections::{BTreeMap, BTreeSet};

use koharu_scene::EntityId;

use crate::Stage;

#[derive(Clone, Copy, Eq, PartialEq)]
enum WorkState {
    Pending,
    Running,
    Finished,
    /// Abandoned because the page failed. Terminal, but never satisfies a
    /// dependent stage's prerequisite.
    Failed,
}

impl WorkState {
    const fn terminal(self) -> bool {
        matches!(self, Self::Finished | Self::Failed)
    }
}

struct StageWork {
    stage: Stage,
    state: WorkState,
}

struct PageWork {
    page: EntityId,
    stages: Vec<StageWork>,
}

impl PageWork {
    fn started(&self) -> bool {
        self.stages
            .iter()
            .any(|work| work.state != WorkState::Pending)
    }

    /// Every stage has reached a terminal state, whether it ran or was
    /// abandoned. This is what releases the page's window slot.
    fn settled(&self) -> bool {
        self.stages.iter().all(|work| work.state.terminal())
    }

    fn ready(&self, index: usize) -> bool {
        let Some(prerequisite) = prerequisite(self.stages[index].stage) else {
            return true;
        };
        self.stages
            .iter()
            .find(|work| work.stage == prerequisite)
            .is_none_or(|work| work.state == WorkState::Finished)
    }
}

pub(crate) struct Scheduler {
    pages: Vec<PageWork>,
    page_index: BTreeMap<EntityId, usize>,
    page_window: usize,
    active_pages: usize,
    head: usize,
    total: usize,
}

impl Scheduler {
    pub(crate) fn new(pages: &[EntityId], stages: &[Stage]) -> Self {
        let pages = pages
            .iter()
            .map(|page| PageWork {
                page: *page,
                stages: stages
                    .iter()
                    .map(|stage| StageWork {
                        stage: *stage,
                        state: WorkState::Pending,
                    })
                    .collect(),
            })
            .collect::<Vec<_>>();
        let total = pages.len().saturating_mul(stages.len());
        Self {
            page_index: pages
                .iter()
                .enumerate()
                .map(|(index, page)| (page.page, index))
                .collect(),
            pages,
            page_window: stages.len().max(1),
            active_pages: 0,
            head: 0,
            total,
        }
    }

    pub(crate) fn total(&self) -> usize {
        self.total
    }

    pub(crate) fn start_next(
        &mut self,
        busy_stages: &BTreeSet<Stage>,
    ) -> Option<(EntityId, Stage)> {
        for page_index in self.head..self.pages.len() {
            let started = self.pages[page_index].started();
            if !started && self.active_pages >= self.page_window {
                break;
            }
            let stage_index =
                self.pages[page_index]
                    .stages
                    .iter()
                    .enumerate()
                    .find_map(|(index, work)| {
                        (work.state == WorkState::Pending
                            && !busy_stages.contains(&work.stage)
                            && self.pages[page_index].ready(index))
                        .then_some(index)
                    });
            let Some(stage_index) = stage_index else {
                continue;
            };
            if !started {
                self.active_pages += 1;
            }
            let page = &mut self.pages[page_index];
            let work = &mut page.stages[stage_index];
            work.state = WorkState::Running;
            return Some((page.page, work.stage));
        }
        None
    }

    pub(crate) fn complete_stage(&mut self, page: EntityId, stage: Stage) -> bool {
        let Some(&page_index) = self.page_index.get(&page) else {
            return false;
        };
        let page = &mut self.pages[page_index];
        let was_settled = page.settled();
        if let Some(work) = page.stages.iter_mut().find(|work| work.stage == stage) {
            work.state = WorkState::Finished;
        }
        let settled = !was_settled && page.settled();
        if settled {
            self.release(page_index);
        }
        settled
    }

    /// Abandons every stage of a page that has not already finished.
    ///
    /// Returns how many work items were abandoned so the caller can keep its
    /// completed-versus-total accounting balanced.
    pub(crate) fn fail_page(&mut self, page: EntityId) -> usize {
        let Some(&page_index) = self.page_index.get(&page) else {
            return 0;
        };
        let page = &mut self.pages[page_index];
        let was_settled = page.settled();
        let mut abandoned = 0;
        for work in &mut page.stages {
            if !work.state.terminal() {
                work.state = WorkState::Failed;
                abandoned += 1;
            }
        }
        if !was_settled && page.settled() {
            self.release(page_index);
        }
        abandoned
    }

    /// Gives back the page's window slot and advances past any settled pages.
    fn release(&mut self, _page_index: usize) {
        self.active_pages = self.active_pages.saturating_sub(1);
        while self.head < self.pages.len() && self.pages[self.head].settled() {
            self.head += 1;
        }
    }
}

const fn prerequisite(stage: Stage) -> Option<Stage> {
    match stage {
        Stage::Detection => None,
        Stage::Ocr | Stage::Inpainting => Some(Stage::Detection),
        Stage::Translation => Some(Stage::Ocr),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pages(count: usize) -> Vec<EntityId> {
        (0..count).map(|_| EntityId::new()).collect()
    }

    #[test]
    fn failing_a_page_abandons_its_remaining_work_and_frees_the_window() {
        let pages = pages(2);
        let mut scheduler = Scheduler::new(&pages, &Stage::ALL);
        let busy = BTreeSet::new();

        // Take the first page's detection, then fail the page outright.
        let first = scheduler.start_next(&busy).unwrap();
        assert_eq!(first, (pages[0], Stage::Detection));

        // The running stage plus its three pending siblings are abandoned.
        assert_eq!(scheduler.fail_page(pages[0]), Stage::ALL.len());
        // Failing again is a no-op, so a late sibling completion cannot
        // double-count.
        assert_eq!(scheduler.fail_page(pages[0]), 0);

        // The window slot came back, so the next page still runs.
        let next = scheduler.start_next(&busy).unwrap();
        assert_eq!(next, (pages[1], Stage::Detection));
    }

    #[test]
    fn a_failed_stage_never_satisfies_a_dependent_stage() {
        let pages = pages(1);
        let mut scheduler = Scheduler::new(&pages, &Stage::ALL);
        let busy = BTreeSet::new();

        assert_eq!(scheduler.start_next(&busy).unwrap().1, Stage::Detection);
        scheduler.fail_page(pages[0]);

        // OCR depends on detection; an abandoned detection must not unblock it.
        assert!(scheduler.start_next(&busy).is_none());
    }

    #[test]
    fn starts_pages_in_order_and_models_independently() {
        let pages = pages(2);
        let mut scheduler = Scheduler::new(&pages, &Stage::ALL);
        let mut busy = BTreeSet::new();

        let first = scheduler.start_next(&busy).unwrap();
        assert_eq!(first, (pages[0], Stage::Detection));
        busy.insert(Stage::Detection);
        assert!(scheduler.start_next(&busy).is_none());

        busy.clear();
        assert!(!scheduler.complete_stage(pages[0], Stage::Detection));
        let ocr = scheduler.start_next(&busy).unwrap();
        busy.insert(ocr.1);
        let inpainting = scheduler.start_next(&busy).unwrap();
        busy.insert(inpainting.1);
        let next_page = scheduler.start_next(&busy).unwrap();
        busy.insert(next_page.1);
        assert_eq!(ocr, (pages[0], Stage::Ocr));
        assert_eq!(inpainting, (pages[0], Stage::Inpainting));
        assert_eq!(next_page, (pages[1], Stage::Detection));

        assert!(!scheduler.complete_stage(pages[0], Stage::Ocr));
        busy.remove(&Stage::Ocr);
        let translation = scheduler.start_next(&busy).unwrap();
        assert_eq!(translation, (pages[0], Stage::Translation));
        assert!(busy.contains(&Stage::Detection));
        assert!(busy.contains(&Stage::Inpainting));
    }

    #[test]
    fn sliding_window_backpressures_fast_upstream_models() {
        let pages = pages(4);
        let stages = [Stage::Detection, Stage::Ocr, Stage::Inpainting];
        let mut scheduler = Scheduler::new(&pages, &stages);
        let mut busy = BTreeSet::new();

        assert_eq!(
            scheduler.start_next(&busy),
            Some((pages[0], Stage::Detection))
        );
        assert!(!scheduler.complete_stage(pages[0], Stage::Detection));
        let ocr = scheduler.start_next(&busy).unwrap();
        busy.insert(ocr.1);
        let inpainting = scheduler.start_next(&busy).unwrap();
        busy.insert(inpainting.1);

        for page in &pages[1..3] {
            assert_eq!(scheduler.start_next(&busy), Some((*page, Stage::Detection)));
            assert!(!scheduler.complete_stage(*page, Stage::Detection));
        }
        assert!(scheduler.start_next(&busy).is_none());

        assert!(!scheduler.complete_stage(pages[0], Stage::Ocr));
        assert!(scheduler.complete_stage(pages[0], Stage::Inpainting));
        busy.clear();
        assert_eq!(scheduler.start_next(&busy), Some((pages[1], Stage::Ocr)));
    }
}
