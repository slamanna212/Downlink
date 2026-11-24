// Downlink — Mantine 9 theme. Source of truth for all visual tokens.
// Spec: docs/design/DESIGN.md · Visual reference: docs/design/reference/*.html
// Rule: hex values live ONLY in this file. Components use var(--dl-*) or Mantine colour keys.
// Decided 2026-11-19: accent = Iris violet, neutrals = cool blue-grey.
import {
  createTheme,
  type ButtonVariant,
  type ActionIconVariant,
  type CSSVariablesResolver,
  type DefaultMantineColor,
  type MantineColorsTuple,
} from '@mantine/core';
import buttonClasses from './styles/Button.module.css';

// ---------- ramps (step 4 = base on dark, step 3 = hover/light, steps 0–2 unused in dark) ----------
const iris: MantineColorsTuple = ['#F1EEFF', '#E0DAFF', '#CDC4FF', '#B5A8FF', '#9D8CFF', '#8571F5', '#6C57E0', '#5442B8', '#3E318A', '#2A215E'];
const live: MantineColorsTuple = ['#FFE9EC', '#FFCCD2', '#FFA3AE', '#FF8B97', '#F2495C', '#DE3549', '#BC2638', '#951D2C', '#6E1621', '#4A0F17'];
const ok: MantineColorsTuple = ['#E4FBF2', '#BFF4DE', '#8EEAC4', '#6EE7B7', '#34D399', '#1FBF84', '#14996A', '#107753', '#0D573D', '#093B2A'];
const warn: MantineColorsTuple = ['#FEF5E4', '#FDE7BF', '#FBD58E', '#F8C565', '#F5B544', '#E09C25', '#B97D17', '#905F12', '#68440D', '#432C08'];
// dark: 0 text · 1 text-2 · 2 text-dim · 3 text-faint · 4 border · 5 surface-3 · 6 surface-2 · 7 surface-0 · 8 letterbox · 9 multiview canvas
const dark: MantineColorsTuple = ['#E8ECF1', '#C3CAD3', '#A3ADB9', '#7C8692', '#2C323A', '#20252B', '#191D22', '#0C0E11', '#08090B', '#050607'];

// Deterministic logo fallback: logoPalette[hash(channelId) % 8], initials in --dl-text.
export const logoPalette = ['#2F4A6B', '#4A3D6E', '#5E3B4C', '#2E5A4F', '#5C4A2A', '#36505F', '#523A5E', '#2C4F63'] as const;

export const focusRing = '0 0 0 2px var(--dl-accent), 0 0 0 6px rgb(157 140 255 / 0.24)';
export const focusRingTv = '0 0 0 3px var(--dl-accent), 0 0 0 9px rgb(157 140 255 / 0.24)';
// Violet is less visible on bright video: add a dark gap ring under the accent.
export const focusRingVideo = '0 0 0 1px rgb(5 6 7 / 0.7), 0 0 0 3px var(--dl-accent), 0 0 0 7px rgb(157 140 255 / 0.30)';

const isLinux = typeof navigator !== 'undefined' && /Linux/.test(navigator.userAgent) && !/Android/.test(navigator.userAgent);

