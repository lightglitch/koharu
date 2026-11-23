'use client'

import { create } from 'zustand'
import { persist } from 'zustand/middleware'

import type {
  Bounds,
  LayerId,
  Flag,
  PageFailure,
  Loading,
  PageId,
  PipelineEvent,
  RunId,
  Size,
  Stage,
  Tool,
  Transfer,
  Typed,
} from '@koharu/ipc/protocol'

export type CanvasTool = Tool
export const MIN_BRUSH_DIAMETER = 1
export function maxBrushDiameter(size: Size): number {
  // Assume that for all sizes below 2048 width, 128 is sufficient.
  // For anything above, scale by percentage greater than 2048 width.
  return Math.round(Math.max(size.width / 2048, 1.0) * 128)
}

export interface CanvasBrush {
  diameter: number
  color: string
}

/** Where the page shows: a page point `p` sits at `p * zoom + (x, y)` in CSS pixels from the
 * canvas element's corner, as the native canvas publishes it. */
export interface Camera {
  zoom: number
  x: number
  y: number
}

/** The right pane's tabs. */
export type Panel = 'layers' | 'story' | 'agent'
export type PipelineScope = 'selected-pages' | 'project'
export const pipelineStages: readonly Stage[] = [
  'detect',
  'recognize',
  'inpaint',
  'translate',
  'validate',
]
/** The stages a run takes when nothing says otherwise: validation only runs when it's asked for,
 * since it costs a pass per page and later model calls. */
// Fork addition: upstream has no validation stage.
export const defaultStages: readonly Stage[] = pipelineStages.filter(
  (stage) => stage !== 'validate',
)

/** A pipeline run as the activity center shows it, from the pipeline's events. */
export interface Job {
  id: RunId
  /** `failed` is a run that went through every page but gave up on some of them. */
  state: 'running' | 'done' | 'failed' | 'stopped'
  /** The page stages done, of `total`. */
  completed: number
  total: number
  /** The stage started last, and its page. */
  stage: Stage | null
  page: PageId | null
  /** The pages given up on so far, in the order they failed; the run goes on without them. */
  failures: PageFailure[]
  /** What validation flagged, each page's texts together, in the order they were measured. */
  // Fork addition.
  flagged: PageFlags[]
}

/** The texts validation flagged on a page. */
export interface PageFlags {
  page: PageId
  flags: Flag[]
}

/** The pages the runs still shown gave up on, for marking them where pages are listed. */
export function failedPages(jobs: Record<string, Job>): Set<PageId> {
  return new Set(Object.values(jobs).flatMap((job) => job.failures.map((failure) => failure.page)))
}

interface KoharuStore {
  initialized: boolean
  jobs: Record<string, Job>
  downloads: Record<string, Transfer>
  camera: Camera
  /** Where the canvas's selection lies on the page as it's laid out, a point text's lines
   * included. */
  selectionBounds: Bounds | null
  /** Where the area the canvas's marquee and lasso marked lies on the page. */
  area: Bounds | null
  /** The text the canvas is typing into, and its selected characters' style. */
  typing: Typed | null
  /** The colour the canvas's picker picked last, numbered so a repeat counts. */
  picked: { color: string; serial: number } | null
  selectedLayers: LayerId[]
  /** The selection the canvas has, so one it published isn't sent back to it. */
  canvasSelection: LayerId[]
  selectedPages: PageId[]
  tool: CanvasTool
  brush: CanvasBrush
  /** The right pane's tab, which the canvas numbers the texts for while it's Story. */
  panel: Panel
  /** The text whose Story entry the pointer is over, outlined on the canvas. */
  storyHover: LayerId | null
  processingScope: PipelineScope
  processingStages: Stage[]
  settingsOpen: boolean
  /** Whether the History window shows over the canvas. */
  historyOpen: boolean
  /** Whether the start view shows the gallery, not the projects. */
  galleryOpen: boolean
  /** Whether the main window reads the open project, not edits it. */
  reading: boolean
  /** How far an extension's chapter loading into the open project went. */
  loading: Loading | null
  selectPages: (pages: PageId[]) => void
  showPanel: (panel: Panel) => void
  setStoryHover: (layer: LayerId | null) => void
  setProcessingScope: (scope: PipelineScope) => void
  setProcessingStages: (stages: Stage[]) => void
  setSettingsOpen: (open: boolean) => void
  setHistoryOpen: (open: boolean) => void
  setGalleryOpen: (open: boolean) => void
  setReading: (reading: boolean) => void
  selectLayers: (layers: LayerId[]) => void
  setTool: (tool: CanvasTool) => void
  setBrush: (brush: CanvasBrush) => void
  dismissJob: (id: string) => void
  dismissDownload: (id: number) => void
}

