'use client'

import { useQuery } from '@tanstack/react-query'
import { openUrl } from '@tauri-apps/plugin-opener'
import { Eraser } from 'lucide-react'
import { useId, useState } from 'react'
import { useTranslation } from 'react-i18next'

import { vendorName } from '@/components/preferences/models'
import {
  PreferencePage,
  PreferenceRow,
  PreferenceSection,
} from '@/components/preferences/PreferenceFields'
import { call, dispatch, refreshModels } from '@/lib/backend'
import { queryClient } from '@/lib/queries'
import {
  Channel,
  commands,
  type Providers,
  type SignInPrompt,
  type Vendor,
} from '@koharu/ipc/protocol'
import { Button } from '@koharu/ui/components/button'
import { Input } from '@koharu/ui/components/input'

const vendorsKey = ['vendors'] as const

/** Lists the vendors and models again after a key or a sign-in changed. */
async function vendorsChanged() {
  await queryClient.invalidateQueries({ queryKey: vendorsKey })
  await refreshModels(true)
}

/** Each vendor's API key, or its sign-in for one the user signs in to with their plan, kept in
 * the OS credential store and saved apart from `config.toml`, and the `[providers]` base URL for a
 * vendor whose server the user names. */
export function ProviderPreferences({
  value,
  onChange,
}: {
  value: Providers
  onChange: (value: Providers) => void
}) {
  const { t } = useTranslation()
  const vendors = useQuery({
    queryKey: vendorsKey,
    queryFn: () => call(commands.listVendors),
  })
  return (
    <PreferencePage
      title={t('settings.providers.title')}
      description={t('settings.providers.description')}
    >
      <PreferenceSection
        title={t('settings.providers.connections')}
        description={t('settings.providers.connectionsDescription')}
      >
        {(vendors.data ?? []).map((vendor) => (
          <PreferenceRow key={vendor.name} title={vendorName(vendor.name, t)} align='start'>
            <div className='grid gap-2'>
              {vendor.signs_in ? <SignInField vendor={vendor} /> : <KeyField vendor={vendor} />}
              {vendor.server && (
                <BaseUrlField
                  value={value[vendor.name]?.base_url ?? ''}
                  onChange={(base_url) =>
                    onChange(withEndpoint(value, vendor.name, { base_url: base_url || undefined }))
                  }
                />
              )}
              {/* Fork addition: the window a server's models hold, which only an
                  OpenAI-compatible server leaves unknown. */}
              {vendor.name === 'openai-compatible' && (
                <ContextField
                  value={value[vendor.name]?.context ?? null}
                  onChange={(context) =>
                    onChange(withEndpoint(value, vendor.name, { context: context ?? undefined }))
                  }
                />
              )}
            </div>
          </PreferenceRow>
        ))}
      </PreferenceSection>
    </PreferencePage>
  )
}

/** A vendor's key, written to the credential store when the field loses the focus. */
function KeyField({ vendor }: { vendor: Vendor }) {
  const { t } = useTranslation()
  const inputId = useId()
  const [draft, setDraft] = useState('')
  const name = vendorName(vendor.name, t)

  const store = (key: string | null) =>
    void call(commands.setApiKey, vendor.name, key)
      .then(async () => {
        setDraft('')
        await vendorsChanged()
      })
      .catch(() => undefined)

  return (
    <div className='grid gap-1'>
      <label htmlFor={inputId} className='text-[10px] text-muted-foreground'>
        {t('settings.providers.credential')}
      </label>
      <div className='flex gap-2'>
        <Input
          id={inputId}
          aria-label={t('settings.providers.credentialLabel', { provider: name })}
          type='text'
          autoComplete='off'
          autoCapitalize='none'
          spellCheck={false}
          value={draft}
          placeholder={
            vendor.connected
              ? t('settings.providers.configured')
              : t('settings.providers.notConfigured')
          }
          className='h-8 min-w-0 flex-1 text-[12px] [-webkit-text-security:disc] [&::placeholder]:[-webkit-text-security:none]'
          onChange={(event) => setDraft(event.currentTarget.value)}
          onBlur={() => draft && store(draft)}
          onKeyDown={(event) => {
            if (event.key === 'Enter') event.currentTarget.blur()
          }}
        />
        {vendor.connected && (
          <Button
            type='button'
            variant='outline'
            size='icon'
            aria-label={t('settings.providers.clearCredential', { provider: name })}
            onClick={() => store(null)}
          >
            <Eraser />
          </Button>
        )}
      </div>
    </div>
  )
}

/** A vendor's sign-in with the user's plan: a button that signs in, showing the page and the code
 * the sign-in waits on, or signs out. The browser opens on the page unless the sign-in opened it. */