const dlOther = {
    allowBlur: !isLinux,
    motion: { fast: 120, base: 160, route: 240, videoFade: 200, ease: 'cubic-bezier(0.2, 0, 0, 1)' },
    controlsIdleMs: 3000,
    previewDwellMs: 300,
    layout: { railExpanded: 224, railCollapsed: 68, railTv: 96, groupSidebar: 220, settingsNav: 240, infoPanel: 380, titleBar: 36, windowButton: 46, macLights: 76, homeHero: 460, guideHero: 288 },
    /** Channel-change overlay (player): fades this long after the last zap or digit. */
    zapOverlayMs: 3000,
    density: {
      compact: { channelRow: 44, guideRow: 48, logo: [48, 24] as [number, number], control: 32 },
      comfortable: { channelRow: 64, guideRow: 64, logo: [72, 36] as [number, number], control: 36 },
      tv: { channelRow: 88, guideRow: 96, logo: [112, 56] as [number, number], control: 56, typeScale: 1.25, focusRing: 3 },
    },
    guide: { channelColumn: 200, headerHeight: 40, tickMinutes: 30 },
    multiview: { maxTiles: 4, maxTilesCapable: 9, tileGap: 4, layouts: ['2-up', '2x2', '1+3', '3x3'] as const },
    buttonSizes: {
      xs: { h: 28, fz: 12, px: 10, icon: 14, gap: 6, r: 4 },
      sm: { h: 32, fz: 13, px: 12, icon: 16, gap: 6, r: 6 },
      md: { h: 36, fz: 14, px: 16, icon: 16, gap: 8, r: 6 },
      lg: { h: 44, fz: 15, px: 20, icon: 20, gap: 8, r: 8 },
      tv: { h: 56, fz: 18, px: 24, icon: 24, gap: 10, r: 8 },
    },
    iconSizes: [14, 16, 20, 24] as const, // 26 in 10-foot
    iconStroke: 1.75,
    logoPalette,
  };

export type DLOther = typeof dlOther;
export const dl = dlOther;

export const theme = createTheme({
  primaryColor: 'iris',
  primaryShade: { light: 6, dark: 4 },
  colors: { iris, live, ok, warn, dark },
  autoContrast: true,
  luminanceThreshold: 0.3, // filled iris gets --dl-on-accent (#120A33)
  black: '#120A33',
  white: '#F3F6F9',

  fontFamily: '"Hanken Grotesk", system-ui, -apple-system, "Segoe UI", sans-serif',
  fontFamilyMonospace: '"Geist Mono", ui-monospace, "SF Mono", Consolas, monospace',
  headings: {
    fontFamily: '"Hanken Grotesk", system-ui, sans-serif',
    fontWeight: '650',
    sizes: {
      h1: { fontSize: '28px', lineHeight: '1.2' },  // page title
      h2: { fontSize: '22px', lineHeight: '1.25' },
      h3: { fontSize: '17px', lineHeight: '1.3' },
      h4: { fontSize: '15px', lineHeight: '1.35' }, // section heading (600, text-2)
    },
  },
  fontSizes: { xs: '12px', sm: '13px', md: '14px', lg: '15px', xl: '18px' },
  lineHeights: { xs: '1.4', sm: '1.45', md: '1.5', lg: '1.5', xl: '1.4' },
  spacing: { xs: '4px', sm: '8px', md: '12px', lg: '16px', xl: '24px' },
  radius: { xs: '2px', sm: '4px', md: '6px', lg: '8px', xl: '12px' },
  defaultRadius: 'md',
  shadows: {
    xs: 'none',
    sm: '0 1px 2px rgb(0 0 0 / 0.4)',
    md: '0 8px 24px rgb(0 0 0 / 0.45)',  // menus, popovers
    lg: '0 16px 48px rgb(0 0 0 / 0.55)', // modals, floating player panels, multiview toolbar
    xl: '0 24px 64px rgb(0 0 0 / 0.6)',
  },
  focusRing: 'never', // we draw our own ring via :focus-visible / data-focused (global.css)
  cursorType: 'pointer',
  respectReducedMotion: true,

  other: dlOther,

  components: {
    Button: { classNames: buttonClasses, defaultProps: { radius: 'md', size: 'md', variant: 'default' } },
    ActionIcon: { classNames: buttonClasses, defaultProps: { variant: 'subtle', radius: 'md', size: 36 } },
    Badge: { defaultProps: { radius: 'sm', size: 'sm', variant: 'light' } },
    SegmentedControl: { defaultProps: { radius: 'md' } },
    Modal: { defaultProps: { radius: 'xl', centered: true, overlayProps: { backgroundOpacity: 0.6 } } },
    Menu: { defaultProps: { radius: 'lg', shadow: 'md' } },
    Popover: { defaultProps: { radius: 'xl', shadow: 'lg' } },
    Tooltip: { defaultProps: { openDelay: 400, radius: 'sm' } },
    Switch: { defaultProps: { size: 'md' } },
    TextInput: { defaultProps: { radius: 'md', size: 'md' } },
    Select: { defaultProps: { radius: 'md', size: 'md' } },
  },
});

