'use client'

import { useEffect, useState } from 'react'
import { useTranslation } from 'react-i18next'

import { activatePage } from '@/lib/pages'
import { usePages } from '@/lib/queries'
import { useKoharuStore } from '@/lib/store'
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

export function GoToPageDialog() {
  const { t } = useTranslation()
  const open = useKoharuStore((state) => state.goToPageOpen)
  const setOpen = useKoharuStore((state) => state.setGoToPageOpen)
  const pages = usePages().data ?? []
  const [value, setValue] = useState('')

  // Each visit starts empty rather than from whatever was typed last time.
  useEffect(() => {
    if (open) setValue('')
  }, [open])

  const number = Number.parseInt(value, 10)
  const target = Number.isNaN(number) ? undefined : pages[number - 1]?.id

  const go = () => {
    if (!target) return
    setOpen(false)
    activatePage(target, [target])
  }

  return (
    <Dialog open={open} onOpenChange={setOpen}>
      <DialogContent className='max-w-xs gap-4 p-5'>
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
          <Button type='button' variant='ghost' size='sm' onClick={() => setOpen(false)}>
            {t('common.cancel')}
          </Button>
          {/* Disabled rather than clamped: silently jumping somewhere other
              than the number typed would be worse than doing nothing. */}
          <Button type='button' size='sm' disabled={!target} onClick={go}>
            {t('navigator.goToAction')}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
