# Fork notes: lightglitch/koharu

This repository is a fork of [koharu-rs/koharu](https://github.com/koharu-rs/koharu).
This file records what the fork adds on top of upstream, where each addition
lives, and how to carry the additions forward when upstream rewrites the code
underneath them.

## Branches

| Branch | Role                                                                                                                              |
| ------ | --------------------------------------------------------------------------------------------------------------------------------- |
| `main` | A byte-identical mirror of `koharu-rs/koharu` `main`. Never commit to it. The **Sync Fork** workflow fast-forwards it every hour. |
| `dev`  | The repository's default branch: upstream plus every fork feature. All fork work lands here.                                      |

Upstream's `AGENTS.md` describes what upstream accepts in a pull request. On
`dev`, the owner has chosen to **keep every feature listed below even where
those rules would reject it**. Examples of rules this overrides:

- "model output fails closed, no repair layer";
- "no second entry point for an operation an existing command covers";
- "the first failure ends the run".

Do not drop or water down a fork feature because of an upstream rule. All the
other rules still apply:

- no AI co-author trailers;
- conventional commits;
- `rg`;
- every string in every locale file;
- no hand-edited generated source.

## CI

- **Sync Fork** (`.github/workflows/sync-fork.yml`, only on `dev`)
  - Scheduled workflows run only from the default branch, so the file lives on `dev`.
  - It calls GitHub's `merge-upstream` API for `main`.
  - It authenticates with the `SYNC_FORK_TOKEN` secret. That is a fine-grained
    personal access token for `lightglitch/koharu` only, with Contents and Workflows
    set to read and write.
  - The built-in `GITHUB_TOKEN` can't be used. Upstream changes files under
    `.github/workflows`, and GitHub refuses that token the workflows permission,
    so the sync fails with HTTP 422. A token with Contents set to read-only fails
    with HTTP 403.
  - The token expires. When the sync starts failing again, renew it first.
- **Every other workflow is disabled** in the Actions settings: Build, Lint, PR,
  Release PR, Release, Sponsors and Test. Release PR fails on the fork because
  upstream's release tags are missing.
  - A workflow file that upstream adds later arrives enabled. Disable it with
    `gh workflow disable "<name>" -R lightglitch/koharu`.

## Fork features

Each entry gives the intent (what to rebuild if the code moves), where the code
lives today, and what upstream does instead.

### Page rail (`packages/koharu`)

Code: `components/editor/PageRailActions.tsx`, `components/editor/GoToPageDialog.tsx`
and `lib/pages.ts`, plus small hooks in `PageRail.tsx`, `CanvasWorkspace.tsx` and
`Editor.tsx`.

- **Reveal the shown page.** The rail scrolls the shown page into view once per
  change of page, for example when Story shows a page. Thumbnail updates during a
  run must not drag the list.
- **Run one page from its row.** A hover button (refresh icon) runs every stage on
  that page. It is disabled while a run is active. Upstream's equivalent is the
  per-stage items in the row's `...` and right-click menu.
- **Header run button.** Runs the whole project, or the selection when more than
  one page is selected. Upstream uses the Run control's scope picker.
- **Header delete button.** Deletes every page, or the multi-page selection,
  after a confirmation that shows the count. A row's Delete on a page inside a
  multi-page selection deletes the whole selection.
- **Keyboard page navigation.** PageUp and PageDown go to the previous and next
  page, Home and End to the first and last, and Ctrl/Cmd+G opens Go to page.
  - The keys do nothing while a text field, button, list, slider, menu or dialog
    has focus.
  - `navigatePages` in `lib/pages.ts` keeps its own focus guard, and
    `CanvasWorkspace.tsx` listens for it on the window, beside upstream's
    `react-hotkeys-hook` shortcuts.
  - The original keys were A, D and G, which now clash with the tool keys: A is
    direct select. Check `shortcuts` and `toolKeys` in `lib/shortcuts.ts` before
    choosing keys.

### Layers panel (`packages/koharu`)

Code: `components/editor/LayersHeader.tsx` and
`components/editor/ReprocessLayerButton.tsx`, plus hooks in `LayersPanel.tsx`.

- **Reprocess one text block.** A hover button on a block row runs
  `runPipeline({ blocks: [id] }, ['recognize', 'translate'])`. It is hidden while
  a run is active and on locked blocks.
- **Show or hide every layer.** A header toggle covers the whole layer tree and
  skips locked layers. The page artwork isn't a layer, so it stays visible for
  comparing against the original.
- **Delete every text layer.** A header button, after a confirmation with the
  count, deletes the unlocked text in one edit, so one Undo restores it.

### Interface (`packages/koharu`)

- **The browser context menu over selectable text** (`preventBrowserMenu` in
  `app/providers.tsx`). Upstream hides the menu everywhere but text fields; the
  fork keeps it over `.select-text` too, so native Copy still works there, and
  everywhere in development builds, for Inspect.
- **Detection thresholds show their defaults** (`PipelinePreferences.tsx`, the
  `detector_thresholds` command in `commands/preferences.rs`,
  `Detector::thresholds` in `koharu-pipeline`, `default_thresholds` in
  `koharu_layout_highres`). Under each threshold, Settings shows the value an
  empty one stands for, as the detector loads it: the checkpoint's recommended
  thresholds, but Koharu's own for sound effects and bubbles. Upstream only
  says "default".
- **The Activity panel scrolls** (`components/editor/ActivityCenter.tsx`). Rows
  scroll under the title past a fixed height, so many failures don't grow the
  panel over the canvas.

### Windows wheel tools

- `crates/koharu-desktop/src/wheel.rs` adds Koharu's own tools to the Windows
  wheel's menu (a Surface Dial, or a laptop dial such as the ASUS Dial, which
  shows up to Windows as a system multi-axis controller), next to Windows'
  scroll, volume and zoom. It uses `RadialController` through
  `IRadialControllerInterop::CreateForWindow` on the main window, made on the
  main thread, with a haptic click every 15°.
