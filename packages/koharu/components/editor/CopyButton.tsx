'use client'

import { Check, Copy } from 'lucide-react'
import { useEffect, useState } from 'react'
import { useTranslation } from 'react-i18next'

import { call } from '@/lib/backend'
import { commands } from '@koharu/ipc/protocol'
import { Button } from '@koharu/ui/components/button'

/** Copies an error to the system clipboard, to paste into an issue or a chat: selecting it out of
 * the small Activity panel by hand is awkward. A tick acknowledges it for a moment, as a copy is
 * otherwise invisible. */
// Fork addition: upstream's Activity has no copy button.
export function CopyButton({ text }: { text: string }) {
  const { t } = useTranslation()
  const [copied, setCopied] = useState(false)
  useEffect(() => {
    if (!copied) return
    const timer = window.setTimeout(() => setCopied(false), 1500)
    return () => window.clearTimeout(timer)
  }, [copied])
  return (
    <Button
      size='icon-xs'
      variant='ghost'
      className='-mt-1 shrink-0'
      aria-label={copied ? t('activity.copied') : t('activity.copyError')}
      onClick={() =>
        void call(commands.copyText, text)
          .then(() => setCopied(true))
          .catch(() => undefined)
      }
    >
      {copied ? <Check /> : <Copy />}
    </Button>
  )
}
