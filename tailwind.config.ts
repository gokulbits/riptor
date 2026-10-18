import type { Config } from "tailwindcss";

// Colours are CSS variables (see src/index.css) so light/dark themes swap at runtime.
const v = (name: string) => `rgb(var(--${name}) / <alpha-value>)`;

export default {
  darkMode: ["class", '[data-theme="dark"]'],
  content: ["./index.html", "./src/**/*.{ts,tsx}"],
  theme: {
    extend: {
      colors: {
        bg: v("bg"),
        panel: v("panel"),
        raised: v("raised"),
        border: v("border"),
        fg: v("fg"),
        muted: v("muted"),
        accent: v("accent"),
        "accent-fg": v("accent-fg"),
        ok: v("ok"),
        warn: v("warn"),
        danger: v("danger"),
        info: v("info"),
      },
      fontFamily: {
        sans: ["Inter", "Segoe UI", "Ubuntu", "system-ui", "sans-serif"],
        mono: ["JetBrains Mono", "Cascadia Mono", "Consolas", "monospace"],
      },
      fontSize: { xxs: ["0.6875rem", "1rem"] },
    },
  },
  plugins: [],
} satisfies Config;