- The interface names the tools in its language through the `show_wheel_tools`
  command and acts on the `wheel` event (`components/editor/WheelTools.tsx`):
  - **Pages:** turn for the previous or next page, press for Go to page.
  - **Canvas zoom:** turn to zoom by 10% a notch, press to fit the page.
  - **Brush size:** turn to resize the brush by 10% a notch, press to swap
    Brush and Eraser.
- Needs the `windows` crate's `Foundation`, `Foundation_Collections`,
  `UI_Input` and `Win32_UI_Input_Radial` features on `koharu-desktop` only.
- The controller is released when the main window closes (`wheel::close`,
  called from `WindowEvent::Destroyed` in `screen/mod.rs`), and never by the
  thread-local cleanup at exit. That cleanup runs after the process has
  stopped the threads that answer COM, so releasing it there waited forever,
  and `koharu.exe` stayed in Task Manager after every close of an open
  project, holding its files.

### Numpad navigation keys

- `crates/koharu-desktop/src/screen/keys.rs` `windows_key_code` sends a numpad
  key with Num Lock off as the navigation key it acts as (Page Up, Page Down,
  Home, End, the arrows, Insert, Delete). Upstream identifies it by position
  only, so it reached the interface as a digit, and laptops whose Page Up and
  Page Down live on the numpad couldn't page.

### Add text from a marked area

- When detection misses text, select it with the Marquee or the Lasso and click
  **Add text** in the area bar (`components/editor/AddTextButton.tsx`, next to
  Generative Fill).
- The `add_text_from_area` command (`commands/text_area.rs`) adds a block
  whose source is the area's box and whose mask is the area, as one undo step.
- The interface then runs the Run control's ticked stages after detection
  (OCR, clean-up, translation; all three when none is ticked) on that block
  only.
- Upstream's marked area only feeds Generative Fill. Since 0.95 upstream has
  its own **Text block** tool (`koharu-canvas` `tools/block.rs`): it drags a
  rectangle, links the block to the balloon under it, and leaves reading and
  clean-up to the block's Story menu. Add text keeps any marked shape as the
  mask and runs the stages itself, so both stay.

### Format Painter

- A toolbar tool (paint roller, key `P`, before the Hand) that copies one
  text's style onto others, as Word's Format Painter and InDesign's
  Eyedropper do with type. Upstream has none.
- The canvas (`koharu-canvas` `tools/painter.rs`, `Tool::FormatPainter`)
  only finds the text under a press, a block's translation among them, and
  outlines it on hover. It answers `Effect::Painter(PainterPress { layer,
  reload })`, `reload` with Alt, which the desktop sends as the `painter`
  event.
