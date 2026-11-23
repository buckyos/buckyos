/** @type {import('tailwindcss').Config} */
export default {
  content: ['./index.html', './src/**/*.{ts,tsx}'],
  theme: {
    extend: {
      colors: {
        bg: 'var(--bg)',
        panel: 'var(--panel)',
        line: 'var(--line)',
        fg: 'var(--fg)',
        mute: 'var(--mute)',
        accent: 'var(--accent)',
        primary: 'var(--primary)',
        'on-primary': 'var(--on-primary)',
        ok: 'var(--ok)',
        warn: 'var(--warn)',
        bad: 'var(--bad)',
        soft: 'var(--soft)',
        hover: 'var(--hover)',
        bar: 'var(--bar)',
      },
      fontFamily: {
        sans: [
          '-apple-system',
          'BlinkMacSystemFont',
          "'Segoe UI'",
          'Roboto',
          "'Helvetica Neue'",
          'Arial',
          "'PingFang SC'",
          "'Hiragino Sans GB'",
          "'Microsoft YaHei'",
          "'Noto Sans CJK SC'",
          'system-ui',
          'sans-serif',
        ],
        mono: ['ui-monospace', "'SF Mono'", "'Cascadia Mono'", "'JetBrains Mono'", 'Menlo', 'Consolas', "'Liberation Mono'", 'monospace'],
      },
      transitionTimingFunction: {
        DEFAULT: 'var(--ease)',
      },
    },
  },
  plugins: [],
}
