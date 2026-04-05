/** @type {import('tailwindcss').Config} */
export default {
  content: ['./index.html', './src/**/*.{ts,tsx}'],
  theme: {
    extend: {
      fontFamily: {
        body: [
          '"Plus Jakarta Sans"',
          '"PingFang SC"',
          '"Hiragino Sans GB"',
          '"Microsoft YaHei"',
          'sans-serif',
        ],
        display: ['"Sora"', '"Plus Jakarta Sans"', 'sans-serif'],
      },
      boxShadow: {
        desktop: '0 24px 80px color-mix(in srgb, var(--cp-shadow) 42%, transparent)',
      },
    },
  },
  plugins: [],
}
