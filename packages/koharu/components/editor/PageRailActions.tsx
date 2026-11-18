'use client'

import { RefreshCcwDot } from 'lucide-react'
import { useTranslation } from 'react-i18next'

import { call } from '@/lib/backend'
import { pipelineStages, useKoharuStore } from '@/lib/store'
import { commands, type PageId, type Target } from '@koharu/ipc/protocol'
import { Button } from '@koharu/ui/components/button'
import { Tooltip, TooltipContent, TooltipTrigger } from '@koharu/ui/components/tooltip'
import { cn } from '@koharu/ui/lib/utils'

/** Whether a pipeline run is in flight: the backend refuses a second one, so the rail's run
 * controls stand down rather than raise an error toast. */
function useRunActive(): boolean {
  return useKoharuStore((state) => Object.values(state.jobs).some((job) => job.state === 'running'))
}

function runPipeline(target: Target) {
  void call(commands.runPipeline, target, [...pipelineStages]).catch(() => undefined)
}

/** The rail header's controls: they act on a multi-selection when there is one, and on the
 * whole project otherwise. A single selected page is just the shown one, which a plain click
 * always leaves, so following it would make the whole project unreachable. */
export function RailHeaderActions({ pages, selected }: { pages: number; selected: PageId[] }) {
  const selection = selected.length > 1
  return (
    <div className={cn('flex shrink-0 items-center gap-0.5', selection ? 'ml-1.5' : 'ml-auto')}>
      <RunPagesButton disabled={pages === 0} selection={selection ? selected : null} />
    </div>
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
