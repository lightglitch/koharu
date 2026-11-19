'use client'

import { useEffect } from 'react'
import { useTranslation } from 'react-i18next'

import { dispatch } from '@/lib/backend'
import { showPageBy, useGoToPage } from '@/lib/pages'
import { pageKey, queryClient } from '@/lib/queries'
import { MIN_BRUSH_DIAMETER, maxBrushDiameter, useKoharuStore } from '@/lib/store'
import { commands, events, type Page, type WheelInput } from '@koharu/ipc/protocol'

// Fork addition: upstream has no wheel tools.

/** How much a notch zooms or sizes the brush: a fraction of what it is, as a mouse wheel zooms. */
const NOTCH = 1.1
/** The status bar's zoom range. */
const MIN_ZOOM = 0.1
const MAX_ZOOM = 8

/** Koharu's tools in the Windows wheel's menu, named in the interface's language, and what their
 * turns and presses do: Pages turns through the pages and asks for one on a press, Zoom zooms and
 * fits, Brush sizes the brush and swaps the Brush and the Eraser. */
export function WheelTools() {
  const { t, i18n } = useTranslation()

  useEffect(
    () =>
      dispatch(commands.showWheelTools, {
        pages: t('wheel.pages'),
        zoom: t('wheel.zoom'),
        brush: t('wheel.brush'),
      }),
    [t, i18n.language],
  )

  useEffect(() => {
    const listening = events.wheel(act)
    return () => void listening.then((unlisten) => unlisten())
  }, [])

  return null
}

function act(input: WheelInput): void {
  const state = useKoharuStore.getState()
  switch (input.tool) {
    case 'pages':
      if (input.kind === 'turn') showPageBy(input.steps)
      else useGoToPage.getState().setOpen(true)
      return
    case 'zoom':
      if (input.kind === 'turn') {
        const zoom = state.camera.zoom * NOTCH ** input.steps
        dispatch(commands.setZoom, Math.min(MAX_ZOOM, Math.max(MIN_ZOOM, zoom)))
      } else dispatch(commands.fit)
      return
    case 'brush':
      if (input.kind === 'turn') {
        const { diameter } = state.brush
        const scaled = Math.round(diameter * NOTCH ** input.steps)
        // A small brush still changes by a pixel a notch.
        const next = scaled === diameter ? diameter + Math.sign(input.steps) : scaled
        const page = queryClient.getQueryData<Page | null>(pageKey)
        const max = maxBrushDiameter(page?.size ?? { width: 2048, height: 2048 })
        state.setBrush({ ...state.brush, diameter: Math.min(max, Math.max(MIN_BRUSH_DIAMETER, next)) })
      } else state.setTool(state.tool === 'draw' ? 'eraser' : 'draw')
      return
  }
}
