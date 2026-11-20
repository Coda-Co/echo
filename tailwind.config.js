/** @type {import('tailwindcss').Config} */
export default {
  content: ["./index.html", "./src/**/*.{ts,tsx}"],
  darkMode: "media",
  theme: {
    extend: {
      fontFamily: {
        // Geist Mono everywhere, bundled from @fontsource-variable/geist-mono (imported in src/main.tsx).
        sans: ["Geist Mono Variable", "ui-monospace", "SFMono-Regular", "Menlo", "Consolas", "monospace"],
        mono: ["Geist Mono Variable", "ui-monospace", "SFMono-Regular", "Menlo", "Consolas", "monospace"],
      },
    },
  },
  plugins: [],
};
