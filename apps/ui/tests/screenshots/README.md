# Marketing screenshots

```bash
pnpm -C apps/ui screenshots
```

Builds the UI, serves it with the in-browser mock backend in showcase mode (`?mock=showcase`), and writes
1600×1000 @2× PNGs to `apps/ui/screenshots/` (gitignored): `home`, `guide-playing`, `movies`, `now-playing`, `multiview`,
`recordings`.

- Data is the synthetic mock catalogue. Nothing touches the network.
- Posters, backdrops and the video frame are procedural SVG landscapes from `art.ts`. libmpv doesn't run in
  a browser, so the spec paints the frame under the transparent video rect, which is where mpv draws in the app.
- To add or change a shot, edit `screenshots.spec.ts`. Window size is in `playwright.screenshots.config.ts`.
