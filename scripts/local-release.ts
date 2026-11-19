#!/usr/bin/env bun
// Fork only (see FORK.md): builds the Windows release, copies the app into
// `release/` and empties `target`. A Windows release build keeps its
// `projects` and `store` beside the executable, so those folders in
// `release/` are the user's and are never touched here.
// bun run local-release

import { cpSync, mkdirSync, readdirSync } from 'node:fs'
import path from 'node:path'

import { $ } from 'bun'

if (process.platform !== 'win32') {
  throw new Error('local-release copies the Windows layout; use `bun run build` elsewhere')
}

const root = path.join(import.meta.dir, '..')
const built = path.join(root, 'target', 'release')
const out = path.join(root, 'release')

await $`bun run build`.cwd(root)

// The executable and the CEF runtime the build put beside it.
const runtime = /^(koharu\.exe|vk_swiftshader_icd\.json|.+\.(dll|pak|dat|bin))$/
mkdirSync(out, { recursive: true })
try {
  for (const name of readdirSync(built).filter((name) => runtime.test(name))) {
    cpSync(path.join(built, name), path.join(out, name))
  }
  cpSync(path.join(built, 'locales'), path.join(out, 'locales'), { recursive: true })
} catch (error) {
  throw new Error(`couldn't copy into ${out}; close Koharu if it runs from there`, { cause: error })
}
console.log(`Koharu is in ${out}`)

await $`cargo clean`.cwd(root)
