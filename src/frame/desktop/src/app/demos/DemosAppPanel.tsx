import {
  Alert,
  Button,
  Checkbox,
  Chip,
  FormControlLabel,
  IconButton,
  LinearProgress,
  Menu,
  MenuItem,
  Radio,
  RadioGroup,
  Slider,
  Switch,
  Tab,
  Tabs,
  TextField,
  useMediaQuery,
} from '@mui/material'
import {
  Bell,
  Check,
  ChevronDown,
  MoonStar,
  Search,
  SunMedium,
} from 'lucide-react'
import { useState } from 'react'
import { AppIcon } from '../../components/DesktopVisuals'
import { appIconSurfaceStyle } from '../../components/DesktopVisualTokens'
import { useI18n } from '../../i18n/provider'
import { localeLabels } from '../../mock/data'
import {
  WindowDialogPermissionError,
  useWindowDialog,
} from '../../desktop/windows/dialogs'
import { DemoSection, MetricCard, PanelIntro } from '../../components/AppPanelPrimitives'
import type { AppContentLoaderProps } from '../types'

export function DemosAppPanel({
  locale,
  themeMode,
}: AppContentLoaderProps) {
  const { t } = useI18n()
  const windowDialog = useWindowDialog()
  const isCompact = useMediaQuery('(max-width: 900px)')
  const [query, setQuery] = useState('Window controls')
  const [owner, setOwner] = useState('Prototype team')
  const [density, setDensity] = useState<'compact' | 'balanced' | 'comfortable'>('balanced')
  const [releaseState, setReleaseState] = useState<'draft' | 'review' | 'ready'>('review')
  const [notes, setNotes] = useState(
    'Validate spacing rhythm, focus rings, disabled states, and mobile fit.',
  )
  const [notifications, setNotifications] = useState(true)
  const [offlineCache, setOfflineCache] = useState(false)
  const [autoArrange, setAutoArrange] = useState(true)
  const [launchMode, setLaunchMode] = useState<'windowed' | 'maximized' | 'focused'>('windowed')
  const [scale, setScale] = useState(58)
  const [tab, setTab] = useState(0)
  const [menuAnchor, setMenuAnchor] = useState<HTMLElement | null>(null)
  const [dialogState, setDialogState] = useState<'idle' | 'applied' | 'dismissed'>('idle')
  const [fullscreenState, setFullscreenState] = useState<'idle' | 'opened' | 'denied'>('idle')

  const densityLabels = {
    compact: t('demos.density.compact'),
    balanced: t('demos.density.balanced'),
    comfortable: t('demos.density.comfortable'),
  }
  const releaseLabels = {
    draft: t('demos.state.draft'),
    review: t('demos.state.review'),
    ready: t('demos.state.ready'),
  }
  const launchModeLabels = {
    windowed: t('demos.launch.windowed'),
    maximized: t('demos.launch.maximized'),
    focused: t('demos.launch.focused'),
  }
  const dialogStateLabels = {
    idle: t('demos.dialog.state.idle'),
    applied: t('demos.dialog.state.applied'),
    dismissed: t('demos.dialog.state.dismissed'),
  }
  const fullscreenStateLabels = {
    idle: t('demos.dialog.fullscreen.state.idle'),
    opened: t('demos.dialog.fullscreen.state.opened'),
    denied: t('demos.dialog.fullscreen.state.denied'),
  }

  const readiness = Math.min(
    100,
    (releaseState === 'ready' ? 40 : releaseState === 'review' ? 28 : 14) +
      (notifications ? 18 : 6) +
      (offlineCache ? 12 : 0) +
      (autoArrange ? 10 : 4) +
      (launchMode === 'maximized' ? 12 : launchMode === 'focused' ? 9 : 7) +
      Math.round(scale / 5),
  )

  const handleOpenWindowDialog = async () => {
    const result = await windowDialog.open<'apply'>({
      title: t('demos.dialog.title'),
      description: t('demos.dialog.description'),
      size: 'md',
      renderBody: () => (
        <div className="space-y-3 text-sm leading-6 text-[color:var(--cp-muted)]">
          <p>{t('demos.dialog.body')}</p>
          <div className="rounded-[18px] border border-[color:color-mix(in_srgb,var(--cp-border)_76%,transparent)] bg-[color:color-mix(in_srgb,var(--cp-accent-soft)_8%,var(--cp-surface))] px-4 py-3 text-[color:var(--cp-text)]">
            {t('demos.dialog.note')}
          </div>
        </div>
      ),
      renderActions: ({ close, dismiss }) => (
        <>
          <Button type="button" variant="text" onClick={() => dismiss()}>
            {t('common.cancel')}
          </Button>
          <Button
            type="button"
            variant="contained"
            onClick={() => close('apply')}
          >
            {t('demos.dialog.confirm')}
          </Button>
        </>
      ),
    })

    if (result === 'apply') {
      setDialogState('applied')
      setOwner(t('demos.dialog.ownerApplied'))
      return
    }

    setDialogState('dismissed')
  }

  const handleOpenFullscreenDialog = async () => {
    try {
      const result = await windowDialog.open<'apply'>({
        presentation: 'fullscreen',
        title: t('demos.dialog.fullscreen.title'),
        description: t('demos.dialog.fullscreen.description'),
        size: 'fullscreen',
        renderBody: () => (
          <p className="text-sm leading-6 text-[color:var(--cp-muted)]">
            {t('demos.dialog.fullscreen.body')}
          </p>
        ),
        renderActions: ({ close, dismiss }) => (
          <>
            <Button type="button" variant="text" onClick={() => dismiss()}>
              {t('common.cancel')}
            </Button>
            <Button type="button" variant="contained" onClick={() => close('apply')}>
              {t('demos.dialog.confirm')}
            </Button>
          </>
        ),
      })

      setFullscreenState(result === 'apply' ? 'opened' : 'idle')
    } catch (error) {
      if (error instanceof WindowDialogPermissionError) {
        setFullscreenState('denied')
        return
      }

      throw error
    }
  }

  return (
    <div className="space-y-4">
      <PanelIntro
        kicker="Controls"
        title={t('demos.title')}
        body={t('demos.body')}
        aside={
          <div className="shell-subtle-panel max-w-[18rem] px-4 py-3">
            <div className="flex items-center gap-3">
              <span
                className="flex h-12 w-12 items-center justify-center rounded-[18px] shadow-[0_8px_20px_color-mix(in_srgb,var(--cp-shadow)_10%,transparent)]"
                style={appIconSurfaceStyle('var(--cp-accent-soft)', 'window')}
              >
                <AppIcon iconKey="demos" className="text-white" />
              </span>
              <div>
                <p className="shell-kicker">{t('apps.demos')}</p>
                <p className="mt-1 text-sm leading-5 text-[color:var(--cp-muted)]">
                  {t('demos.previewBody')}
                </p>
              </div>
            </div>
            <Button
              className="!mt-3"
              color="secondary"
              data-testid="demos-open-window-dialog"
              size="small"
              type="button"
              variant="contained"
              onClick={() => void handleOpenWindowDialog()}
            >
              {t('demos.dialog.trigger')}
            </Button>
          </div>
        }
      />

      <div className="sm:hidden">
        <Button
          fullWidth
          color="secondary"
          data-testid="demos-open-window-dialog"
          type="button"
          variant="contained"
          onClick={() => void handleOpenWindowDialog()}
        >
          {t('demos.dialog.trigger')}
        </Button>
      </div>

      <div className="grid gap-3 md:grid-cols-3">
        <MetricCard label={t('demos.readiness')} tone="success" value={`${readiness}%`} />
        <MetricCard
          label={t('demos.selection')}
          tone="accent"
          value={launchModeLabels[launchMode]}
        />
        <MetricCard
          label={t('demos.menuState')}
          tone={menuAnchor ? 'warning' : 'neutral'}
          value={menuAnchor ? t('demos.menu.open') : t('demos.menu.closed')}
        />
      </div>

      <div className="grid gap-4 xl:grid-cols-[minmax(0,1.15fr)_minmax(320px,0.85fr)]">
        <div className="space-y-4">
          <DemoSection title={t('demos.actions')} body={t('demos.actionsBody')}>
            <div className="flex flex-wrap gap-3">
              <Button startIcon={<Check size={16} />} type="button">
                {t('demos.primaryAction')}
              </Button>
              <Button variant="outlined" type="button">
                {t('demos.secondaryAction')}
              </Button>
              <Button variant="text" type="button">
                {t('demos.tertiaryAction')}
              </Button>
              <Button disabled type="button">
                {t('demos.disabledAction')}
              </Button>
              <Button
                variant="outlined"
                type="button"
                endIcon={<ChevronDown size={16} />}
                onClick={(event) => setMenuAnchor(event.currentTarget)}
              >
                {t('demos.quickMenu')}
              </Button>
              <Button
                data-testid="demos-open-fullscreen-dialog"
                type="button"
                variant="outlined"
                onClick={() => void handleOpenFullscreenDialog()}
              >
                {t('demos.dialog.fullscreen.trigger')}
              </Button>
            </div>

            <div className="mt-4 flex flex-wrap gap-2">
              <IconButton aria-label={t('demos.searchLabel')} size="small">
                <Search size={16} />
              </IconButton>
              <IconButton aria-label={t('demos.selection')} size="small">
                <Bell size={16} />
              </IconButton>
              <IconButton aria-label={t('common.light')} size="small">
                <SunMedium size={16} />
              </IconButton>
              <IconButton aria-label={t('common.dark')} size="small">
                <MoonStar size={16} />
              </IconButton>
            </div>

            <div className="mt-4 flex flex-wrap gap-2">
              <Chip size="small" label={`${t('common.theme')}: ${t(themeMode === 'light' ? 'common.light' : 'common.dark')}`} />
              <Chip size="small" label={`${t('common.language')}: ${localeLabels[locale as keyof typeof localeLabels] ?? locale}`} />
              <Chip size="small" label={`${t('demos.stateLabel')}: ${releaseLabels[releaseState]}`} />
              <Chip size="small" label={`${t('demos.dialog.metric')}: ${dialogStateLabels[dialogState]}`} />
              <Chip size="small" label={`${t('demos.dialog.fullscreen.permission')}: ${windowDialog.permissions.fullscreen ? t('demos.dialog.fullscreen.allowed') : t('demos.dialog.fullscreen.denied')}`} />
              <Chip size="small" label={`${t('demos.dialog.fullscreen.metric')}: ${fullscreenStateLabels[fullscreenState]}`} />
            </div>

            <Menu
              anchorEl={menuAnchor}
              open={Boolean(menuAnchor)}
              onClose={() => setMenuAnchor(null)}
            >
              <MenuItem onClick={() => setMenuAnchor(null)}>
                {t('demos.quickMenu.pin')}
              </MenuItem>
              <MenuItem onClick={() => setMenuAnchor(null)}>
                {t('demos.quickMenu.duplicate')}
              </MenuItem>
              <MenuItem onClick={() => setMenuAnchor(null)}>
                {t('demos.quickMenu.archive')}
              </MenuItem>
            </Menu>
          </DemoSection>

          <DemoSection title={t('demos.inputs')} body={t('demos.inputsBody')}>
            <div className="grid gap-3 sm:grid-cols-2">
              <TextField
                label={t('demos.searchLabel')}
                value={query}
                onChange={(event) => setQuery(event.target.value)}
              />
              <TextField
                label={t('demos.ownerLabel')}
                value={owner}
                onChange={(event) => setOwner(event.target.value)}
              />
              <TextField
                select
                label={t('demos.densityLabel')}
                value={density}
                onChange={(event) =>
                  setDensity(event.target.value as 'compact' | 'balanced' | 'comfortable')
                }
                SelectProps={{ native: true }}
              >
                {Object.entries(densityLabels).map(([value, label]) => (
                  <option key={value} value={value}>
                    {label}
                  </option>
                ))}
              </TextField>
              <TextField
                select
                label={t('demos.stateLabel')}
                value={releaseState}
                onChange={(event) =>
                  setReleaseState(event.target.value as 'draft' | 'review' | 'ready')
                }
                SelectProps={{ native: true }}
              >
                {Object.entries(releaseLabels).map(([value, label]) => (
                  <option key={value} value={value}>
                    {label}
                  </option>
                ))}
              </TextField>
              <div className="sm:col-span-2">
                <TextField
                  label={t('demos.notesLabel')}
                  value={notes}
                  multiline
                  minRows={4}
                  onChange={(event) => setNotes(event.target.value)}
                />
              </div>
            </div>
          </DemoSection>
        </div>

        <div className="space-y-4">
          <DemoSection title={t('demos.selectionTitle')} body={t('demos.selectionBody')}>
            <div className="grid gap-4 lg:grid-cols-[minmax(0,1fr)_minmax(220px,0.9fr)]">
              <div className="space-y-3">
                <div className="rounded-[22px] border border-[color:color-mix(in_srgb,var(--cp-border)_84%,transparent)] bg-[color:color-mix(in_srgb,var(--cp-surface)_88%,transparent)] px-4 py-3">
                  <FormControlLabel
                    control={
                      <Switch
                        checked={notifications}
                        onChange={(event) => setNotifications(event.target.checked)}
                      />
                    }
                    label={t('demos.selection.notifications')}
                  />
                </div>
                <div className="rounded-[22px] border border-[color:color-mix(in_srgb,var(--cp-border)_84%,transparent)] bg-[color:color-mix(in_srgb,var(--cp-surface)_88%,transparent)] px-4 py-3">
                  <FormControlLabel
                    control={
                      <Checkbox
                        checked={offlineCache}
                        onChange={(event) => setOfflineCache(event.target.checked)}
                      />
                    }
                    label={t('demos.selection.offline')}
                  />
                  <FormControlLabel
                    control={
                      <Checkbox
                        checked={autoArrange}
                        onChange={(event) => setAutoArrange(event.target.checked)}
                      />
                    }
                    label={t('demos.selection.autoArrange')}
                  />
                </div>
                <div className="rounded-[22px] border border-[color:color-mix(in_srgb,var(--cp-border)_84%,transparent)] bg-[color:color-mix(in_srgb,var(--cp-surface)_88%,transparent)] px-4 py-3">
                  <p className="mb-2 text-sm font-medium text-[color:var(--cp-text)]">
                    {t('demos.launchMode')}
                  </p>
                  <RadioGroup
                    value={launchMode}
                    onChange={(event) =>
                      setLaunchMode(
                        event.target.value as 'windowed' | 'maximized' | 'focused',
                      )
                    }
                  >
                    <FormControlLabel
                      value="windowed"
                      control={<Radio />}
                      label={launchModeLabels.windowed}
                    />
                    <FormControlLabel
                      value="maximized"
                      control={<Radio />}
                      label={launchModeLabels.maximized}
                    />
                    <FormControlLabel
                      value="focused"
                      control={<Radio />}
                      label={launchModeLabels.focused}
                    />
                  </RadioGroup>
                </div>
              </div>

              <div className="rounded-[24px] border border-[color:color-mix(in_srgb,var(--cp-border)_84%,transparent)] bg-[color:color-mix(in_srgb,var(--cp-surface)_90%,transparent)] px-4 py-4">
                <p className="shell-kicker">{t('demos.scale')}</p>
                <div className="mt-4 px-1">
                  <Slider
                    value={scale}
                    min={20}
                    max={100}
                    onChange={(_, value) => setScale(value as number)}
                  />
                </div>
                <div className="mt-4 rounded-[20px] bg-[color:color-mix(in_srgb,var(--cp-accent-soft)_12%,var(--cp-surface))] px-4 py-3">
                  <p className="text-sm text-[color:var(--cp-muted)]">{t('demos.progressValue')}</p>
                  <p className="mt-1 font-display text-3xl font-semibold text-[color:var(--cp-text)]">
                    {scale}%
                  </p>
                </div>
              </div>
            </div>
          </DemoSection>

          <DemoSection title={t('demos.feedback')} body={t('demos.feedbackBody')}>
            <Tabs
              value={tab}
              onChange={(_, value) => setTab(value)}
              variant={isCompact ? 'scrollable' : 'fullWidth'}
              allowScrollButtonsMobile
            >
              <Tab label={t('demos.tab.alerts')} />
              <Tab label={t('demos.tab.status')} />
              <Tab label={t('demos.tab.preview')} />
            </Tabs>

            <div className="mt-4">
              {tab === 0 ? (
                <div className="space-y-3">
                  <Alert severity="info">{t('demos.alert.info')}</Alert>
                  <Alert severity="success">{t('demos.alert.success')}</Alert>
                  <Alert severity="warning">{t('demos.alert.warning')}</Alert>
                  <Alert severity="error">{t('demos.alert.error')}</Alert>
                </div>
              ) : null}

              {tab === 1 ? (
                <div className="space-y-4">
                  <div className="rounded-[22px] border border-[color:color-mix(in_srgb,var(--cp-border)_84%,transparent)] bg-[color:color-mix(in_srgb,var(--cp-surface)_88%,transparent)] px-4 py-4">
                    <div className="flex items-center justify-between gap-3">
                      <p className="text-sm font-medium text-[color:var(--cp-text)]">
                        {t('demos.progressLabel')}
                      </p>
                      <span className="text-sm font-semibold text-[color:var(--cp-muted)]">
                        {readiness}%
                      </span>
                    </div>
                    <LinearProgress
                      className="!mt-4"
                      variant="determinate"
                      value={readiness}
                    />
                  </div>
                  <div className="flex flex-wrap gap-2">
                    <Chip label={densityLabels[density]} />
                    <Chip label={launchModeLabels[launchMode]} />
                    <Chip label={offlineCache ? t('demos.selection.offline') : t('demos.quickMenu')} />
                  </div>
                </div>
              ) : null}

              {tab === 2 ? (
                <div className="rounded-[24px] border border-[color:color-mix(in_srgb,var(--cp-border)_84%,transparent)] bg-[color:color-mix(in_srgb,var(--cp-surface)_92%,transparent)] p-4">
                  <div className="flex items-start gap-4">
                    <span
                      className="flex h-12 w-12 shrink-0 items-center justify-center rounded-[18px] shadow-[0_8px_20px_color-mix(in_srgb,var(--cp-shadow)_10%,transparent)]"
                      style={appIconSurfaceStyle('var(--cp-accent-soft)', 'window')}
                    >
                      <AppIcon iconKey="demos" className="text-white" />
                    </span>
                    <div className="min-w-0">
                      <p className="font-display text-lg font-semibold text-[color:var(--cp-text)]">
                        {query || t('apps.demos')}
                      </p>
                      <p className="mt-1 text-sm leading-6 text-[color:var(--cp-muted)]">
                        {t('demos.previewTitle')}
                      </p>
                    </div>
                  </div>

                  <p className="mt-4 rounded-[20px] bg-[color:color-mix(in_srgb,var(--cp-surface-2)_88%,transparent)] px-4 py-3 text-sm leading-6 text-[color:var(--cp-muted)]">
                    {notes}
                  </p>

                  <div className="mt-4 grid gap-2 sm:grid-cols-2">
                    <div className="rounded-[18px] bg-[color:color-mix(in_srgb,var(--cp-accent-soft)_12%,var(--cp-surface))] px-3 py-3 text-sm">
                      <span className="text-[color:var(--cp-muted)]">{t('demos.ownerLabel')}</span>
                      <p className="mt-1 font-medium text-[color:var(--cp-text)]">{owner}</p>
                    </div>
                    <div className="rounded-[18px] bg-[color:color-mix(in_srgb,var(--cp-accent-soft)_12%,var(--cp-surface))] px-3 py-3 text-sm">
                      <span className="text-[color:var(--cp-muted)]">{t('demos.densityLabel')}</span>
                      <p className="mt-1 font-medium text-[color:var(--cp-text)]">
                        {densityLabels[density]}
                      </p>
                    </div>
                    <div className="rounded-[18px] bg-[color:color-mix(in_srgb,var(--cp-accent-soft)_12%,var(--cp-surface))] px-3 py-3 text-sm">
                      <span className="text-[color:var(--cp-muted)]">{t('demos.stateLabel')}</span>
                      <p className="mt-1 font-medium text-[color:var(--cp-text)]">
                        {releaseLabels[releaseState]}
                      </p>
                    </div>
                    <div className="rounded-[18px] bg-[color:color-mix(in_srgb,var(--cp-accent-soft)_12%,var(--cp-surface))] px-3 py-3 text-sm">
                      <span className="text-[color:var(--cp-muted)]">{t('demos.launchMode')}</span>
                      <p className="mt-1 font-medium text-[color:var(--cp-text)]">
                        {launchModeLabels[launchMode]}
                      </p>
                    </div>
                  </div>
                </div>
              ) : null}
            </div>
          </DemoSection>
        </div>
      </div>
    </div>
  )
}
