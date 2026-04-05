/* eslint-disable react-refresh/only-export-components */
import {
  createContext,
  useContext,
  useEffect,
  useMemo,
  useState,
  type PropsWithChildren,
} from 'react'
import { CssBaseline, ThemeProvider, createTheme } from '@mui/material'
import type { ThemeMode } from '../models/ui'

interface ThemeModeContextValue {
  themeMode: ThemeMode
  setThemeMode: (mode: ThemeMode) => void
}

const storageKey = 'buckyos.prototype.theme.v1'

const ThemeModeContext = createContext<ThemeModeContextValue | null>(null)

export function PrototypeThemeProvider({ children }: PropsWithChildren) {
  const [themeMode, setThemeMode] = useState<ThemeMode>(() => {
    const saved = window.localStorage.getItem(storageKey) as ThemeMode | null
    return saved ?? 'light'
  })

  useEffect(() => {
    window.localStorage.setItem(storageKey, themeMode)
    document.documentElement.dataset.theme = themeMode
  }, [themeMode])

  const theme = useMemo(() => {
    const primaryMain = themeMode === 'light' ? '#5f67e8' : '#95a3ff'
    const secondaryMain = themeMode === 'light' ? '#9887ea' : '#b19cff'
    const successMain = themeMode === 'light' ? '#3ea67e' : '#69c49c'
    const backgroundDefault = themeMode === 'light' ? '#f6f1fb' : '#2a2737'
    const backgroundPaper = themeMode === 'light' ? '#fcfaff' : '#39354a'
    const textPrimary = themeMode === 'light' ? '#3a3553' : '#f2effa'
    const textSecondary = themeMode === 'light' ? '#726c89' : '#c2bbd6'
    const divider = themeMode === 'light' ? 'rgba(117, 109, 148, 0.24)' : 'rgba(165, 157, 195, 0.24)'
    const primaryButton = themeMode === 'light'
      ? {
          background: 'oklch(0.58 0.108 258)',
          hoverBackground: 'oklch(0.54 0.118 258)',
          activeBackground: 'oklch(0.51 0.112 258)',
          text: 'oklch(0.985 0.006 286)',
          outlineHover: 'color-mix(in srgb, var(--cp-accent) 12%, var(--cp-surface))',
        }
      : {
          background: 'oklch(0.72 0.082 258)',
          hoverBackground: 'oklch(0.75 0.086 258)',
          activeBackground: 'oklch(0.68 0.078 258)',
          text: 'oklch(0.24 0.02 272)',
          outlineHover: 'color-mix(in srgb, var(--cp-accent) 16%, var(--cp-surface))',
        }

    return createTheme({
      palette: {
        mode: themeMode,
        primary: {
          main: primaryMain,
        },
        secondary: {
          main: secondaryMain,
        },
        success: {
          main: successMain,
        },
        background: {
          default: backgroundDefault,
          paper: backgroundPaper,
        },
        text: {
          primary: textPrimary,
          secondary: textSecondary,
        },
        divider,
      },
      typography: {
        fontFamily: '"Work Sans", sans-serif',
        h1: { fontFamily: '"Space Grotesk", sans-serif' },
        h2: { fontFamily: '"Space Grotesk", sans-serif' },
        h3: { fontFamily: '"Space Grotesk", sans-serif' },
        button: {
          fontFamily: '"Space Grotesk", sans-serif',
          fontWeight: 700,
          letterSpacing: '0.02em',
          textTransform: 'none',
        },
      },
      shape: {
        borderRadius: 20,
      },
      components: {
        MuiButton: {
          defaultProps: {
            disableElevation: true,
            variant: 'contained',
          },
          styleOverrides: {
            root: {
              minHeight: 44,
              borderRadius: 16,
              paddingInline: 18,
              boxShadow:
                '0 14px 30px color-mix(in srgb, var(--cp-accent) 16%, transparent)',
              transition:
                'transform 180ms var(--cp-ease-emphasis), background-color 180ms var(--cp-ease-smooth), border-color 180ms var(--cp-ease-smooth), box-shadow 180ms var(--cp-ease-smooth)',
              '&:hover': {
                transform: 'translateY(-1px)',
                boxShadow:
                  '0 18px 36px color-mix(in srgb, var(--cp-accent) 22%, transparent)',
              },
              '&:active': {
                transform: 'translateY(0)',
              },
              '&.Mui-disabled': {
                boxShadow: 'none',
                transform: 'none',
              },
            },
            contained: {
              background: primaryButton.background,
              color: primaryButton.text,
              '&:hover': {
                background: primaryButton.hoverBackground,
              },
              '&:active': {
                background: primaryButton.activeBackground,
              },
              '&.Mui-disabled': {
                background: 'color-mix(in srgb, var(--cp-text) 10%, var(--cp-surface-2))',
                color: 'color-mix(in srgb, var(--cp-muted) 82%, transparent)',
              },
            },
            outlined: {
              borderColor: 'var(--cp-border)',
              backgroundColor: 'color-mix(in srgb, var(--cp-surface) 78%, transparent)',
              color: 'var(--cp-text)',
              boxShadow: 'none',
              '&:hover': {
                borderColor: 'color-mix(in srgb, var(--cp-accent) 28%, var(--cp-border))',
                backgroundColor: primaryButton.outlineHover,
                boxShadow: 'none',
              },
            },
            text: {
              color: 'var(--cp-text)',
              boxShadow: 'none',
              '&:hover': {
                backgroundColor:
                  'color-mix(in srgb, var(--cp-accent-soft) 10%, transparent)',
                boxShadow: 'none',
              },
            },
            sizeSmall: {
              minHeight: 36,
              paddingInline: 14,
              borderRadius: 12,
            },
            sizeLarge: {
              minHeight: 50,
              paddingInline: 22,
              borderRadius: 18,
            },
          },
        },
        MuiIconButton: {
          styleOverrides: {
            root: {
              border: '1px solid color-mix(in srgb, var(--cp-border) 78%, transparent)',
              backgroundColor:
                'color-mix(in srgb, var(--cp-surface) 84%, transparent)',
              color: 'var(--cp-text)',
              '&:hover': {
                backgroundColor:
                  'color-mix(in srgb, var(--cp-accent-soft) 10%, var(--cp-surface))',
                borderColor:
                  'color-mix(in srgb, var(--cp-accent) 24%, var(--cp-border))',
              },
            },
            sizeSmall: {
              width: 34,
              height: 34,
            },
          },
        },
        MuiTextField: {
          defaultProps: {
            fullWidth: true,
            size: 'small',
            variant: 'outlined',
          },
        },
        MuiOutlinedInput: {
          styleOverrides: {
            root: {
              borderRadius: 18,
              backgroundColor:
                'color-mix(in srgb, var(--cp-surface) 88%, transparent)',
              '& .MuiOutlinedInput-notchedOutline': {
                borderColor: 'var(--cp-border)',
              },
              '&:hover .MuiOutlinedInput-notchedOutline': {
                borderColor:
                  'color-mix(in srgb, var(--cp-accent) 24%, var(--cp-border))',
              },
              '&.Mui-focused .MuiOutlinedInput-notchedOutline': {
                borderColor: 'var(--cp-accent)',
              },
            },
            input: {
              color: 'var(--cp-text)',
            },
          },
        },
        MuiInputLabel: {
          styleOverrides: {
            root: {
              color: 'var(--cp-muted)',
            },
          },
        },
        MuiMenu: {
          styleOverrides: {
            paper: {
              border: '1px solid var(--cp-border)',
              borderRadius: 18,
              background:
                'linear-gradient(180deg, color-mix(in srgb, var(--cp-surface) 94%, transparent), color-mix(in srgb, var(--cp-surface-2) 96%, transparent))',
              boxShadow: 'var(--cp-panel-shadow)',
              backdropFilter: 'blur(18px)',
            },
          },
        },
        MuiMenuItem: {
          styleOverrides: {
            root: {
              minHeight: 42,
              borderRadius: 12,
              marginInline: 6,
              marginBlock: 2,
            },
          },
        },
        MuiAlert: {
          styleOverrides: {
            root: {
              borderRadius: 18,
              border: '1px solid var(--cp-border)',
              boxShadow: 'var(--cp-panel-shadow)',
            },
          },
        },
      },
    })
  }, [themeMode])

  const value = useMemo(
    () => ({
      themeMode,
      setThemeMode,
    }),
    [themeMode],
  )

  return (
    <ThemeModeContext.Provider value={value}>
      <ThemeProvider theme={theme}>
        <CssBaseline />
        {children}
      </ThemeProvider>
    </ThemeModeContext.Provider>
  )
}

export function useThemeMode() {
  const context = useContext(ThemeModeContext)

  if (!context) {
    throw new Error('useThemeMode must be used within PrototypeThemeProvider')
  }

  return context
}
