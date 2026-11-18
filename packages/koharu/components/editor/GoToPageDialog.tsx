'use client'

import { useState } from 'react'
import { useTranslation } from 'react-i18next'

import { showPage, useGoToPage } from '@/lib/pages'
import { useProject } from '@/lib/queries'
import { type PageSummary } from '@koharu/ipc/protocol'
import { Button } from '@koharu/ui/components/button'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@koharu/ui/components/dialog'
import { Input } from '@koharu/ui/components/input'

const emptyPages: PageSummary[] = []

export function GoToPageDialog() {
  const open = useGoToPage((state) => state.open)
  const setOpen = useGoToPage((state) => state.setOpen)
  const pages = useProject().data?.pages ?? emptyPages

  return (
    <Dialog open={open} onOpenChange={setOpen}>
      <DialogContent className='max-w-xs gap-4 p-5'>
        {/* Mounted only while open, so each visit starts empty. */}
        <GoToPageForm pages={pages} onClose={() => setOpen(false)} />
      </DialogContent>
    </Dialog>
  )
}

function GoToPageForm({ pages, onClose }: { pages: PageSummary[]; onClose: () => void }) {
  const { t } = useTranslation()
  const [value, setValue] = useState('')
  const number = Number(value.trim())
  // Refused rather than clamped: landing somewhere other than the number typed would be worse
  // than doing nothing.
  const target = value.trim() !== '' && Number.isInteger(number) ? pages[number - 1]?.id : undefined

  const go = () => {
    if (!target) return
    onClose()
    showPage(target)
  }

  return (
    <>
      <DialogHeader className='gap-1'>
        <DialogTitle className='text-[15px]'>{t('navigator.goToTitle')}</DialogTitle>
        <DialogDescription className='text-[11px]'>
          {t('navigator.goToHint', { count: pages.length })}
        </DialogDescription>
      </DialogHeader>
      <form
        onSubmit={(event) => {
          event.preventDefault()
          go()
        }}
      >
        <Input
          autoFocus
          inputMode='numeric'
          aria-label={t('navigator.goToLabel')}
          placeholder={t('navigator.goToLabel')}
          value={value}
          className='h-8 text-[12px]'
          onChange={(event) => setValue(event.currentTarget.value)}
        />
      </form>
      <DialogFooter>
        <Button type='button' variant='ghost' size='sm' onClick={onClose}>
          {t('common.cancel')}
        </Button>
        <Button type='button' size='sm' disabled={!target} onClick={go}>
          {t('navigator.goToAction')}
        </Button>
      </DialogFooter>
    </>
  )
}
