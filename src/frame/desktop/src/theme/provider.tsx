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

/** `mode` fixes the theme for an embedded page (the embedding App decides it); it is not stored. `initialMode` is the
 * theme a page knows better than this browser (a Workspace origin's: the account's): it replaces the stored one and
 * is stored. */
export function PrototypeThemeProvider({ children, mode, initialMode }: PropsWithChildren<{ mode?: ThemeMode; initialMode?: ThemeMode }>) {
  const [storedMode, setThemeMode] = useState<ThemeMode>(() => {
    if (initialMode) return initialMode
    const saved = window.localStorage.getItem(storageKey)
    return saved === 'dark' || saved === 'light' ? saved : 'light'
  })
  const themeMode = mode ?? storedMode

  useEffect(() => {
    if (!mode) window.localStorage.setItem(storageKey, themeMode)
    document.documentElement.dataset.theme = themeMode
  }, [themeMode, mode])

  const theme = useMemo(() => {
    const light = themeMode === 'light'
    const primaryMain = light ? '#406ad5' : '#87aaf1'
    const secondaryMain = light ? '#6a7587' : '#a1a1a6'
    const successMain = light ? '#3ea67e' : '#69c49c'
    const backgroundDefault = light ? '#f3f3f2' : '#141415'
    const backgroundPaper = light ? '#fdfdfc' : '#1c1c1e'
    const textPrimary = light ? '#1b1b1a' : '#ededed'
    const textSecondary = light ? '#6b6b69' : '#a1a1a6'
    const divider = light ? 'rgba(27, 27, 26, 0.12)' : 'rgba(237, 237, 237, 0.1)'
    const primaryButton = light
      ? {
          background: '#406ad5',
          hoverBackground: '#3a60c4',
          activeBackground: '#3557b3',
          text: '#ffffff',
        }
      : {
          background: '#ededed',
          hoverBackground: '#ffffff',
          activeBackground: '#d6d6d6',
          text: '#141415',
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
        fontFamily: 'var(--cp-font-sans)',
        h1: { fontWeight: 500 },
        h2: { fontWeight: 500 },
        h3: { fontWeight: 500 },
        button: {
          fontWeight: 500,
          letterSpacing: 0,
          textTransform: 'none',
        },
      },
      shape: {
        borderRadius: 12,
      },
      components: {
        MuiButton: {
          defaultProps: {
            disableElevation: true,
            variant: 'contained',
          },
          styleOverrides: {
            root: {
              minHeight: 40,
              borderRadius: 10,
              paddingInline: 16,
              boxShadow: 'none',
              transition:
                'transform 120ms var(--cp-ease-smooth), background-color 150ms var(--cp-ease-smooth), border-color 150ms var(--cp-ease-smooth), color 150ms var(--cp-ease-smooth)',
              '&:hover': {
                boxShadow: 'none',
              },
              '&:active': {
                transform: 'scale(0.98)',
              },
              '&.Mui-disabled': {
                boxShadow: 'none',
                transform: 'none',
              },
            },
            containedPrimary: {
              background: primaryButton.background,
              color: primaryButton.text,
              '&:hover': {
                background: primaryButton.hoverBackground,
              },
              '&:active': {
                background: primaryButton.activeBackground,
              },
            },
            contained: {
              '&.Mui-disabled': {
                background: 'color-mix(in srgb, var(--cp-text) 8%, var(--cp-surface-2))',
                color: 'var(--cp-muted)',
              },
            },
            outlined: {
              borderColor: 'var(--cp-border)',
              backgroundColor: 'var(--cp-surface)',
              color: 'var(--cp-text)',
              '&:hover': {
                borderColor: 'color-mix(in srgb, var(--cp-text) 22%, transparent)',
                backgroundColor: 'color-mix(in srgb, var(--cp-text) 4%, var(--cp-surface))',
              },
            },
            text: {
              color: 'var(--cp-text)',
              '&:hover': {
                backgroundColor: 'color-mix(in srgb, var(--cp-text) 6%, transparent)',
              },
            },
            sizeSmall: {
              minHeight: 32,
              paddingInline: 12,
              borderRadius: 8,
            },
            sizeLarge: {
              minHeight: 48,
              paddingInline: 20,
              borderRadius: 12,
            },
          },
        },
        MuiIconButton: {
          styleOverrides: {
            root: {
              color: 'var(--cp-text)',
              '&:hover': {
                backgroundColor: 'color-mix(in srgb, var(--cp-text) 7%, transparent)',
              },
            },
            sizeSmall: {
              width: 32,
              height: 32,
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
              borderRadius: 10,
              backgroundColor: 'var(--cp-surface)',
              '& .MuiOutlinedInput-notchedOutline': {
                borderColor: 'var(--cp-border)',
              },
              '&:hover .MuiOutlinedInput-notchedOutline': {
                borderColor: 'color-mix(in srgb, var(--cp-text) 24%, transparent)',
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
              borderRadius: 12,
              background: 'var(--cp-surface-opaque)',
              boxShadow: 'var(--cp-panel-shadow)',
            },
          },
        },
        MuiMenuItem: {
          styleOverrides: {
            root: {
              minHeight: 36,
              borderRadius: 8,
              marginInline: 4,
              marginBlock: 2,
              fontSize: 14,
            },
          },
        },
        MuiAlert: {
          styleOverrides: {
            root: {
              borderRadius: 10,
              border: '1px solid var(--cp-border)',
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