export const useKoharuStore = create<KoharuStore>()(
  persist(
    (set) => ({
      initialized: false,
      jobs: {},
      downloads: {},
      camera: { zoom: 1, x: 0, y: 0 },
      selectionBounds: null,
      area: null,
      typing: null,
      picked: null,
      selectedLayers: [],
      canvasSelection: [],
      selectedPages: [],
      tool: 'select',
      brush: { diameter: 48, color: '#FFFFFF' },
      panel: 'layers',
      storyHover: null,
      processingScope: 'project',
      processingStages: [...defaultStages],
      settingsOpen: false,
      historyOpen: false,
      galleryOpen: false,
      reading: false,
      loading: null,
      selectPages: (selectedPages) => set({ selectedPages: [...new Set(selectedPages)] }),
      showPanel: (panel) => set({ panel, storyHover: null }),
      setStoryHover: (storyHover) => set({ storyHover }),
      setProcessingScope: (processingScope) => set({ processingScope }),
      setProcessingStages: (processingStages) => set({ processingStages }),
      setSettingsOpen: (settingsOpen) => set({ settingsOpen }),
      setHistoryOpen: (historyOpen) => set({ historyOpen }),
      setGalleryOpen: (galleryOpen) => set({ galleryOpen }),
      setReading: (reading) => set({ reading }),
      selectLayers: (selectedLayers) => set({ selectedLayers: [...new Set(selectedLayers)] }),
      setTool: (tool) => set({ tool }),
      setBrush: (brush) => set({ brush }),
      dismissJob: (id) =>
        set((state) => {
          const jobs = { ...state.jobs }
          delete jobs[id]
          return { jobs }
        }),
      dismissDownload: (id) =>
        set((state) => {
          const downloads = { ...state.downloads }
          delete downloads[String(id)]
          return { downloads }
        }),
    }),
    {
      name: 'koharu.workspace',
      // The choices a user makes once and expects again next time, as Photoshop keeps its tool
      // and brush; what's selected, running, or open starts afresh.
      partialize: ({ tool, brush, processingScope, processingStages, galleryOpen }) => ({
        tool,
        brush,
        processingScope,
        processingStages,
        galleryOpen,
      }),
    },
  ),
)

/** Every run's events so far, as the editor loads. */
export function receiveRuns(events: PipelineEvent[]): void {
  const jobs: Record<string, Job> = {}
  for (const event of events) jobs[String(event.run)] = fold(jobs[String(event.run)], event)
  useKoharuStore.setState({ initialized: true, jobs })
}

export function receivePipeline(event: PipelineEvent): void {
  useKoharuStore.setState((state) => ({
    jobs: { ...state.jobs, [String(event.run)]: fold(state.jobs[String(event.run)], event) },
  }))
}

function fold(job: Job | undefined, event: PipelineEvent): Job {
  const current: Job = job ?? {
    id: event.run,
    state: 'running',
    completed: 0,
    total: 0,
    stage: null,
    page: null,
    failures: [],
    flagged: [],
  }
  switch (event.kind) {
    case 'queued':
      return { ...current, total: event.steps }
    case 'started':
      return { ...current, stage: event.stage, page: event.page }
    case 'finished':
      return { ...current, completed: current.completed + 1 }
    case 'failed': {
      const { page, stage, reason } = event
      return { ...current, failures: [...current.failures, { page, stage, reason }] }
    }
    case 'flagged': {
      const { page, flags } = event
      return { ...current, flagged: [...current.flagged, { page, flags }] }
    }
    case 'ended':
      // The outcome's list is the whole of it, so a history replayed in part still ends right.
      return event.outcome.kind === 'failed'
        ? { ...current, state: 'failed', failures: event.outcome.failures }
        : { ...current, state: event.outcome.kind }
  }
}

export function receiveDownload(download: Transfer): void {
  useKoharuStore.setState((state) => {
    const downloads = { ...state.downloads }
    if (download.state === 'Finished') delete downloads[String(download.id)]
    else downloads[String(download.id)] = download
    return { downloads }
  })
}

/** A selection the canvas made. */
export function receiveCanvasSelection(layers: LayerId[]): void {
  useKoharuStore.setState({ selectedLayers: [...new Set(layers)], canvasSelection: layers })
}

export function receivePickedColor(color: string): void {
  useKoharuStore.setState((state) => ({
    picked: { color, serial: (state.picked?.serial ?? 0) + 1 },
  }))
}
