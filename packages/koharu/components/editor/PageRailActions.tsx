'use client'

import { useMutation } from '@tanstack/react-query'
import { RefreshCcwDot, Trash2 } from 'lucide-react'
import { useTranslation } from 'react-i18next'
import { create } from 'zustand'

import { send, usePage } from '@/lib/queries'
import { deletePages } from '@/lib/scene'
import { defaultStages, useKoharuStore } from '@/lib/store'
import { commands, type PageId, type PageSummary, type Target } from '@koharu/ipc/protocol'
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from '@koharu/ui/components/alert-dialog'
import { Button } from '@koharu/ui/components/button'
import { Tooltip, TooltipContent, TooltipTrigger } from '@koharu/ui/components/tooltip'
import { cn } from '@koharu/ui/lib/utils'

/** Whether a pipeline run is in flight: the backend refuses a second one, so the rail's run
 * controls stand down rather than raise an error toast. */
function useRunActive(): boolean {
  return useKoharuStore((state) => Object.values(state.jobs).some((job) => job.state === 'running'))
}

function runPipeline(target: Target) {
  send(commands.runPipeline, target, [...defaultStages])
}

/** The rail header's controls: they act on a multi-selection when there is one, and on the
 * whole project otherwise. A single selected page is just the shown one, which a plain click
 * always leaves, so following it would make the whole project unreachable. */
export function RailHeaderActions({
  pages,
  selected,
}: {
  pages: PageSummary[]
  selected: PageId[]
}) {
  const { t } = useTranslation()
  const running = useRunActive()
  const ask = useDeleteRequest((state) => state.ask)
  const selection = selected.length > 1
  const deleteLabel = selection
    ? t('navigator.deleteSelected', { count: selected.length })
    : t('navigator.deleteAll')
  return (
    <div className={cn('flex shrink-0 items-center gap-0.5', selection ? 'ml-1.5' : 'ml-auto')}>
      <RunPagesButton disabled={pages.length === 0} selection={selection ? selected : null} />
      <Tooltip>
        <TooltipTrigger
          render={
            <Button
              variant='ghost'
              size='icon-xs'
              disabled={running || pages.length === 0}
              aria-label={deleteLabel}
              className='shadow-none hover:text-destructive'
              onClick={() => ask(selection ? selected : pages.map((page) => page.id), selection)}
            />
          }
        >
          <Trash2 />
        </TooltipTrigger>
        <TooltipContent side='bottom'>{deleteLabel}</TooltipContent>
      </Tooltip>
      <DeletePagesDialog pages={pages} />
    </div>
  )
}

/** The pages a bulk delete asks about, snapshotted when it asks and kept while its dialog closes,
 * so the dialog still names what it asked about. Shared with the rail's rows: a row's Delete on a
 * page within a multi-selection asks about the whole selection here. */
const useDeleteRequest = create<{
  open: boolean
  pages: PageId[]
  selection: boolean
  ask: (pages: PageId[], selection: boolean) => void
  setOpen: (open: boolean) => void
}>()((set) => ({
  open: false,
  pages: [],
  selection: false,
  ask: (pages, selection) => set({ open: true, pages: [...pages], selection }),
  setOpen: (open) => set({ open }),
}))

/** Asks to delete the whole selection when `page` belongs to a multi-selection, and reports
 * whether it did; otherwise the row's own single-page Delete applies. */
export function deleteWithSelection(page: PageId, selected: PageId[]): boolean {
  if (selected.length < 2 || !selected.includes(page)) return false
  useDeleteRequest.getState().ask(selected, true)
  return true
}

function DeletePagesDialog({ pages }: { pages: PageSummary[] }) {
  const { t } = useTranslation()
  const { open, pages: asked, selection, setOpen } = useDeleteRequest()
  const shown = usePage().data?.id ?? null
  const count = asked.length
  const remove = useMutation({
    mutationFn: (targets: PageId[]) => deletePages(targets),
    onSuccess: (_, targets) => {
      const store = useKoharuStore.getState()
      store.selectPages(store.selectedPages.filter((page) => !targets.includes(page)))
      if (shown !== null && targets.includes(shown)) store.selectLayers([])
    },
  })

  const confirm = () => {
    setOpen(false)
    // Only pages still in the project: one may have gone while the dialog was open.
    const targets = asked.filter((id) => pages.some((page) => page.id === id))
    if (targets.length === 0) return
    remove.mutate(targets)
  }

  return (
    <AlertDialog open={open} onOpenChange={setOpen}>
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>
            {selection
              ? t('navigator.deleteSelectedTitle', { count })
              : t('navigator.deleteAllTitle')}
          </AlertDialogTitle>
          <AlertDialogDescription>
            {selection
              ? t('navigator.deleteSelectedDescription', { count })
              : t('navigator.deleteAllDescription', { count })}
          </AlertDialogDescription>
        </AlertDialogHeader>
        <AlertDialogFooter>
          <AlertDialogCancel>{t('common.cancel')}</AlertDialogCancel>
          <AlertDialogAction variant='destructive' onClick={confirm}>
            {selection ? t('navigator.deleteSelectedAction') : t('navigator.deleteAllAction')}
          </AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  )
}

function RunPagesButton({
  disabled,
  selection,
}: {
  disabled: boolean
  selection: PageId[] | null
}) {
  const { t } = useTranslation()
  const running = useRunActive()
  const label = selection
    ? t('navigator.processSelection', { count: selection.length })
    : t('navigator.processProject')
  return (
    <Tooltip>
      <TooltipTrigger
        render={
          <Button
            variant='ghost'
            size='icon-xs'
            disabled={disabled || running}
            aria-label={label}
            className='shadow-none'
            onClick={() => runPipeline(selection ? { pages: selection } : 'project')}
          />
        }
      >
        <RefreshCcwDot />
      </TooltipTrigger>
      <TooltipContent side='bottom'>{label}</TooltipContent>
    </Tooltip>
  )
}

/** Runs every stage on the page its row stands for, revealed while the row is hovered. */
export function RunPageButton({ page, name }: { page: PageId; name: string }) {
  const { t } = useTranslation()
  const running = useRunActive()
  const label = t('navigator.processPage', { page: name })
  return (
    <Tooltip>
      <TooltipTrigger
        render={
          <Button
            variant='ghost'
            size='icon-xs'
            disabled={running}
            aria-label={label}
            // Revealed with visibility, not opacity: Button's disabled:opacity-50 outranks a
            // plain opacity-0, which would show the button on every row while a run disables them.
            className='invisible -mt-1 shrink-0 shadow-none group-hover:visible focus-visible:visible'
            onClick={() => runPipeline({ pages: [page] })}
          />
        }
      >
        <RefreshCcwDot />
      </TooltipTrigger>
      <TooltipContent side='left'>{label}</TooltipContent>
    </Tooltip>
  )
}
