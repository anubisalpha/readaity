import type { ReaderPrefs, ReaderThemeId } from "../types";

/** Palette for each reading theme. `name` is the label shown in Settings. */
export const READER_THEMES: Record<
  ReaderThemeId,
  { name: string; bg: string; fg: string; link: string; faint: string }
> = {
  dark: { name: "Dark", bg: "#14161a", fg: "#d8dade", link: "#6ea8fe", faint: "#2a2d33" },
  light: { name: "Light", bg: "#faf9f7", fg: "#1e2024", link: "#1f5fbf", faint: "#e6e4df" },
  sepia: { name: "Sepia", bg: "#f4ecd8", fg: "#4b3f2f", link: "#8a5a2b", faint: "#e2d5b8" },
};

export const DEFAULT_READER_PREFS: ReaderPrefs = { theme: "dark", fontScale: 1 };

export const FONT_SCALE_MIN = 0.8;
export const FONT_SCALE_MAX = 1.6;
export const FONT_SCALE_STEP = 0.1;

export function clampFontScale(n: number): number {
  if (!Number.isFinite(n) || n <= 0) return 1;
  return Math.min(FONT_SCALE_MAX, Math.max(FONT_SCALE_MIN, Math.round(n * 10) / 10));
}

/** Parse the two persisted setting strings into a `ReaderPrefs`. */
export function parseReaderPrefs(
  theme: string | null,
  fontScale: string | null,
): ReaderPrefs {
  const t = (theme ?? "") as ReaderThemeId;
  return {
    theme: t in READER_THEMES ? t : "dark",
    fontScale: fontScale ? clampFontScale(Number(fontScale)) : 1,
  };
}

/**
 * CSS injected into the reflowable readers' iframe (MOBI/AZW3/RTF via HtmlReader,
 * and the TXT reader). Base font size is 18px × the user's scale.
 */
export function readerThemeCss(prefs: ReaderPrefs): string {
  const t = READER_THEMES[prefs.theme];
  const px = (18 * prefs.fontScale).toFixed(1);
  return `
  html, body { margin: 0; background: ${t.bg}; color: ${t.fg}; }
  body { font-family: Georgia, serif; line-height: 1.6; font-size: ${px}px; }
  .rdr { max-width: 42rem; margin: 0 auto; padding: 28px 28px 96px; }
  img, image, svg { max-width: 100% !important; height: auto !important; object-fit: contain; }
  a { color: ${t.link}; }
  h1, h2, h3 { line-height: 1.25; }
  p { margin: 0.6em 0; }
  #kf8-toc { display: none; }
  .kf8-ncx { display: inline; }
`;
}