export const cssVariablesResolver: CSSVariablesResolver = () => ({
  variables: {
    // accent (Iris violet)
    '--dl-accent': '#9D8CFF',
    '--dl-accent-hover': '#B5A8FF',
    '--dl-accent-pressed': '#8571F5',
    '--dl-accent-text': '#BDB2FF',
    '--dl-on-accent': '#120A33',
    '--dl-accent-wash': 'rgb(157 140 255 / 0.06)',   // playing row, audio tile
    '--dl-accent-banner': 'rgb(157 140 255 / 0.10)', // banners, progress fill in blocks
    '--dl-accent-tint': 'rgb(157 140 255 / 0.14)',   // accent badges, toggled icon buttons
    '--dl-accent-glow': 'rgb(157 140 255 / 0.24)',   // focus ring halo only
    '--dl-accent-bar': 'rgb(157 140 255 / 0.55)',    // info panel buffer graph bars only
    // status
    '--dl-live': '#F2495C',
    '--dl-live-text': '#FF8B97',
    '--dl-live-tint': 'rgb(242 73 92 / 0.18)',
    '--dl-live-tint-hover': 'rgb(242 73 92 / 0.24)',
    '--dl-live-tint-pressed': 'rgb(242 73 92 / 0.30)',
    '--dl-live-banner': 'rgb(242 73 92 / 0.10)',     // error banners (10% tier)
    '--dl-danger-border': 'rgb(242 73 92 / 0.40)',
    '--dl-danger-hover': 'rgb(242 73 92 / 0.10)',
    '--dl-danger-pressed': 'rgb(242 73 92 / 0.16)',
    '--dl-ok': '#34D399',
    '--dl-ok-text': '#6EE7B7',
    '--dl-ok-tint': 'rgb(52 211 153 / 0.10)',
    '--dl-warn': '#F5B544',
    '--dl-warn-tint': 'rgb(245 181 68 / 0.10)',
    '--dl-notice': '#9D8CFF',                         // "update waiting" dot (rail, settings nav) only
    // interaction layers (white over any surface)
    '--dl-hover': 'rgb(255 255 255 / 0.06)',
    '--dl-pressed': 'rgb(255 255 255 / 0.10)',
    '--dl-focus-wash': 'rgb(255 255 255 / 0.04)',
    '--dl-outline-badge': 'rgb(255 255 255 / 0.18)', // outline badges (NEW/HD/4K/RADIO)
    '--dl-focus-ring': focusRing,
    '--dl-focus-ring-tv': focusRingTv,
    '--dl-focus-ring-video': focusRingVideo,
    // third-party brand: Plex mark only (DESIGN §2)
    '--dl-plex': '#E5A00D',
    // first-sync Uplink scene only (DESIGN §12): the app icon's night sky, sparkles and planet shades
    '--dl-uplink-night': '#120A33',
    '--dl-uplink-mount': '#5442B8',
    '--dl-uplink-crater': '#6C57E0',
    '--dl-sparkle-cyan': '#7FE3E0',
    '--dl-sparkle-blue': '#9CC8FF',
    // video
    '--dl-on-video': '#F3F6F9',
    '--dl-scrim': 'linear-gradient(to top, rgb(5 6 7 / 0.92), rgb(5 6 7 / 0.6) 55%, rgb(5 6 7 / 0))',
    '--dl-glass': 'rgb(12 14 17 / 0.72)',        // tile labels (with blur)
    '--dl-glass-solid': 'rgb(12 14 17 / 0.88)',  // same, when allowBlur is false
    '--dl-video-btn': 'rgb(255 255 255 / 0.10)',
    '--dl-video-btn-hover': 'rgb(255 255 255 / 0.16)',
    '--dl-video-btn-pressed': 'rgb(255 255 255 / 0.22)',
    '--dl-tile-btn': 'rgb(12 14 17 / 0.78)',          // multiview tile actions: solid enough to read on any picture
    '--dl-tile-btn-hover': 'rgb(32 37 43 / 0.9)',
    '--dl-tile-btn-pressed': 'rgb(44 50 58 / 0.92)',
    // player overlay (DESIGN §8): heavier bottom scrim, light top scrim under the title bar
    '--dl-player-scrim': 'linear-gradient(to top, rgb(5 6 7 / 0.94), rgb(5 6 7 / 0.8) 45%, rgb(5 6 7 / 0))',
    '--dl-player-top-scrim': 'linear-gradient(to bottom, rgb(5 6 7 / 0.7), rgb(5 6 7 / 0))',
    '--dl-tile-scrim': 'linear-gradient(to top, rgb(5 6 7 / 0.85), rgb(5 6 7 / 0))', // guide mini player controls
    // window chrome
    '--dl-titlebar-close': '#E5484D', // close button hover only
    '--dl-on-titlebar-close': '#FFFFFF',
    '--dl-scrollbar-thumb': 'rgb(232 236 241 / 0.22)',
    '--dl-scrollbar-thumb-hover': 'rgb(232 236 241 / 0.36)',
    '--dl-ease': 'cubic-bezier(0.2, 0, 0, 1)',
  },
  light: {},
  dark: {
    '--dl-surface-0': '#0C0E11',
    '--dl-surface-1': '#121519',
    '--dl-surface-2': '#191D22',
    '--dl-surface-3': '#20252B',
    '--dl-surface-3-hover': '#2C323A',
    '--dl-surface-3-pressed': '#262C33',
    '--dl-row-focus': '#1A1E23',
    '--dl-overlay': 'rgb(12 14 17 / 0.82)',
    '--dl-border': '#2C323A',
    '--dl-hairline': 'rgb(255 255 255 / 0.06)',
    '--dl-hairline-strong': 'rgb(255 255 255 / 0.08)',
    '--dl-text': '#E8ECF1',
    '--dl-text-2': '#C3CAD3',
    '--dl-text-dim': '#A3ADB9',
    '--dl-text-faint': '#7C8692',   // ≥ 12px caps labels and column headers only
    '--dl-text-disabled': '#5F6873',
    '--dl-guide-past': '#161A1F',
    '--dl-guide-now': '#222830',
    '--dl-guide-future': '#191D22',
    '--dl-guide-focus': '#2A3140', // focused guide block (no ring)
    // heroes (home, guide): fade programme art into surface-0 from the left, bottom and top
    '--dl-hero-scrim': 'linear-gradient(90deg, #0C0E11 0%, rgb(12 14 17 / 0.85) 35%, rgb(12 14 17 / 0) 70%), linear-gradient(0deg, #0C0E11 0%, rgb(12 14 17 / 0) 45%), linear-gradient(180deg, rgb(12 14 17 / 0.55) 0%, rgb(12 14 17 / 0) 18%)',
    '--dl-letterbox': '#08090B',
    '--dl-multiview-canvas': '#050607',
  },
});

// ---------- type augmentation ----------
type DLColor = 'iris' | 'live' | 'ok' | 'warn' | DefaultMantineColor;
type DLButtonVariant = ButtonVariant | 'live' | 'danger' | 'overlay';
type DLActionIconVariant = ActionIconVariant | 'live' | 'danger' | 'overlay';

declare module '@mantine/core' {
  export interface MantineThemeColorsOverride {
    colors: Record<DLColor, MantineColorsTuple>;
  }
  // eslint-disable-next-line @typescript-eslint/no-empty-object-type
  export interface MantineThemeOther extends DLOther {}
  export interface ButtonProps {
    variant?: DLButtonVariant;
  }
  export interface ActionIconProps {
    variant?: DLActionIconVariant;
  }
}