- The interface (`lib/formatPainter.ts`) holds the copied style: the first
  press copies, the next ones paste, Alt-click copies afresh. A paste is
  `pasteStyle` in `lib/scene.ts`, the type bar's `setStyle` edit under its
  own undo label, `paste_style`.
- It copies what every character shares (`sharedChars` in `TypeBar.tsx`) of
  font, weight, italic, size, fill, outline and its width, and emphasis,
  plus alignment, line height, and kernel. A value the source leaves unset
  is cleared on the target, so it follows its style as the source does. The
  box, its position, the words, and vertical orientation stay.
- `FormatPainterBar.tsx`, at the canvas's foot while the tool is on, says
  what to click, then shows the style held as an "Aa" preview, with
  **Apply to selected (N)** and a × (or Esc) to drop it. Another tool drops
  it too, except the Hand, which Space holds for a moment.

### Translate a book from the command line

- `koharu --input x.cbz --output y.cbz [--format png|jpeg|webp]
  [--stages detect,ocr,inpaint,translate]` translates a book without the
  window (`crates/koharu-desktop/src/convert.rs`, flattened into the `Cli` in
  `crates/koharu/src/main.rs`).
  - It reads the input as the import does (`commands/import.rs` `pages`).
  - It runs the pipeline on every page with the saved settings, in a project in
    the temporary folder that is deleted afterwards.
  - It writes the CBZ as the export does (`commands/export.rs` `write`), at the
    quality in Settings.
- `--project NAME` works on one of the app's projects instead, in the same
  projects folder (`projects()` in `lib.rs`), and keeps what the stages did,
  as the app does. `--output` is optional with it.
  - With `--input`, it makes that project of the book first; the project
    mustn't exist yet.
  - Without, the project must exist, and mustn't be open in the app, which
    locks it.
  - `--project NAME --stages validate` validates a book translated in the app.
    An exported CBZ can't be validated: its pages are flat images.
- `--stages` defaults to all but `validate`. `ocr` also answers to
  `recognize`, and `inpaint` to `clean`.
- A failed page is reported, the CBZ is still written, and the exit code is 1.
- Windows starts `koharu.exe` as a windowed app, so PowerShell doesn't wait
  for it. Use `koharu.exe ... | Out-Null`.
- Run it from the release build in `release/`, which has the models in
  `release\store`. It loads models into the same GPU memory as an open app.

### Outlines over dark art

- `outline()` in `crates/koharu-ml/src/koharu_layout_highres/measure.rs`
  measures a lettering's outline from rings of pixels around its glyphs.
  Lettering over hair or a tone has that art, as dark as its ink, inside its
  mask, so upstream measured the rings from the art's edge and found no
  outline, and the translation had none either.
- Since 0.96 upstream first looks for outlined lettering by its glyphs
  (`outlined.rs`), and measures its outline there; `outline()` is the
  fallback where it finds none, and that's where the fork's changes apply.
  `outlined()` calls `outline()` with `in_bubble` set, which keeps
  upstream's own rule for the glyphs it tuned against it.
- The fork leaves out of the glyphs the ink-coloured areas that reach the
  measuring window's edge, which run on past the lettering, and the specks
  smaller than (size / 12)² pixels, a tone's dots. An outline keeps the
  glyphs apart from both; glyphs touching the art have none to find.
- Upstream also wants the band to end sharply all round. Lettering on light
  paper with a few lines, or crossing paper in one place and art in another,
  has paper of the band's colour past most of it, which hides the art the
  band keeps out elsewhere. The fork also asks the rings cell by cell, about
  a glyph across: where there's art past a cell's band, the band there must
  hold its colour and keep out at least half of it, and most such cells
  agreeing is an outline.
  - Its width is the narrowest quarter of those cells' bands, capped at 1.5
    of the glyphs' strokes, since a band whose paper matches it ends only at
    the art.
  - Lettering in a bubble keeps to upstream's rule: the bubble's border, close
    by with paper between, reads as a band.

### Validation stage

