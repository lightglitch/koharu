'use client'

import { TextCursorInput } from 'lucide-react'
import { useTranslation } from 'react-i18next'

import { call } from '@/lib/backend'
import { defaultStages, pipelineStages, useKoharuStore } from '@/lib/store'
import { commands, type Stage } from '@koharu/ipc/protocol'
import { Button } from '@koharu/ui/components/button'
import { Tooltip, TooltipContent, TooltipTrigger } from '@koharu/ui/components/tooltip'

// Fork addition: upstream's area bar only offers Generative Fill.

/** The stages a block can run after detection, and those it runs when the Run control has none
 * of them ticked: validation only when ticked. */
const AFTER_DETECTION: readonly Stage[] = pipelineStages.filter((stage) => stage !== 'detect')
const BY_DEFAULT: readonly Stage[] = defaultStages.filter((stage) => stage !== 'detect')

/** Makes the marked area a text block, for text detection missed, and runs the Run control's
 * ticked stages after detection on that block alone: OCR, clean-up, and translation by default. */
export function AddTextButton() {
  const { t } = useTranslation()
  const add = async () => {
    const block = await call(commands.addTextFromArea, t('area.addTextStep'))
    if (block === null) return
    const ticked = useKoharuStore.getState().processingStages
    const stages = AFTER_DETECTION.filter((stage) => ticked.includes(stage))
    await call(commands.runPipeline, { blocks: [block] }, stages.length ? stages : [...BY_DEFAULT])
  }
  return (
    <Tooltip>
      <TooltipTrigger
        render={
          <Button
            type='button'
            variant='secondary'
            size='sm'
            className='h-7 text-[11px]'
            onClick={() => void add().catch(() => undefined)}
          />
        }
      >
        <TextCursorInput />
        {t('area.addText')}
      </TooltipTrigger>
      <TooltipContent side='bottom'>{t('area.addTextHint')}</TooltipContent>
    </Tooltip>
  )
}
