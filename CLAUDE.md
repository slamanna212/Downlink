# Downlink — instructions for Claude Code

Desktop IPTV client (Dispatcharr, M3U/XMLTV, Xtream). Tauri 2 + Rust workspace, React 19.3 + TypeScript
+ Vite + Mantine 9 UI, in-process libmpv rendered under a transparent webview.

The plan is `docs/PLAN.md`. It is the source of truth. If code and plan disagree, stop and ask; if a
change invalidates the plan, update the plan in the same PR.

## Work order

- Build one milestone at a time (PLAN §9). Do not start M1 until the M0 go/no-go record exists.
- Read only the plan sections the current task needs.

## Hard rules

1. `downlink-core` never depends on Tauri, wry, React or mpv.
2. The UI never talks HTTP. Every call is a Tauri command returning a typed DTO.
3. `downlink-player` owns every mpv option string (PLAN §4.3). No mpv options in JS or in core.
4. Nothing requests a stream endpoint except an actual play: `/proxy/ts/stream/`, `/proxy/hls/`,
   `/proxy/vod/`, catch-up session URLs, Xtream `/live/` `/movie/` `/series/` `/timeshift/`.
   No preflight, no prefetch, no HEAD. Dwell prefetch is SQLite-only. Status endpoints are fine.
5. Auth headers go to mpv as per-file options, never global. Redirect-profile channels are resolved
   in Rust with redirects disabled, and mpv gets the `Location` URL without headers (PLAN §3.5).
6. Every catch-up, HLS and TS session is stopped/deleted on zap, navigate-away, sleep and quit;
   session ids persist in `open_session` for crash cleanup.
7. Secrets live only in the OS keychain. Never in SQLite, logs, errors or the debug panel.
8. All logging, `AppError` messages and diagnostics pass through `redact/`. Every new URL shape that
   can carry credentials gets a redaction test.
9. UI branches on `Caps`, never on source kind.
10. Adult/radio/hidden/DRM filtering lives only in `catalog/`.
11. Store UTC integers; render through the single `time.ts` module (dayjs utc + timezone). No other
    date math in the UI.
12. Catch-up times come from the programme row. Template time semantics are data, not inferred.
13. No admin features. Dispatcharr's web UI is the admin console.
14. Release artefacts never load a system libmpv; `DOWNLINK_MPV_PATH` is a dev escape hatch only.
15. No `unwrap()`/`expect()` on network or user-input paths.
16. No real provider names, URLs, credentials or EPG in fixtures or tests. Automated tests never hit
    the real server.

## UI conventions

- **Design spec: `docs/design/DESIGN.md`.** Read it before any UI task. Visual reference in
  `docs/design/reference/*.html` (open in a browser). Where they disagree, DESIGN.md wins.
- Mantine 9 theme is `apps/ui/src/theme.ts` (from Claude Design) with `Button.module.css` and
  `global.css` in `apps/ui/src/styles/`. Don't invent tokens; if one is missing, add it to theme.ts
  and DESIGN.md in the same PR.
- `MantineProvider theme={theme} cssVariablesResolver={cssVariablesResolver} defaultColorScheme="dark"`.
- Hex colours only in theme.ts. Components use `var(--dl-*)` tokens or Mantine colour keys
  (`iris`, `live`, `ok`, `warn`, `dark`). Spacing/radius/font sizes from the Mantine scale.
- Accent is Iris violet and means focused / selected / progress only. Red/green/amber are status only.
- Buttons through `<DLButton>` / `<DLIconButton>` wrappers: variants filled | default | subtle | live |
  danger | overlay; sizes xs | sm | md | lg (tv is automatic). One primary per view.
- Every interactive component ships a Storybook story showing rest, hover, pressed, focus, disabled,
  loading.
- html/body/`#root` transparent; opaque background painted on the app shell; player route leaves the
  video rect transparent.
- Lists: `@tanstack/react-virtual`. Guide: absolutely positioned blocks on native scroll, not
  `ScrollArea`, not `@mantine/schedule`.
- Focus: `@noriginmedia/norigin-spatial-navigation`; focus ring always visible (`--dl-focus-ring`,
  applied via `data-focused`).
- Blur only via the theme `allowBlur` flag (false on Linux).
- Icons: `@tabler/icons-react`, stroke 1.75, sizes 14/16/20/24 through an `<Icon>` wrapper.
- All copy through `t()`. Sentence case; caps only in badges and 12 px labels.
- Data: TanStack Query over `invoke`, invalidated by `catalog://updated`; Zustand for player/UI state.

## Quality gates (must pass before a PR)

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo deny check
cargo nextest run --workspace
pnpm -C apps/ui lint && pnpm -C apps/ui typecheck && pnpm -C apps/ui test
pnpm -C apps/ui build
```

`pnpm -C apps/ui lint` includes stylelint (`color-no-hex` outside theme.ts) and the design-rule ESLint
config described in DESIGN.md §11.

## Commits

Conventional Commits (`feat:`, `fix:`, `perf:`, `docs:`, `chore:`). PR body: what, why, how verified,
screenshots for UI, and whether it touches the player surface.