- A fifth pipeline stage, **Validate**, after Translate
  (`crates/koharu-pipeline/src/stages/validate/`). It measures each visible
  translation as the renderer draws it and flags:
  - text too small (under `min_size` of the page height, or dialogue under
    `small` × the page's median dialogue size) or too big (dialogue over
    `big` × the median; sound effects never);
  - text that overflows its box;
  - a sound effect over dialogue, or dialogue over dialogue, measured on
    each line's letters, not the boxes (upstream's `ResolvedText::lines`,
    which replaced the fork's own `strips`), turned with the text;
  - text off the page or out of its balloon;
  - a runaway translation: a word repeated 8 times in a row, or a long text
    ten times its source's length. Only retranslating fixes it, so the model
    never sees it;
  - text without an outline in low contrast with what's behind its letters,
    or with art as dark (or light) as itself under or within 3 pixels of
    its letters (`busy`, 1% by default; a clean balloon measures 0). The
    letters are the text drawn alone, and what's behind them the page drawn
    without its translations (`Frame::without`), so black line art isn't
    mistaken for black letters.
- **Fixes** (`stages/validate/fix.rs`): while something's flagged and
  `max_iterations` rounds are left (3 by default, 0 only measures), a model is
  shown the page as drawn and its texts, and answers per flagged text, held to
  a schema: `accept`, a `size`, a `box` in page pixels, or an `outline`.
  Koharu colours the outline unlike the letters (white round dark ones) at an
  eighth of their size; a model left to choose outlined black text in black.
  - Each fix is checked first: sizes are held to the page's range, boxes to
    the page, and a dialogue box to the area its balloon safely holds (1/√2
    of the balloon's bounds).
  - All of a round's fixes are one undo step, labelled `validate`. The page is
    then measured again. The model keeps the page's earlier rounds, without
    their images, so it tries something else when a fix didn't help.
  - **A fix never makes a text worse.** When the next measure finds a problem
    a changed text, or a text it moved onto, didn't have, that text's fix is
    undone (`Round::worse`, `undo`) and the model is told why. A fix that left
    its text flagged as before is reported to the model as having no effect,
    and a round that changed no flag ends the loop.
  - A text in a balloon, dialogue or a sound effect, is kept inside it.
  - The model is `[pipeline.validation] model`, or the translator's when
    empty, with the translator's reasoning. It must read images and be
    hosted. With a model of Koharu's own, validation only measures.
- What's still flagged goes out as `Event::Flagged` and stays in Activity
  until dismissed (`FlaggedPages` in `ActivityCenter.tsx`). A click shows the
  page with the text selected. The CLI prints them.
- Thresholds are in `[pipeline.validation]` (`ValidationConfig`).
- It's never part of a default run (`defaultStages` in `lib/store.ts`): tick
  it in the Run control, use a page's Validate menu item, or pass
  `--stages ...,validate` to the CLI.
  - The Run control remembers its ticked stages between sessions (upstream's
    `persist` in `lib/store.ts`), so a ticked Validate stays ticked. A first
    start uses `defaultStages`, not upstream's `pipelineStages`.
- "Out of its balloon" tests sampled points of each line's letters against the
  balloon's outline, so an oval's corners count as outside.

### Sentry is off

- `crates/koharu/src/sentry.rs` starts the Sentry client without a DSN and
  without the minidump integration, so nothing is reported and no crash reporter
  runs, whatever `SENTRY_DSN` a build has. Upstream reports everything to its
  Sentry project.
- The interface's Send Feedback menu item still opens its form, but the report goes
  nowhere.

### Pipeline, Activity and translation (Rust)

- **Keep running when a page fails** (`crates/koharu-pipeline`: `scheduler/mod.rs`
  `done()` and `leave()`, `scheduler/run.rs` `Run::abandon`, `event.rs`).
  - Since 0.99 upstream goes past a page whose translation the model blocks
    or can't answer (`stages::Failed`), and ends the run at any other
    failure. The fork treats every failure as the page's alone, so
    `Failed` only matters upstream; upstream's `fail()` and
    `Outcome::Failed { page, stage, reason }` are replaced by the fork's.
  - The interface keeps the fork's `Job.failures` (page, stage, reason),
    not upstream's `Job.failed` and `Job.error`.
  - A failed stage drops only that page's remaining jobs, and the run goes on.
  - The translation chain is bridged over the failed page, so the next page's
    translation still waits for the one before it.
  - A new `Event::Failed` reports each page at once, and `Outcome::Failed` lists
    every failed page.
  - Activity lists the failed pages while the run goes on (`ActivityCenter.tsx`,
    `Job.failures` in `lib/store.ts`).
  - The pipeline README section 5 and the processing and rendering docs describe
    this. Upstream ends the run at its first failure.
- **Translator repair** (`crates/koharu-pipeline/src/stages/translate/repair.rs`,
  hooked in `turn.rs` `Turn::read`, which `Turn::ask` calls where upstream
  fails the page with `stages::Failed`). Upstream's own retries (without the
  history, halved, warmer) run first, and upstream now also finds JSON in a
  code fence or prose (`json_in_text`) before the repair is needed.
  - It runs only when the strict parse refuses an answer, and recovers:
    - a key whose quote swallowed the colon;
    - unescaped quotes inside a value;
    - a missing comma between strings;
    - fenced or prose-wrapped JSON;
    - a cut-off answer.
  - A text it can't find keeps its source. Only an answer with none of the
    page's texts fails, quoting the answer on one line.
  - A repaired answer enters the conversation history as clean JSON of the
    texts it gave.
  - Upstream fails the page.
