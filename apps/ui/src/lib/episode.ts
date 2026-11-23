import { t } from './i18n';

interface Episodic {
  season?: number | null;
  episode?: number | null;
}

/** `S2 E5`, `E5` (no season), or null when there is no episode number. */
export function episodeCode({ season, episode }: Episodic): string | null {
  if (episode == null || episode <= 0) return null;
  return season != null && season > 0 ? t('episode.code', { s: season, e: episode }) : t('episode.codeE', { e: episode });
}

/** `S2 E5 · Low Tide`, either half alone, or null. The subtitle is dropped when it only repeats the code. */
export function episodeLine(p: Episodic & { subtitle?: string | null; episodeTitle?: string | null }): string | null {
  const code = episodeCode(p);
  const sub = (p.subtitle ?? p.episodeTitle ?? '').trim();
  const useful = sub && !/^(s\d+\s*e\d+|episode\s*\d+|ep\.?\s*\d+)$/i.test(sub) ? sub : null;
  if (code && useful) return t('episode.line', { code, title: useful });
  return code ?? useful;
}
