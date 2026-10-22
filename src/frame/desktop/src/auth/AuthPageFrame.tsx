import type { ReactNode } from 'react'
import { Box, Container, Link, Paper, Typography } from '@mui/material'
import { Lock } from 'lucide-react'
import { ObjectImage } from '../presentation/ObjectIcon'
import { useZonePresentation } from '../presentation/zone'

/** The centered card of the signed-out pages (sign-in, share entries, invitations, consent): Zone icon, title, body. */
export function AuthPageFrame({
  title,
  header,
  children,
  maxWidth = 'xs',
  testId,
}: {
  title: ReactNode
  /** Lines under the title (Zone description, requesting app, …). */
  header?: ReactNode
  children: ReactNode
  maxWidth?: 'xs' | 'sm'
  testId?: string
}) {
  const zone = useZonePresentation()
  return (
    <Box
      sx={{
        minHeight: '100vh',
        display: 'flex',
        alignItems: { xs: 'flex-start', sm: 'center' },
        justifyContent: 'center',
        background:
          'linear-gradient(135deg, var(--cp-bg) 0%, var(--cp-bg-strong) 100%)',
        px: 2,
        pt: { xs: 6, sm: 4 },
        pb: 4,
      }}
      data-testid={testId}
    >
      <Container maxWidth={maxWidth} disableGutters>
        <Paper
          elevation={0}
          sx={{
            p: { xs: 3, sm: 4 },
            borderRadius: 4,
            border: '1px solid var(--cp-border)',
            backdropFilter: 'blur(24px)',
            boxShadow: 'var(--cp-panel-shadow)',
          }}
        >
          <Box
            sx={{
              display: 'flex',
              flexDirection: 'column',
              alignItems: 'center',
              mb: 3,
            }}
          >
            <ObjectImage
              icons={zone.icons}
              alt=""
              className="mb-4 h-12 w-12 rounded-xl object-cover"
              fallback={
                <Box
                  sx={{
                    width: 48,
                    height: 48,
                    borderRadius: 3,
                    display: 'flex',
                    alignItems: 'center',
                    justifyContent: 'center',
                    bgcolor: 'primary.main',
                    color: 'white',
                    mb: 2,
                  }}
                >
                  <Lock size={24} />
                </Box>
              }
            />
            <Typography
              variant="h5"
              sx={{ fontWeight: 700, fontFamily: '"Space Grotesk", sans-serif', textAlign: 'center' }}
            >
              {title}
            </Typography>
            {header}
          </Box>

          {children}

          <Typography
            variant="caption"
            color="text.secondary"
            sx={{
              display: 'block',
              textAlign: 'center',
              mt: 3,
              lineHeight: 1.6,
            }}
          >
            Powered by{' '}
            <Link
              href="https://github.com/buckyos/buckyos/"
              target="_blank"
              rel="noreferrer noopener"
              color="inherit"
              underline="hover"
              aria-label="BuckyOS on GitHub (opens in a new tab)"
              sx={{ fontWeight: 600 }}
            >
              BuckyOS
            </Link>
            , an open-source AI operating system.
          </Typography>
        </Paper>
      </Container>
    </Box>
  )
}
