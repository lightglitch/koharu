'use client'

import { RefreshCcwDot } from 'lucide-react'
import { useTranslation } from 'react-i18next'

import { call } from '@/lib/backend'
import { pipelineStages, useKoharuStore } from '@/lib/store'
import { commands, type PageId, type Target } from '@koharu/ipc/protocol'
import { Button } from '@koharu/ui/components/button'
import { Tooltip, TooltipContent, TooltipTrigger } from '@koharu/ui/components/tooltip'

/** Whether a pipeline run is in flight: the backend refuses a second one, so the rail's run
 * controls stand down rather than raise an error toast. */
function useRunActive(): boolean {
  return useKoharuStore((state) => Object.values(state.jobs).some((job) => job.state === 'running'))
}

function runPipeline(target: Target) {
  void call(commands.runPipeline, target, [...pipelineStages]).catch(() => undefined)
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
