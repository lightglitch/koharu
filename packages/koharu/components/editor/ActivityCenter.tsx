'use client'

import { useMutationState } from '@tanstack/react-query'
import type { TFunction } from 'i18next'
import {
  CircleAlert,
  Download,
  LoaderCircle,
  ScanSearch,
  Square,
  TriangleAlert,
  X,
} from 'lucide-react'
import type { ReactNode } from 'react'
import { useTranslation } from 'react-i18next'

import { useExportProgress } from '@/lib/exportProgress'
import { showPage } from '@/lib/pages'
import { send, usePages } from '@/lib/queries'
import { useKoharuStore, type Job, type PageFlags } from '@/lib/store'
import {
  commands,
  type Flag,
  type PageFailure,
  type PageId,
  type Transfer,
} from '@koharu/ipc/protocol'
import { Button } from '@koharu/ui/components/button'
import { Progress, ProgressLabel, ProgressValue } from '@koharu/ui/components/progress'
import { ScrollArea } from '@koharu/ui/components/scroll-area'

import { CopyButton } from './CopyButton'

export function ActivityCenter() {
  const { t } = useTranslation()
  const jobs = useKoharuStore((state) => state.jobs)
  const downloads = useKoharuStore((state) => state.downloads)
  // A chapter's pages still coming; a failure stops it, and its toast says why.
  const loading = useKoharuStore((state) => state.loading)
  const chapter = loading && !loading.error && loading.loaded < loading.total ? loading : null
  const activities = useMutationState({
    filters: { status: 'pending' },
    select: (mutation) => ({ id: mutation.mutationId, label: mutation.meta?.activity }),
  }).flatMap(({ id, label }) => (label ? [{ id, label }] : []))
  // Fork addition: a run that flagged texts stays until dismissed, as a failed one does.
  const visibleJobs = Object.values(jobs).filter(
    (job) => job.state === 'running' || job.state === 'failed' || job.flagged.length > 0,
  )
  const runningDownloads = Object.values(downloads).filter(
    (download) => download.state === 'Running',
  )
  const failedDownloads = Object.values(downloads).filter((download) => failure(download) !== null)
  const visibleDownloads = [...runningDownloads, ...failedDownloads]
  if (
    activities.length === 0 &&
    visibleJobs.length === 0 &&
    visibleDownloads.length === 0 &&
    !chapter
  )
    return null

  return (
    <aside
      aria-label={t('activity.title')}
      className='absolute right-3 bottom-9 z-30 flex w-72 max-w-[calc(100%-1.5rem)] flex-col rounded-2xl border border-border/50 bg-popover shadow-md'
    >
      <div className='border-b px-3 py-2'>
        <span className='text-[10px] font-semibold tracking-[0.1em] uppercase'>
          {t('activity.title')}
        </span>
      </div>
      {/* Fork addition: failed runs stay until dismissed, so the rows scroll under the title past
          a few rather than grow the panel up over the canvas. */}
      <ScrollArea viewportClassName='max-h-80'>
        {activities.map((activity) =>
          // Fork addition: the export's row follows its pages and stops it.
          activity.label === 'menu.export' ? (
            <ExportItem key={activity.id} />
          ) : (
            <div
              key={activity.id}
              role='status'
              className='grid grid-cols-[1rem_minmax(0,1fr)] items-center gap-x-2.5 border-b p-3 last:border-b-0'
            >
              <LoaderCircle
                className='size-3.5 animate-spin justify-self-center text-primary motion-reduce:animate-none'
                aria-hidden='true'
              />
              <span className='truncate text-[11px] font-medium'>{t(activity.label)}</span>
            </div>
          ),
        )}
        {chapter && (
          <ProgressRow
            icon={<Download className='mt-0.5 size-3.5 justify-self-center text-primary' />}
            label={t('activity.loadingPages', { loaded: chapter.loaded, total: chapter.total })}
            completed={chapter.loaded}
            total={chapter.total}
          />
        )}
        {runningDownloads.length > 1 ? (
          <DownloadGroup downloads={runningDownloads} />
        ) : (
          runningDownloads.map((download) => <DownloadItem key={download.id} download={download} />)
        )}
        {failedDownloads.map((download) => (
          <DownloadItem key={download.id} download={download} />
        ))}
        {visibleJobs.map((job) => (
          <JobItem key={job.id} job={job} />
        ))}
      </ScrollArea>
    </aside>
  )
}

function DownloadGroup({ downloads }: { downloads: Transfer[] }) {
  const { t } = useTranslation()
  const total = downloads.reduce((sum, download) => sum + download.total, 0)
  const completed = downloads.reduce((sum, download) => sum + download.completed, 0)
  return (
    <ProgressRow
      icon={<Download className='mt-0.5 size-3.5 justify-self-center text-primary' />}
      label={t('activity.downloadingFiles', { count: downloads.length })}
      completed={downloads.every((download) => download.total > 0) ? completed : null}
      total={total}
    />
  )
}

