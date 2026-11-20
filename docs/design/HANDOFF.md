# Downlink design handoff

Drop the contents of this folder into the repo root. Paths already match the repo layout.

```
CLAUDE.md                              replaces repo CLAUDE.md (adds design rules to "UI conventions")
docs/design/DESIGN.md                  the spec Claude Code follows
docs/design/reference/
  design-language-v1.html              foundations
  design-language-v2.html              colour, buttons, rules
  screens.html                         12 screens incl. multiview
apps/ui/src/theme.ts                   Mantine 9 theme + CSS variables + type augmentation
apps/ui/src/styles/Button.module.css   button/action-icon variants and states
apps/ui/src/styles/global.css          fonts, focus ring, tabular numerals, transparency
```

Setup in `apps/ui`:
```bash
pnpm add @fontsource/hanken-grotesk @fontsource/geist-mono @tabler/icons-react
pnpm add -D postcss-preset-mantine postcss-simple-vars stylelint stylelint-config-standard
```
```tsx
// main.tsx
import '@mantine/core/styles.css';
import './styles/global.css';
import { MantineProvider } from '@mantine/core';
import { theme, cssVariablesResolver } from './theme';

<MantineProvider theme={theme} cssVariablesResolver={cssVariablesResolver} defaultColorScheme="dark">
  <div className="dl-shell" data-allow-blur={String(theme.other.allowBlur)}>…</div>
</MantineProvider>
```
Set `data-density="compact|comfortable|tv"` on `<html>` from the density setting.

Suggested first prompt for Claude Code:
> Read CLAUDE.md and docs/design/DESIGN.md. Wire theme.ts, global.css and Button.module.css into
> apps/ui, build `<DLButton>`, `<DLIconButton>`, `<Icon>`, `<ChannelLogo>` (with deterministic fallback)
> and `<StatusBadge>` with Storybook stories covering every state, and add the stylelint/ESLint rules
> from DESIGN.md §11. No feature screens yet.

Notes:
- Multiview is in 1.0 (PLAN §1.5, §4.11).
- Light theme, Movies/Series screens and the command palette aren't designed yet.
- Verify Mantine 9 API names (variant augmentation, `focusRing`, `autoContrast`) against the installed
  version; they're written against the Mantine 7/8 API.
