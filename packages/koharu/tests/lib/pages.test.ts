import { describe, expect, it } from 'vitest'

import { pageStep } from '@/lib/pages'

const pages = [{ id: 'a' }, { id: 'b' }, { id: 'c' }]

describe('page navigation', () => {
  it('steps forward and back through the project order', () => {
    expect(pageStep(pages, 'a', 1)).toBe('b')
    expect(pageStep(pages, 'b', 1)).toBe('c')
    expect(pageStep(pages, 'c', -1)).toBe('b')
  })

  it('stops at either end rather than wrapping', () => {
    // Wrapping would take a reader from the last page back to the cover
    // without asking, which is not what paging through a book means.
    expect(pageStep(pages, 'c', 1)).toBeUndefined()
    expect(pageStep(pages, 'a', -1)).toBeUndefined()
  })

  it('starts from the near end when nothing is active', () => {
    expect(pageStep(pages, null, 1)).toBe('a')
    expect(pageStep(pages, null, -1)).toBe('c')
    expect(pageStep(pages, 'missing', 1)).toBe('a')
  })

  it('has nowhere to go in an empty project', () => {
    expect(pageStep([], null, 1)).toBeUndefined()
    expect(pageStep([], 'a', -1)).toBeUndefined()
  })
})
