## What

<!-- What changed. -->

## Why

<!-- Motivation, linked PLAN section or issue. -->

## How verified

<!-- Commands run, tests added, manual checks. Quality gates from CLAUDE.md must pass. -->

- [ ] `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo deny check`, `cargo nextest run --workspace`
- [ ] `pnpm -C apps/ui lint`, `typecheck`, `test`, `build`

## Screenshots

<!-- Required for UI changes (rest, focus, relevant states). Delete if not UI. -->

## Touches the player surface?

- [ ] No
- [ ] Yes: `downlink-player`, mpv options, compositing, or session start/stop. Describe the platforms tested (Windows / macOS / Linux X11 / Wayland).

## Design checklist (docs/design/DESIGN.md §11)

- [ ] No hex colours outside `theme.ts`; tokens (`var(--dl-*)`) or Mantine colour keys only
- [ ] Accent (iris) only for focused / selected / progress; red/green/amber for status only
- [ ] Buttons via `DLButton` / `DLIconButton`; one primary per view
- [ ] Every new interactive component has a Storybook story (rest, hover, pressed, focus, disabled, loading)
- [ ] Focus ring visible via `data-focused`; works with spatial navigation
- [ ] All copy through `t()`, sentence case
- [ ] Not applicable (no UI change)

## Plan

- [ ] Code and `docs/PLAN.md` agree (or the plan is updated in this PR)