- **Copy an error from Activity** (`components/editor/CopyButton.tsx`, and the
  `copy_text` command in `crates/koharu-desktop/src/commands/clipboard.rs`).
  - Each failed page, a whole failed run and a failed download copy to the system
    clipboard through the desktop, not the webview's clipboard API.
- **CBZ page format, quality, progress and stop**
  (`crates/koharu-desktop/src/commands/export.rs`: `PageEncoding`, `ExportConfig`,
  `stop_export`).
  - CBZ pages can be PNG, JPEG (flattened onto white) or WebP.
  - An `[export]` settings section holds `jpeg_quality` (default 90) and
    `webp_quality` (default 85), each from 1 to 100. The UI is
    `components/preferences/ExportPreferences.tsx`, on a Settings tab.
  - The format is chosen in `components/app/ExportCbzDialog.tsx`, which the
    title bar holds open (`cbzPages` in `TitleBar.tsx`) for upstream's
    **Export** and **Export selected** menus alike.
  - Upstream's PDF export and its selected pages (0.95) go through the same
    `write`, so a PDF shows its progress and stops too, and one left
    unfinished is deleted.
  - Activity shows the pages written, and Stop ends the export
    (`lib/exportProgress.ts`).
  - A CBZ that is stopped or fails is deleted.
  - Upstream writes PNG-only CBZs that can't be followed or stopped.
