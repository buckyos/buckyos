/** @type {import('tailwindcss').Config} */
export default {
  content: ['./index.html', './src/**/*.{ts,tsx}'],
  theme: {
    extend: {
      fontFamily: {
        sans: ['var(--cp-font-sans)'],
        body: ['var(--cp-font-sans)'],
        display: ['var(--cp-font-sans)'],
      },
      boxShadow: {
        desktop: 'var(--cp-panel-shadow)',
      },
    },
  },
  plugins: [],
}