function SignInField({ vendor }: { vendor: Vendor }) {
  const { t } = useTranslation()
  const [waiting, setWaiting] = useState(false)
  const [prompt, setPrompt] = useState<SignInPrompt | null>(null)
  const name = vendorName(vendor.name, t)

  const signIn = async () => {
    setWaiting(true)
    const channel = new Channel<SignInPrompt>()
    channel.onmessage = (asked) => {
      setPrompt(asked)
      if (!asked.opened) void openUrl(asked.url).catch(() => undefined)
    }
    try {
      await call(commands.signIn, vendor.name, channel)
      await vendorsChanged()
    } catch {
      // `call` reports the failure.
    } finally {
      setWaiting(false)
      setPrompt(null)
    }
  }

  const signOut = () =>
    void call(commands.signOut, vendor.name)
      .then(vendorsChanged)
      .catch(() => undefined)

  return (
    <div className='grid gap-1'>
      <span className='text-[10px] text-muted-foreground'>
        {t('settings.providers.subscription')}
      </span>
      {waiting ? (
        <div className='grid gap-2 rounded-md border bg-muted/40 p-2.5'>
          {prompt?.code && (
            <p className='font-mono text-base font-semibold tracking-[0.18em] select-text'>
              {prompt.code}
            </p>
          )}
          <p className='text-[11px] leading-4 text-muted-foreground'>
            {prompt === null
              ? t('settings.providers.signingIn')
              : prompt.code
                ? t('settings.providers.deviceCodeHint')
                : t('settings.providers.browserHint')}
          </p>
          <div className='flex flex-wrap gap-2'>
            {prompt && (
              <Button
                type='button'
                variant='outline'
                size='sm'
                onClick={() => void openUrl(prompt.url).catch(() => undefined)}
              >
                {t('settings.providers.openPage')}
              </Button>
            )}
            <Button
              type='button'
              variant='ghost'
              size='sm'
              onClick={() => dispatch(commands.stopSignIn)}
            >
              {t('common.cancel')}
            </Button>
          </div>
        </div>
      ) : (
        <Button
          type='button'
          variant='outline'
          size='sm'
          className='justify-self-start'
          aria-label={
            vendor.connected
              ? t('settings.providers.signOutLabel', { provider: name })
              : t('settings.providers.signInLabel', { provider: name })
          }
          onClick={vendor.connected ? signOut : () => void signIn()}
        >
          {vendor.connected ? t('settings.providers.signOut') : t('settings.providers.signIn')}
        </Button>
      )}
    </div>
  )
}

/** `providers` with `vendor`'s endpoint changed by `change`, which keeps its other fields; an
 * endpoint left with none goes. */
function withEndpoint(
  providers: Providers,
  vendor: string,
  change: Partial<NonNullable<Providers[string]>>,
): Providers {
  const endpoint = { ...providers[vendor], ...change }
  const kept = Object.fromEntries(
    Object.entries(endpoint).filter(([, field]) => field !== undefined && field !== null),
  )
  const next = { ...providers }
  if (Object.keys(kept).length) next[vendor] = kept
  else delete next[vendor]
  return next
}

/** The tokens a server's models hold, which reach the preferences when the field loses the focus
 * holding a whole number, or nothing while it's empty. */
function ContextField({
  value,
  onChange,
}: {
  value: number | null
  onChange: (value: number | null) => void
}) {
  const { t } = useTranslation()
  const [draft, setDraft] = useState(value === null ? '' : String(value))
  const parsed = draft.trim() === '' ? null : Number(draft)
  const valid = parsed === null || (Number.isInteger(parsed) && parsed > 0 && parsed < 2 ** 32)
  return (
    <label className='grid gap-1 text-[10px] text-muted-foreground'>
      {t('settings.providers.context')}
      <Input
        type='number'
        inputMode='numeric'
        min={1}
        placeholder={t('settings.providers.contextUnknown')}
        value={draft}
        aria-invalid={!valid}
        className='h-8 text-[12px] text-foreground'
        onChange={(event) => setDraft(event.currentTarget.value)}
        onBlur={() => valid && parsed !== value && onChange(parsed)}
      />
      <span>{t('settings.providers.contextHint')}</span>
    </label>
  )
}

/** A base URL, which reaches the preferences when the field loses the focus holding a URL, or
 * nothing for the vendor's own server. */
function BaseUrlField({ value, onChange }: { value: string; onChange: (value: string) => void }) {
  const { t } = useTranslation()
  const [draft, setDraft] = useState(value)
  const valid = draft === '' || URL.canParse(draft)
  return (
    <label className='grid gap-1 text-[10px] text-muted-foreground'>
      {t('model.baseUrl')}
      <Input
        type='url'
        value={draft}
        aria-invalid={!valid}
        className='h-8 text-[12px] text-foreground'
        onChange={(event) => setDraft(event.currentTarget.value)}
        onBlur={() => valid && draft !== value && onChange(draft)}
      />
    </label>
  )
}
