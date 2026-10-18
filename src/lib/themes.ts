// Theme registry. Each theme is a full set of colour tokens (RGB triplets so
// Tailwind's `/alpha` modifiers work); applying one sets CSS variables on <html>.

export type Tokens = {
  bg: string;
  panel: string;
  raised: string;
  border: string;
  fg: string;
  muted: string;
  accent: string;
  "accent-fg": string;
  ok: string;
  warn: string;
  danger: string;
  info: string;
};

export interface ThemeDef {
  id: string;
  label: string;
  scheme: "light" | "dark";
  tokens: Tokens;
}

export const THEMES: ThemeDef[] = [
  {
    id: "light",
    label: "Light",
    scheme: "light",
    tokens: {
      bg: "246 247 249", panel: "255 255 255", raised: "240 242 245", border: "226 229 234",
      fg: "22 27 34", muted: "101 109 122", accent: "79 70 229", "accent-fg": "255 255 255",
      ok: "22 163 74", warn: "217 119 6", danger: "220 38 38", info: "8 145 178",
    },
  },
  {
    id: "dark",
    label: "Dark",
    scheme: "dark",
    tokens: {
      bg: "13 15 20", panel: "20 23 30", raised: "29 33 42", border: "40 45 56",
      fg: "230 233 239", muted: "139 147 162", accent: "129 140 248", "accent-fg": "13 15 20",
      ok: "74 222 128", warn: "251 191 36", danger: "248 113 113", info: "34 211 238",
    },
  },
  {
    id: "midnight",
    label: "Midnight (OLED)",
    scheme: "dark",
    tokens: {
      bg: "0 0 0", panel: "8 8 10", raised: "20 20 24", border: "34 34 40",
      fg: "236 236 240", muted: "130 130 142", accent: "96 165 250", "accent-fg": "0 0 0",
      ok: "52 211 153", warn: "250 204 21", danger: "248 113 113", info: "56 189 248",
    },
  },
  {
    id: "nord",
    label: "Nord",
    scheme: "dark",
    tokens: {
      bg: "46 52 64", panel: "59 66 82", raised: "67 76 94", border: "76 86 106",
      fg: "236 239 244", muted: "180 188 204", accent: "136 192 208", "accent-fg": "46 52 64",
      ok: "163 190 140", warn: "235 203 139", danger: "191 97 106", info: "129 161 193",
    },
  },
  {
    id: "dracula",
    label: "Dracula",
    scheme: "dark",
    tokens: {
      bg: "33 34 44", panel: "40 42 54", raised: "52 55 70", border: "68 71 90",
      fg: "248 248 242", muted: "160 166 196", accent: "189 147 249", "accent-fg": "33 34 44",
      ok: "80 250 123", warn: "241 250 140", danger: "255 85 85", info: "139 233 253",
    },
  },
  {
    id: "solarized-dark",
    label: "Solarized Dark",
    scheme: "dark",
    tokens: {
      bg: "0 43 54", panel: "7 54 66", raised: "13 66 80", border: "30 82 96",
      fg: "238 232 213", muted: "147 161 161", accent: "38 139 210", "accent-fg": "253 246 227",
      ok: "133 153 0", warn: "181 137 0", danger: "220 50 47", info: "42 161 152",
    },
  },
  {
    id: "solarized-light",
    label: "Solarized Light",
    scheme: "light",
    tokens: {
      bg: "253 246 227", panel: "255 251 240", raised: "238 232 213", border: "220 212 188",
      fg: "7 54 66", muted: "101 123 131", accent: "38 139 210", "accent-fg": "253 246 227",
      ok: "133 153 0", warn: "181 137 0", danger: "220 50 47", info: "42 161 152",
    },
  },
  {
    id: "forest",
    label: "Forest",
    scheme: "dark",
    tokens: {
      bg: "17 24 20", panel: "23 32 27", raised: "32 44 37", border: "45 60 51",
      fg: "226 236 228", muted: "142 163 150", accent: "74 222 128", "accent-fg": "17 24 20",
      ok: "134 239 172", warn: "250 204 21", danger: "248 113 113", info: "45 212 191",
    },
  },
  {
    id: "high-contrast",
    label: "High contrast",
    scheme: "dark",
    tokens: {
      bg: "0 0 0", panel: "0 0 0", raised: "26 26 26", border: "255 255 255",
      fg: "255 255 255", muted: "210 210 210", accent: "255 255 0", "accent-fg": "0 0 0",
      ok: "0 255 0", warn: "255 200 0", danger: "255 80 80", info: "0 230 255",
    },
  },
];

/** Optional accent override, applied on top of any theme. */
export const ACCENTS: { id: string; label: string; rgb: string }[] = [
  { id: "theme", label: "Theme default", rgb: "" },
  { id: "indigo", label: "Indigo", rgb: "99 102 241" },
  { id: "blue", label: "Blue", rgb: "59 130 246" },
  { id: "teal", label: "Teal", rgb: "20 184 166" },
  { id: "green", label: "Green", rgb: "34 197 94" },
  { id: "amber", label: "Amber", rgb: "245 158 11" },
  { id: "rose", label: "Rose", rgb: "244 63 94" },
  { id: "violet", label: "Violet", rgb: "139 92 246" },
];

export function resolveTheme(id: string, prefersDark: boolean): ThemeDef {
  if (id === "system") return THEMES.find((t) => t.id === (prefersDark ? "dark" : "light"))!;
  return THEMES.find((t) => t.id === id) ?? THEMES[1];
}

/** Pick black or white text for a given accent so buttons stay readable. */
function contrastFg(rgb: string): string {
  const [r, g, b] = rgb.split(" ").map(Number);
  const lum = 0.2126 * r + 0.7152 * g + 0.0722 * b;
  return lum > 150 ? "0 0 0" : "255 255 255";
}

export function applyTheme(theme: ThemeDef, accentId: string) {
  const root = document.documentElement;
  const tokens: Tokens = { ...theme.tokens };
  const accent = ACCENTS.find((a) => a.id === accentId);
  if (accent && accent.rgb) {
    tokens.accent = accent.rgb;
    tokens["accent-fg"] = contrastFg(accent.rgb);
  }
  for (const [k, v] of Object.entries(tokens)) root.style.setProperty(`--${k}`, v);
  root.style.colorScheme = theme.scheme;
  root.dataset.theme = theme.id;
  root.dataset.scheme = theme.scheme;
}
