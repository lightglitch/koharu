'use client'

import { showCanvasPage } from '@koharu/bridge/canvas'
import {
  commands,
  type CanvasPagePreparation,
  type EntityId,
  type Page,
  type ProjectInfo,
} from '@koharu/bridge/protocol'

import { call } from './backend'
import { pageKey, preparedPageKey, projectKey, queryClient } from './queries'
import { useKoharuStore } from './store'

/// Shared across every caller on purpose: the page rail and the keyboard both
/// change the same active page, and a later request must always win.
let request = 0

/// Abandons any activation still in flight, so an unmounting view cannot apply
/// its result over a newer one.
export function cancelPageActivation(): void {
  request += 1
}

/// Makes a page active, optimistically where the canvas already holds it.
///
/// The prepared frame is shown before the backend answers when its revision
/// still matches, which is what keeps paging through a project immediate. The
/// query data is rolled back if the command fails and nothing newer has
/// happened since.
export function activatePage(page: EntityId, selection: EntityId[]): void {
  const previousProject = queryClient.getQueryData<ProjectInfo | null>(projectKey)
  const previousPage = queryClient.getQueryData<Page | null>(pageKey)
  const prepared = queryClient.getQueryData<CanvasPagePreparation>(preparedPageKey(page))
  const activated = showCanvasPage(page, previousProject?.revision ?? null)
  const current = ++request

  const synchronize = () => {
    if (request !== current) return
    if (activated && previousProject && prepared?.revision === previousProject.revision) {
      queryClient.setQueryData(projectKey, { ...previousProject, active_page: page })
      queryClient.setQueryData(pageKey, prepared.page)
    }
    const store = useKoharuStore.getState()
    store.selectPages(selection)
    store.selectLayers([])
    void call(commands.selectPage, page)
      .then((selected) => {
        if (request !== current) return
        queryClient.setQueryData(projectKey, selected.project)
        queryClient.setQueryData(pageKey, selected.page)
      })
      .catch(() => {
        if (request !== current) return
        if (queryClient.getQueryData<ProjectInfo | null>(projectKey)?.active_page === page) {
          queryClient.setQueryData(projectKey, previousProject)
          queryClient.setQueryData(pageKey, previousPage)
        }
      })
  }

  if (activated) {
    requestAnimationFrame(() => window.setTimeout(synchronize, 0))
  } else {
    synchronize()
  }
}

/// The page a step away from the active one, or undefined at either end.
export function pageStep(
  pages: readonly { id: EntityId }[],
  active: EntityId | null | undefined,
  delta: number,
): EntityId | undefined {
  if (pages.length === 0) return undefined
  const index = pages.findIndex((page) => page.id === active)
  // With nothing active, stepping forward starts at the first page and
  // stepping back starts at the last.
  if (index < 0) return delta > 0 ? pages[0]?.id : pages[pages.length - 1]?.id
  return pages[index + delta]?.id
}