- **AVIF import** (`crates/koharu-desktop/Cargo.toml` enables `image`'s
  `avif-native`; `"avif"` is in `EXTENSIONS` and `raster()` in
  `commands/import.rs`).
  - It is enabled on the desktop crate only, so the other crates build without
    dav1d. See [AVIF import needs dav1d](#avif-import-needs-dav1d).
- **Not carried: the page's file name in Activity.** Pages on upstream have no
  name any more (`PageSummary` has no label), so Activity says "Page N" only.

### AVIF import needs dav1d

`avif-native` links dav1d, a system library, so building `koharu-desktop` needs
dav1d 1.3.0 or later installed:

- Linux: `sudo apt install libdav1d-dev`
- macOS: `brew install dav1d`
- Windows: there is no pkg-config.
  1. Install dav1d with vcpkg: `vcpkg install dav1d:x64-windows-static-md`.
     That links it statically with the dynamic CRT Rust expects.
  2. Set these user environment variables:

     ```
     SYSTEM_DEPS_DAV1D_NO_PKG_CONFIG=1
     SYSTEM_DEPS_DAV1D_LIB=dav1d
     SYSTEM_DEPS_DAV1D_SEARCH_NATIVE=C:/vcpkg/installed/x64-windows-static-md/lib
     ```

  `NO_PKG_CONFIG` is required. Without it, `dav1d-sys` still runs pkg-config,
  and its build script panics.

The fork's CI workflows are disabled. Any workflow that builds `koharu-desktop`
would need dav1d installed first: the apt step on Linux, Homebrew on macOS, and
vcpkg plus the variables above on Windows.

### Local release build (Windows)

- `bun run local-release` (`scripts/local-release.ts`) runs `bun run build`
  and copies `koharu.exe` and the CEF runtime beside it into `release/`
  (ignored by Git). `target` stays as the build cache, so the next build only
  compiles what changed.
- It then runs `cargo sweep --time 14`, which deletes build files untouched
  for 14 days, so `target` doesn't grow without end. Install it once with
  `cargo install cargo-sweep`; without it the script only warns. `cargo clean`
  empties `target` completely, while no dev Koharu runs.
- A Windows release build keeps its projects and models in `projects` and
  `store` beside the executable, not in `%LOCALAPPDATA%` as the dev build does.
  The script never touches those two folders, so rebuilding keeps them.
- Close a Koharu started from `release/` before running it; the copy fails
  otherwise.
- Its update prompt offers upstream's release, without the fork features.
  Decline it.

### Already upstream (nothing to carry)

These went upstream from the fork or were made unnecessary by upstream:

- the selection count in the rail header (#1096);
- the CBZ export core (#1095);
- the Activity page label (#1041, later trimmed to "Page N");
- the translator's error diagnostics (#1022);
- the detection crash on a region a pixel across (imageproc patched upstream
  in 0.86.4, `daaa6b5e`; the fork's `connected_components` wrapper was dropped);
- an OpenAI-compatible server's context window in Settings (upstream
  `83ec928e`, as `context_window`; the fork's `context` was dropped);
- re-detecting a page whose text was deleted (detection now always replaces a
  page's blocks).

## Rebuilding after a large upstream rewrite

Use this when merging `main` into `dev` produces conflicts across many files
because upstream moved or deleted the code a feature hooks into.

1. **Update both branches.**
   - `git fetch origin`
   - `git switch dev && git pull --ff-only`
   - `git fetch origin main:main`
2. **Try the merge.** Run `git merge main`. If it resolves cleanly, or with a few
   local conflicts, you're done. If the conflicts are in code upstream rewrote,
   run `git merge --abort` and continue.
3. **Back up `dev`.** `git branch dev-before-<version>-rebuild dev`
4. **Cut a new branch from `main`.** `git switch -c dev-rebuild main`, then
   cherry-pick the CI commits (the sync workflow), which apply cleanly.
5. **Triage every fork commit.** List them with `git log --no-merges main..dev`.
   For each one:
   - read the old diff for intent: `git show <sha> -- ':!*/locales/*'`;
   - find the code's new owner on `main` with `rg`;
   - check `git log main -i --grep=<keyword>` for an upstream version.

   Sort it into one of three outcomes:
   - already upstream, so drop it;
   - nothing left to fix, so drop it;
   - rebuild it on the new owner.

   Upstream rule conflicts are **not** a reason to drop a feature here.

6. **Rebuild.** Use one commit per feature.
   - Put fork code in its own files, with small hooks into upstream files, so the
     next merge conflicts as little as possible.
   - When a feature reverses documented upstream behaviour, update that
     documentation in the same commit, and mark the divergence with a comment.
   - Rust work goes in one tree, one feature at a time. A cold
     `cargo check -p koharu-pipeline` takes about 8 to 9 minutes, and parallel
     worktrees each start cold.
   - TypeScript-only work can run in parallel worktrees. Expect conflicts in the
     locale files when merging those branches.
7. **Update this file.** Every feature that moved gets its new location here.
8. **Replace `dev`.** Review the whole branch. Then either reset `dev` to
   `dev-rebuild` and force-push, or open a pull request. Keep the backup branch
   until the new `dev` has been used for a while.

### Things that bite

- **Generated IPC.** `packages/ipc/src/protocol.ts` is generated. Change the Rust
  types, then run `cargo run -p koharu-desktop --bin ipc` and `bun run format`.
- **Locale files.** The app has 12 at the time of writing:
  `packages/koharu/public/locales/*`. Every string goes in all of them. Strings
  for locales that upstream added after a feature was written are machine
  translations and need review.
- **Specta and serde attributes.** A field with `#[serde(deserialize_with = ...)]`
  also needs `#[specta(type = ...)]`, or the IPC generator refuses it.
- **Stale crate folders.** An empty leftover folder from a deleted crate (for
  example `crates/koharu-app/`) breaks `cargo` with "failed to load manifest for
  workspace member". Delete the empty folder.
- **Installing dependencies.** Use `bun install --ignore-scripts` for TypeScript
  checks. The `postinstall` builds `tauri-cli` from source.
- **The TypeScript check.** Run it from `packages/koharu` with
  `../../node_modules/.bin/tsc --noEmit -p .`
- **AI sessions.** Upstream's `AGENTS.md` steers AI sessions toward dropping these
  features. Add `@FORK.md` next to `@AGENTS.md` in your `CLAUDE.md` so that a
  session knows `dev` keeps them. `CLAUDE.md` is listed in `.git/info/exclude`,
  so each clone needs that line added by hand.
