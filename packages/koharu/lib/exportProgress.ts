import { Channel } from '@tauri-apps/api/core'
import { create } from 'zustand'

import type { ExportProgress } from '@koharu/ipc/protocol'

// Fork addition: upstream's export can't be followed or stopped.

/** The export going on's pages written, of its total, for Activity. */
export const useExportProgress = create<{ progress: ExportProgress | null }>(() => ({
  progress: null,
}))

/** A channel for a new export's progress, which starts it over from nothing written. */
export function exportProgressChannel(): Channel<ExportProgress> {
  useExportProgress.setState({ progress: null })
  const channel = new Channel<ExportProgress>()
  channel.onmessage = (progress) => useExportProgress.setState({ progress })
  return channel
}
