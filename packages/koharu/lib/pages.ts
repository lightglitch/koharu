'use client'

import { create } from 'zustand'

import { commands, type PageId } from '@koharu/ipc/protocol'

import { dispatch } from './backend'
import { projectKey, queryClient, type ProjectView } from './queries'
import { useKoharuStore } from './store'

/** Whether the Go to page dialog shows. Its own store, so the fork's navigation stays out of the
 * editor's. */
export const useGoToPage = create<{ open: boolean; setOpen: (open: boolean) => void }>()((set) => ({
  open: false,
  setOpen: (open) => set({ open }),
}))

/** Shows a page and makes it the selection, as a plain click on its rail row does. */
export function showPage(page: PageId): void {
  const store = useKoharuStore.getState()
  store.selectPages([page])
  if (page === queryClient.getQueryData<ProjectView | null>(projectKey)?.page) return
  store.selectLayers([])
  dispatch(commands.show, page)
}

/** The page `delta` steps from the shown one, or undefined past either end: paging past the last
 * page should not quietly return to the first. With none shown, forward starts at the first page
 * and back at the last. */
export function pageStep(
  pages: readonly { id: PageId }[],
  shown: PageId | null | undefined,
  delta: number,
): PageId | undefined {
  const index = pages.findIndex((page) => page.id === shown)
  if (index < 0) return (delta > 0 ? pages[0] : pages[pages.length - 1])?.id
  return pages[index + delta]?.id
}

/** Focused elements that give the navigation keys meanings of their own: text fields, buttons,
 * lists, sliders, menus, tabs, splitters, and open dialogs. A plain scroll container such as the
 * rail's, which a click on a row focuses, isn't one: paging the pages is what its keys are for. */
const controls = [
  'input',
  'textarea',
  'select',
  'button',
  'a[href]',
  '[contenteditable]',
  ...[
    'button',
    'link',
    'checkbox',
    'radio',
    'switch',
    'listbox',
    'option',
    'slider',
    'spinbutton',
    'scrollbar',
    'separator',
    'menu',
    'menubar',
    'menuitem',
    'menuitemcheckbox',
    'menuitemradio',
    'tab',
    'tablist',
    'combobox',
    'tree',
    'treeitem',
    'grid',
    'dialog',
    'alertdialog',
  ].map((role) => `[role="${role}"]`),
].join(',')

/** Handles the page navigation keys, reporting whether `event` was one: Page Up and Page Down
 * step through the pages, Home and End go to the first and last, Ctrl+G asks for a page number.
 * They never act while a control has focus. */
export function navigatePages(event: KeyboardEvent): boolean {
  if (event.defaultPrevented) return false
  if (event.target instanceof Element && event.target.closest(controls)) return false
  const project = queryClient.getQueryData<ProjectView | null>(projectKey)
  if (!project || project.pages.length === 0) return false
  const { pages, page: shown } = project
  const command = event.ctrlKey || event.metaKey

  if (command && !event.shiftKey && !event.altKey && event.key.toLowerCase() === 'g') {
    event.preventDefault()
    useGoToPage.getState().setOpen(true)
    return true
  }
  if (command || event.shiftKey || event.altKey) return false
  const next =
    event.key === 'PageDown'
      ? pageStep(pages, shown, 1)
      : event.key === 'PageUp'
        ? pageStep(pages, shown, -1)
        : event.key === 'Home'
          ? pages[0]?.id
          : event.key === 'End'
            ? pages[pages.length - 1]?.id
            : null
  if (next === null) return false
  event.preventDefault()
  if (next !== undefined) showPage(next)
  return true
}

/** Shows the page `steps` from the shown one, stopping at the first and the last, as a turn of the
 * wheel's Pages tool asks: a fast turn past either end stays there. */
export function showPageBy(steps: number): void {
  const project = queryClient.getQueryData<ProjectView | null>(projectKey)
  if (!project || project.pages.length === 0) return
  const { pages, page: shown } = project
  const index = pages.findIndex((page) => page.id === shown)
  const from = index < 0 ? (steps > 0 ? -1 : pages.length) : index
  const next = pages[Math.min(pages.length - 1, Math.max(0, from + steps))]
  if (next && next.id !== shown) showPage(next.id)
}
