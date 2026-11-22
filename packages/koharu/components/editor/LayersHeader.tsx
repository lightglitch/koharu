'use client'

import { useMutation } from '@tanstack/react-query'
import { Eye, EyeOff, Trash2 } from 'lucide-react'
import { useState } from 'react'
import { useTranslation } from 'react-i18next'

import { isBlockLayer, isContainer, isLockedLayer, isTextLayer, walkLayers } from '@/lib/document'
import { send } from '@/lib/queries'
import { deleteLayers, setVisibility } from '@/lib/scene'
import { useKoharuStore } from '@/lib/store'
import { type Layer, type LayerId } from '@koharu/ipc/protocol'
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogMedia,
  AlertDialogTitle,
} from '@koharu/ui/components/alert-dialog'
import { Button } from '@koharu/ui/components/button'
import { Tooltip, TooltipContent, TooltipTrigger } from '@koharu/ui/components/tooltip'

/** The Layers panel's own header: page-wide actions over the shown page's layers. */
export function LayersHeader({ layers }: { layers: Layer[] }) {
  const { t } = useTranslation()
  const [confirmingDelete, setConfirmingDelete] = useState(false)
  // The scene refuses an edit to a locked layer or anything inside one, and one refusal fails the
  // whole edit, so page-wide actions leave those layers alone.
  const unlocked = walkLayers(layers).filter((placed) => !isLockedLayer(placed))

  // A container stays visible while its children are hidden, so counting one would keep the page
  // from ever reading as hidden. Containers are still written to: one left hidden keeps its
  // children off the canvas however visible they are.
  const anyVisible = unlocked.some(({ layer }) => !isContainer(layer) && layer.visible)
  const toggleLabel = anyVisible ? t('layers.hideAll') : t('layers.showAll')

  const texts = unlocked
    .map(({ layer }) => layer)
    .filter((layer) => isTextLayer(layer) || isBlockLayer(layer))
  const remove = useMutation({
    mutationFn: ({ layers }: { layers: LayerId[]; gone: Set<LayerId> }) => deleteLayers(layers),
    onSuccess: (_, { gone }) => {
      const { selectedLayers, selectLayers } = useKoharuStore.getState()
      selectLayers(selectedLayers.filter((selected) => !gone.has(selected)))
    },
  })
  const deleteTexts = () => {
    setConfirmingDelete(false)
    if (texts.length === 0) return
    // A block's clean-up and mask go with it, so they leave the selection too.
    const gone = new Set<LayerId>(
      texts.flatMap((layer) =>
        isBlockLayer(layer)
          ? [layer.id, layer.cleanup?.id, layer.mask?.id].filter((id) => id !== undefined)
          : [layer.id],
      ),
    )
    remove.mutate({ layers: texts.map((layer) => layer.id), gone })
  }

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
                send(
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
      <Tooltip>
        <TooltipTrigger
          render={
            <Button
              variant='ghost'
              size='icon-xs'
              disabled={texts.length === 0}
              aria-label={t('layers.deleteAllText')}
              className='shadow-none'
              onClick={() => setConfirmingDelete(true)}
            />
          }
        >
          <Trash2 />
        </TooltipTrigger>
        <TooltipContent side='bottom'>{t('layers.deleteAllText')}</TooltipContent>
      </Tooltip>
      {/* Unlike a row's delete, this is bulk, so it asks first, as deleting a page does. */}
      <AlertDialog open={confirmingDelete} onOpenChange={setConfirmingDelete}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogMedia className='bg-destructive/10 text-destructive'>
              <Trash2 className='size-5' />
            </AlertDialogMedia>
            <AlertDialogTitle>{t('layers.deleteAllTextTitle')}</AlertDialogTitle>
            <AlertDialogDescription>
              {t('layers.deleteAllTextDescription', { count: texts.length })}
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>{t('common.cancel')}</AlertDialogCancel>
            <AlertDialogAction variant='destructive' onClick={deleteTexts}>
              {t('layers.deleteAllTextAction')}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </header>
  )
}