function JobItem({ job }: { job: Job }) {
  const { t } = useTranslation()
  const dismiss = useKoharuStore((state) => state.dismissJob)
  // Numbered by position in the rail, so the row says where in the run this is.
  const pages = usePages(
    job.page !== null || job.failures.length > 0 || job.flagged.length > 0,
  ).data
  const pageLabel = (id: PageId) => {
    const index = pages?.findIndex((page) => page.id === id) ?? -1
    return index >= 0 ? t('navigator.pageNumber', { number: index + 1 }) : undefined
  }
  // Fork addition: the run goes on past a failed page, and lists each, and what validation flagged.
  const failures = job.failures.map((failure) => ({
    ...failure,
    label: pageLabel(failure.page),
  }))
  const flagged = job.flagged.map((page) => ({ ...page, label: pageLabel(page.page) }))
  const details =
    failures.length > 0 || flagged.length > 0 ? (
      <>
        {failures.length > 0 ? <PageFailures failures={failures} /> : null}
        {flagged.length > 0 ? <FlaggedPages pages={flagged} /> : null}
      </>
    ) : null
  if (job.state !== 'running' && details) {
    const failed = job.state === 'failed'
    const Icon = failed ? CircleAlert : ScanSearch
    // Each failure and flag on its own line, naming its page once the text has left the panel.
    const report = [
      ...failures.map((failure) =>
        failure.label ? `${failure.label}: ${failure.reason}` : failure.reason,
      ),
      ...flagged.flatMap((page) =>
        page.flags.map((flag) => `${page.label ?? ''}: ${flagMessage(t, flag)}`),
      ),
    ].join('\n')
    return (
      <div className='grid grid-cols-[1rem_minmax(0,1fr)_2.25rem_1.5rem] items-start gap-x-2.5 border-b p-3 text-[11px] last:border-b-0'>
        <Icon
          className={`mt-0.5 size-3.5 justify-self-center ${failed ? 'text-destructive' : 'text-amber-600 dark:text-amber-400'}`}
        />
        <p
          className={`min-w-0 font-medium ${failed ? 'text-destructive' : 'text-amber-700 dark:text-amber-400'}`}
        >
          {failed
            ? t('activity.finishedWithFailures')
            : t('validation.flaggedPages', { count: flagged.length })}
        </p>
        <span className='justify-self-end'>
          <CopyButton text={report} />
        </span>
        <Button
          size='icon-xs'
          variant='ghost'
          className='-mt-1'
          aria-label={t('activity.dismiss')}
          onClick={() => dismiss(String(job.id))}
        >
          <X />
        </Button>
        {details}
      </div>
    )
  }
  return (
    <ProgressRow
      icon={<span className='mt-1.5 size-1.5 justify-self-center rounded-full bg-primary' />}
      label={
        job.stage ? t(`phase.${job.stage}`, { defaultValue: job.stage }) : t('activity.processing')
      }
      detail={job.page ? pageLabel(job.page) : undefined}
      completed={job.completed}
      total={job.total}
      action={
        <Button
          size='icon-xs'
          variant='ghost'
          className='-mt-1'
          aria-label={t('activity.stop')}
          onClick={() => send(commands.stopPipeline)}
        >
          <Square className='size-2.5 fill-current' />
        </Button>
      }
    >
      {details}
    </ProgressRow>
  )
}

// Fork addition: what validation flagged, as warnings beside the failures.

/** A flag in the interface's language: what's wrong, with what was measured. */
function flagMessage(t: TFunction, flag: Flag) {
  const message = t(`validation.${flag.kind}`, {
    size: Math.round(flag.size),
    typical: flag.typical === null ? undefined : Math.round(flag.typical),
    contrast: flag.contrast === null ? undefined : flag.contrast.toFixed(1),
  })
  return flag.text ? `${message} “${flag.text}”` : message
}

/** The texts validation flagged, page by page; a click shows the page with the text selected. */
function FlaggedPages({ pages }: { pages: (PageFlags & { label?: string })[] }) {
  const { t } = useTranslation()
  const select = (page: PageId, flag: Flag) => {
    showPage(page)
    useKoharuStore.getState().selectLayers([flag.block])
  }
  return (
    <ul className='col-start-2 col-end-5 mt-2 grid gap-1'>
      {pages.flatMap((page) =>
        page.flags.map((flag, at) => (
          <li key={`${page.page}:${flag.block}:${flag.kind}:${at}`}>
            <button
              type='button'
              className='flex w-full items-start gap-1.5 rounded text-left hover:bg-accent'
              onClick={() => select(page.page, flag)}
            >
              <TriangleAlert className='mt-px size-3 shrink-0 text-amber-600 dark:text-amber-400' />
              <span className='line-clamp-3 min-w-0 flex-1 text-[10px] leading-4 break-words'>
                {page.label ? (
                  <span className='mr-1 font-medium text-amber-700 dark:text-amber-400'>
                    {page.label}
                  </span>
                ) : null}
                <span className='text-muted-foreground'>{flagMessage(t, flag)}</span>
              </span>
            </button>
          </li>
        )),
      )}
    </ul>
  )
}

