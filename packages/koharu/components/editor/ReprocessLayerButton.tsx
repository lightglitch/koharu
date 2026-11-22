'use client'

import { RefreshCcwDot } from 'lucide-react'
import { useTranslation } from 'react-i18next'

import { type BlockLayer } from '@/lib/document'
import { send } from '@/lib/queries'
import { useKoharuStore } from '@/lib/store'
import { commands } from '@koharu/ipc/protocol'
import { Tooltip, TooltipContent, TooltipTrigger } from '@koharu/ui/components/tooltip'

/** Reruns recognition and translation on one block from its row in Layers. Withheld while a run
 * is in flight, as the command bar's run control is, and on a locked block, as Story's translate
 * is. */
export function ReprocessLayerButton({
  block,
  name,
  locked,
}: {
  block: BlockLayer
  name: string
  locked: boolean
}) {
  const { t } = useTranslation()
  const running = useKoharuStore((state) =>
    Object.values(state.jobs).some((job) => job.state === 'running'),
  )
  if (running || locked) return null
  return (
    <Tooltip>
      <TooltipTrigger
        render={
          <button
            type='button'
            aria-label={t('layers.process', { name })}
            className='grid size-6 shrink-0 place-items-center rounded-md text-muted-foreground opacity-0 group-hover:opacity-100 hover:bg-foreground/[0.06] hover:text-foreground focus-visible:opacity-100 focus-visible:ring-2 focus-visible:ring-ring/25'
            onClick={() =>
              send(commands.runPipeline, { blocks: [block.id] }, ['recognize', 'translate'])
            }
          />
        }
      >
        <RefreshCcwDot className='size-3.5' />
      </TooltipTrigger>
      {/* The accessible name carries the block's text, which can be a whole line of dialogue;
          the tooltip stays short. */}
      <TooltipContent side='left'>{t('layers.processLayer')}</TooltipContent>
    </Tooltip>
  )
}
