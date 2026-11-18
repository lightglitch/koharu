'use client'

import { Eye, EyeOff } from 'lucide-react'
import { useTranslation } from 'react-i18next'

import { dispatch } from '@/lib/backend'
import { isContainer, isLockedLayer, walkLayers } from '@/lib/document'
import { setVisibility } from '@/lib/scene'
import { type Layer } from '@koharu/ipc/protocol'
import { Button } from '@koharu/ui/components/button'
import { Tooltip, TooltipContent, TooltipTrigger } from '@koharu/ui/components/tooltip'

/** The Layers panel's own header: page-wide actions over the shown page's layers. */
export function LayersHeader({ layers }: { layers: Layer[] }) {
  const { t } = useTranslation()
  // The scene refuses an edit to a locked layer or anything inside one, and one refusal fails the
  // whole edit, so page-wide actions leave those layers alone.
  const unlocked = walkLayers(layers).filter((placed) => !isLockedLayer(placed))

  // A container stays visible while its children are hidden, so counting one would keep the page
  // from ever reading as hidden. Containers are still written to: one left hidden keeps its
  // children off the canvas however visible they are.
  const anyVisible = unlocked.some(({ layer }) => !isContainer(layer) && layer.visible)
  const toggleLabel = anyVisible ? t('layers.hideAll') : t('layers.showAll')

  return (
    <header className='flex h-8 shrink-0 items-center justify-end gap-0.5 border-b border-border/80 px-2'>
      <Tooltip>
        <TooltipTrigger
          render={
            <Button
              variant='ghost'
              size='icon-xs'
              disabled={unlocked.length === 0}
              aria-label={toggleLabel}
              className='shadow-none'
              onClick={() =>
                dispatch(
                  setVisibility,
                  unlocked.map(({ layer }) => layer.id),
                  !anyVisible,
                  null,
                )
              }
            />
          }
        >
          {anyVisible ? <Eye /> : <EyeOff />}
        </TooltipTrigger>
        <TooltipContent side='bottom'>{toggleLabel}</TooltipContent>
      </Tooltip>
    </header>
  )
}