/** The pages a run gave up on, each with why, as they come in while it goes on. A reason, which
 * can quote a model's whole answer, is cut at a few lines inside the panel's scrolling rows. */
// Fork addition.
function PageFailures({ failures }: { failures: (PageFailure & { label?: string })[] }) {
  return (
    <ul className='col-start-2 col-end-5 mt-2 grid gap-1'>
      {failures.map((failure) => (
        <li key={`${failure.page}:${failure.stage}`} className='flex items-start gap-1.5'>
          <CircleAlert className='mt-px size-3 shrink-0 text-destructive' />
          <span
            className='line-clamp-4 min-w-0 flex-1 text-[10px] leading-4 break-words'
            title={failure.reason}
          >
            {failure.label ? (
              <span className='mr-1 font-medium text-destructive'>{failure.label}</span>
            ) : null}
            <span className='text-muted-foreground'>{failure.reason}</span>
          </span>
          <CopyButton
            text={failure.label ? `${failure.label}: ${failure.reason}` : failure.reason}
          />
        </li>
      ))}
    </ul>
  )
}

/** The export going on: its pages written, and Stop, which deletes a CBZ it leaves unfinished. */
// Fork addition: upstream's export shows only that it's going on.
function ExportItem() {
  const { t } = useTranslation()
  const written = useExportProgress((state) => state.progress)
  return (
    <ProgressRow
      icon={
        <LoaderCircle
          className='mt-0.5 size-3.5 animate-spin justify-self-center text-primary motion-reduce:animate-none'
          aria-hidden='true'
        />
      }
      label={t('activity.exporting')}
      completed={written ? written.done : null}
      total={written?.total ?? 0}
      action={
        <Button
          size='icon-xs'
          variant='ghost'
          className='-mt-1'
          aria-label={t('activity.stop')}
          onClick={() => send(commands.stopExport)}
        >
          <Square className='size-2.5 fill-current' />
        </Button>
      }
    />
  )
}

function DownloadItem({ download }: { download: Transfer }) {
  const { t } = useTranslation()
  const dismiss = useKoharuStore((state) => state.dismissDownload)
  const error = failure(download)
  if (error !== null) {
    return (
      <Failure
        message={error || t('activity.downloadFailed')}
        onDismiss={() => dismiss(download.id)}
      />
    )
  }
  return (
    <ProgressRow
      icon={<Download className='mt-0.5 size-3.5 justify-self-center text-primary' />}
      label={download.name || t('activity.modelDownload')}
      completed={download.completed}
      total={download.total}
    />
  )
}

/** A row of work under way: its name and how far along it is, in the interface's language, over
 * its bar, which runs on its own while the total is unknown. */
function ProgressRow({
  icon,
  label,
  detail,
  completed,
  total,
  action,
  children,
}: {
  icon: ReactNode
  label: string
  /** Where the work is, under its name. */
  detail?: string
  completed: number | null
  total: number
  action?: ReactNode
  /** What goes under the bar, such as the pages a run gave up on. */
  children?: ReactNode
}) {
  const { i18n } = useTranslation()
  return (
    <div className='grid grid-cols-[1rem_minmax(0,1fr)_1.5rem] items-start gap-x-2.5 border-b p-3 last:border-b-0'>
      {icon}
      <Progress
        value={completed !== null && total > 0 ? Math.min(completed, total) : null}
        max={total || 100}
        locale={i18n.language}
        className='min-w-0 gap-x-2 gap-y-1.5'
      >
        <ProgressLabel className='min-w-0 flex-1 truncate text-[11px] font-medium'>
          {label}
        </ProgressLabel>
        <ProgressValue className='text-[10px] text-foreground' />
        {detail && <p className='w-full truncate text-[10px] text-muted-foreground'>{detail}</p>}
      </Progress>
      {action ?? <span aria-hidden='true' />}
      {children}
    </div>
  )
}

/** A failure's row: why, and under it where, as a running row names its page. */
function Failure({
  message,
  detail,
  onDismiss,
}: {
  message: string
  detail?: string
  onDismiss: () => void
}) {
  const { t } = useTranslation()
  return (
    <div className='grid grid-cols-[1rem_minmax(0,1fr)_2.25rem_1.5rem] items-start gap-x-2.5 border-b p-3 text-[11px] last:border-b-0'>
      <CircleAlert className='mt-0.5 size-3.5 justify-self-center text-destructive' />
      <div className='min-w-0'>
        <p className='text-destructive'>{message}</p>
        {detail ? (
          <p className='mt-0.5 truncate text-[10px] text-muted-foreground'>{detail}</p>
        ) : null}
      </div>
      {/* Fork addition. */}
      <span className='justify-self-end'>
        <CopyButton text={detail ? `${detail}: ${message}` : message} />
      </span>
      <Button
        size='icon-xs'
        variant='ghost'
        className='-mt-1'
        aria-label={t('activity.dismiss')}
        onClick={onDismiss}
      >
        <X />
      </Button>
    </div>
  )
}

/** Why a download failed, or `null` while it hasn't. */
function failure(download: Transfer): string | null {
  return typeof download.state === 'object' ? download.state.Failed : null
}
