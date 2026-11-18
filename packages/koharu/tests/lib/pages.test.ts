import { describe, expect, it } from 'vitest'

import { pageStep } from '@/lib/pages'

describe('pageStep', () => {
  const pages = [{ id: 'a' }, { id: 'b' }, { id: 'c' }]

  it('steps through the project order and stops at either end', () => {
    expect(pageStep(pages, 'b', 1)).toBe('c')
    expect(pageStep(pages, 'b', -1)).toBe('a')
    expect(pageStep(pages, 'c', 1)).toBeUndefined()
    expect(pageStep(pages, 'a', -1)).toBeUndefined()
  })

  it('starts from the first page forward and the last page back when none is shown', () => {
    expect(pageStep(pages, null, 1)).toBe('a')
    expect(pageStep(pages, null, -1)).toBe('c')
    expect(pageStep([], null, 1)).toBeUndefined()
  })
})
