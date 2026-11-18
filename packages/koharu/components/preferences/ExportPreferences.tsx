'use client'

import { useTranslation } from 'react-i18next'

import {
  NumberField,
  PreferencePage,
  PreferenceRow,
  PreferenceSection,
} from '@/components/preferences/PreferenceFields'
import type { ExportConfig } from '@koharu/ipc/protocol'

/** The `[export]` section: the quality of a CBZ's lossy pages, which the backend holds to 1–100. */
export function ExportPreferences({
  value,
  onChange,
}: {
  value: ExportConfig
  onChange: (value: ExportConfig) => void
}) {
  const { t } = useTranslation()
  const qualities = [
    ['jpeg_quality', 'jpegQuality'],
    ['webp_quality', 'webpQuality'],
  ] as const

  return (
    <PreferencePage
      title={t('settings.export.title')}
      description={t('settings.export.description')}
    >
      <PreferenceSection
        title={t('settings.export.quality')}
        description={t('settings.export.qualityDescription')}
      >
        {qualities.map(([field, label]) => (
          <PreferenceRow
            key={field}
            title={t(`settings.export.${label}`)}
            description={t('settings.export.qualityHint')}
          >
            <NumberField
              label={t('settings.export.qualityLabel')}
              value={value[field] ?? null}
              min={1}
              max={100}
              step={1}
              onChange={(quality) => {
                if (quality !== null) onChange({ ...value, [field]: Math.round(quality) })
              }}
            />
          </PreferenceRow>
        ))}
      </PreferenceSection>
    </PreferencePage>
  )
}
