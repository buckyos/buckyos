import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useState,
  type PropsWithChildren,
} from 'react'
import {
  applyDictionaryToModels,
  applyImportPlan,
  applyOpsBulkOperation,
  applyLogicalDirectoryMounts,
  deleteModelRule,
  deleteDictionaryItem,
  deleteMetadataVariant,
  deleteMetadataVersionRule,
  deleteNickRule,
  deleteOriginMappingRule,
  deleteOriginProviderAlias,
  deleteProvider,
  discardImportPlanDraft,
  loadProviderCloudWorkspace,
  previewPublish,
  publishPendingChanges,
  refreshPublishBaseRevision,
  restoreImportPlanDraft,
  saveImportPlanDraft,
  saveDictionaryItem,
  saveLogicalDirectory,
  saveMetadataVariant,
  saveMetadataVersionRule,
  saveModelOpsOverlay,
  saveModelRule,
  saveNickRule,
  saveOriginMappingRule,
  saveOriginProviderAlias,
  saveProviderOpsOverlay,
  saveProvider,
  saveProviderWizard,
  saveResolverOpsOverlay,
  saveTechSource,
  saveResolverRule,
  saveSelectionRule,
  simulateRevisionConflict,
  simulateTechSourceStale,
  startEditSession,
  syncTechSource,
  testTechSourceConnection,
} from '../mock/api'
import type {
  DeleteModelRuleInput,
  DictionaryBulkApplyInput,
  DictionaryItemInput,
  ImportPlanDraftInput,
  ImportPlanInput,
  LogicalDirectoryInput,
  LogicalDirectoryMountInput,
  MetadataVariantInput,
  MetadataVersionRuleInput,
  ModelOpsInput,
  OpsBulkOperationInput,
  ModelRuleInput,
  NickRuleInput,
  OriginMappingRuleInput,
  OriginProviderAliasInput,
  ProviderOpsInput,
  ProviderInput,
  ProviderWizardInput,
  PublishWizardInput,
  ResolverOpsOverlayInput,
  ResolverRuleInput,
  SelectionRuleInput,
  TechSourceInput,
} from '../datamodel/schemas'
import type { DataState, ProviderCloudSeed, PublishPreview, ServiceRole, ViewMode } from '../datamodel/types'

interface ProviderMetadataStoreValue {
  workspace: DataState<ProviderCloudSeed>
  publishPreview: PublishPreview | null
  serviceRole: ServiceRole
  viewMode: ViewMode
  setServiceRole: (role: ServiceRole) => void
  setViewMode: (mode: ViewMode) => void
  reload: () => Promise<void>
  enterEdit: () => Promise<void>
  runPublishPreview: () => Promise<PublishPreview>
  upsertProvider: (input: ProviderInput) => Promise<void>
  runProviderWizard: (input: ProviderWizardInput) => Promise<void>
  upsertModelRule: (input: ModelRuleInput) => Promise<void>
  removeModelRule: (input: DeleteModelRuleInput) => Promise<void>
  removeProvider: (providerKey: string) => Promise<void>
  upsertSelectionRule: (input: SelectionRuleInput) => Promise<void>
  upsertNickRule: (input: NickRuleInput) => Promise<void>
  removeNickRule: (nickKey: string) => Promise<void>
  upsertOriginProviderAlias: (input: OriginProviderAliasInput) => Promise<void>
  removeOriginProviderAlias: (aliasKey: string) => Promise<void>
  upsertOriginMappingRule: (input: OriginMappingRuleInput) => Promise<void>
  removeOriginMappingRule: (mappingKey: string) => Promise<void>
  upsertResolverRule: (input: ResolverRuleInput) => Promise<void>
  upsertMetadataVariant: (input: MetadataVariantInput) => Promise<void>
  removeMetadataVariant: (variantKey: string) => Promise<void>
  upsertMetadataVersionRule: (input: MetadataVersionRuleInput) => Promise<void>
  removeMetadataVersionRule: (versionRuleKey: string) => Promise<void>
  upsertLogicalDirectory: (input: LogicalDirectoryInput) => Promise<void>
  applyDirectoryMounts: (input: LogicalDirectoryMountInput) => Promise<void>
  upsertDictionaryItem: (input: DictionaryItemInput) => Promise<void>
  removeDictionaryItem: (kind: 'api_type' | 'capability', key: string) => Promise<void>
  applyDictionaryTag: (input: DictionaryBulkApplyInput) => Promise<void>
  importPlan: (input: ImportPlanInput) => Promise<void>
  saveImportDraft: (input: ImportPlanDraftInput) => Promise<void>
  restoreImportDraft: () => Promise<void>
  discardImportDraft: () => Promise<void>
  simulateConflict: () => Promise<void>
  refreshPublishBase: () => Promise<void>
  completePublish: (input: PublishWizardInput) => Promise<void>
  configureTechSource: (input: TechSourceInput) => Promise<void>
  testTechSource: () => Promise<void>
  syncSource: () => Promise<void>
  upsertProviderOps: (input: ProviderOpsInput) => Promise<void>
  upsertModelOps: (input: ModelOpsInput) => Promise<void>
  upsertResolverOps: (input: ResolverOpsOverlayInput) => Promise<void>
  applyBulkOperation: (input: OpsBulkOperationInput) => Promise<void>
  markTechSourceStale: () => Promise<void>
}

const StoreContext = createContext<ProviderMetadataStoreValue | null>(null)

export function ProviderMetadataStoreProvider({ children }: PropsWithChildren) {
  const [workspace, setWorkspace] = useState<DataState<ProviderCloudSeed>>({
    status: 'idle',
    data: null,
    error: null,
  })
  const [publishPreviewState, setPublishPreviewState] = useState<PublishPreview | null>(null)
  const [serviceRole, setServiceRole] = useState<ServiceRole>('tech')
  const [viewMode, setViewMode] = useState<ViewMode>('browse')

  const reload = useCallback(async () => {
    setWorkspace({ status: 'loading', data: null, error: null })
    try {
      const data = await loadProviderCloudWorkspace()
      setWorkspace({ status: 'success', data, error: null })
    } catch (error) {
      setWorkspace({
        status: 'error',
        data: null,
        error: error instanceof Error ? error.message : String(error),
      })
    }
  }, [])

  useEffect(() => {
    void reload()
  }, [reload])

  const enterEdit = useCallback(async () => {
    const data = await startEditSession(serviceRole)
    setWorkspace({ status: 'success', data, error: null })
    setViewMode('edit')
  }, [serviceRole])

  const runPublishPreview = useCallback(async () => {
    const preview = await previewPublish()
    setPublishPreviewState(preview)
    setViewMode('edit')
    return preview
  }, [])

  const applyWorkspaceUpdate = useCallback((data: ProviderCloudSeed) => {
    setWorkspace({ status: 'success', data, error: null })
    setViewMode('edit')
  }, [])

  const upsertProvider = useCallback(async (input: ProviderInput) => {
    applyWorkspaceUpdate(await saveProvider(input))
  }, [applyWorkspaceUpdate])

  const runProviderWizard = useCallback(async (input: ProviderWizardInput) => {
    applyWorkspaceUpdate(await saveProviderWizard(input))
  }, [applyWorkspaceUpdate])

  const upsertModelRule = useCallback(async (input: ModelRuleInput) => {
    applyWorkspaceUpdate(await saveModelRule(input))
  }, [applyWorkspaceUpdate])

  const removeModelRule = useCallback(async (input: DeleteModelRuleInput) => {
    applyWorkspaceUpdate(await deleteModelRule(input))
  }, [applyWorkspaceUpdate])

  const removeProvider = useCallback(async (providerKey: string) => {
    applyWorkspaceUpdate(await deleteProvider(providerKey))
  }, [applyWorkspaceUpdate])

  const upsertSelectionRule = useCallback(async (input: SelectionRuleInput) => {
    applyWorkspaceUpdate(await saveSelectionRule(input))
  }, [applyWorkspaceUpdate])

  const upsertNickRule = useCallback(async (input: NickRuleInput) => {
    applyWorkspaceUpdate(await saveNickRule(input))
  }, [applyWorkspaceUpdate])

  const removeNickRule = useCallback(async (nickKey: string) => {
    applyWorkspaceUpdate(await deleteNickRule(nickKey))
  }, [applyWorkspaceUpdate])

  const upsertOriginProviderAlias = useCallback(async (input: OriginProviderAliasInput) => {
    applyWorkspaceUpdate(await saveOriginProviderAlias(input))
  }, [applyWorkspaceUpdate])

  const removeOriginProviderAlias = useCallback(async (aliasKey: string) => {
    applyWorkspaceUpdate(await deleteOriginProviderAlias(aliasKey))
  }, [applyWorkspaceUpdate])

  const upsertOriginMappingRule = useCallback(async (input: OriginMappingRuleInput) => {
    applyWorkspaceUpdate(await saveOriginMappingRule(input))
  }, [applyWorkspaceUpdate])

  const removeOriginMappingRule = useCallback(async (mappingKey: string) => {
    applyWorkspaceUpdate(await deleteOriginMappingRule(mappingKey))
  }, [applyWorkspaceUpdate])

  const upsertResolverRule = useCallback(async (input: ResolverRuleInput) => {
    applyWorkspaceUpdate(await saveResolverRule(input))
  }, [applyWorkspaceUpdate])

  const upsertMetadataVariant = useCallback(async (input: MetadataVariantInput) => {
    applyWorkspaceUpdate(await saveMetadataVariant(input))
  }, [applyWorkspaceUpdate])

  const removeMetadataVariant = useCallback(async (variantKey: string) => {
    applyWorkspaceUpdate(await deleteMetadataVariant(variantKey))
  }, [applyWorkspaceUpdate])

  const upsertMetadataVersionRule = useCallback(async (input: MetadataVersionRuleInput) => {
    applyWorkspaceUpdate(await saveMetadataVersionRule(input))
  }, [applyWorkspaceUpdate])

  const removeMetadataVersionRule = useCallback(async (versionRuleKey: string) => {
    applyWorkspaceUpdate(await deleteMetadataVersionRule(versionRuleKey))
  }, [applyWorkspaceUpdate])

  const upsertLogicalDirectory = useCallback(async (input: LogicalDirectoryInput) => {
    applyWorkspaceUpdate(await saveLogicalDirectory(input))
  }, [applyWorkspaceUpdate])

  const applyDirectoryMounts = useCallback(async (input: LogicalDirectoryMountInput) => {
    applyWorkspaceUpdate(await applyLogicalDirectoryMounts(input))
  }, [applyWorkspaceUpdate])

  const upsertDictionaryItem = useCallback(async (input: DictionaryItemInput) => {
    applyWorkspaceUpdate(await saveDictionaryItem(input))
  }, [applyWorkspaceUpdate])

  const removeDictionaryItem = useCallback(async (kind: 'api_type' | 'capability', key: string) => {
    applyWorkspaceUpdate(await deleteDictionaryItem(kind, key))
  }, [applyWorkspaceUpdate])

  const applyDictionaryTag = useCallback(async (input: DictionaryBulkApplyInput) => {
    applyWorkspaceUpdate(await applyDictionaryToModels(input))
  }, [applyWorkspaceUpdate])

  const importPlan = useCallback(async (input: ImportPlanInput) => {
    applyWorkspaceUpdate(await applyImportPlan(input))
  }, [applyWorkspaceUpdate])

  const saveImportDraft = useCallback(async (input: ImportPlanDraftInput) => {
    applyWorkspaceUpdate(await saveImportPlanDraft(input))
  }, [applyWorkspaceUpdate])

  const restoreImportDraft = useCallback(async () => {
    applyWorkspaceUpdate(await restoreImportPlanDraft())
  }, [applyWorkspaceUpdate])

  const discardImportDraft = useCallback(async () => {
    applyWorkspaceUpdate(await discardImportPlanDraft())
  }, [applyWorkspaceUpdate])

  const simulateConflict = useCallback(async () => {
    applyWorkspaceUpdate(await simulateRevisionConflict())
  }, [applyWorkspaceUpdate])

  const refreshPublishBase = useCallback(async () => {
    applyWorkspaceUpdate(await refreshPublishBaseRevision())
  }, [applyWorkspaceUpdate])

  const completePublish = useCallback(async (input: PublishWizardInput) => {
    applyWorkspaceUpdate(await publishPendingChanges(input))
    setPublishPreviewState(null)
    setViewMode('browse')
  }, [applyWorkspaceUpdate])

  const configureTechSource = useCallback(async (input: TechSourceInput) => {
    applyWorkspaceUpdate(await saveTechSource(input))
  }, [applyWorkspaceUpdate])

  const testTechSource = useCallback(async () => {
    applyWorkspaceUpdate(await testTechSourceConnection())
  }, [applyWorkspaceUpdate])

  const syncSource = useCallback(async () => {
    applyWorkspaceUpdate(await syncTechSource())
  }, [applyWorkspaceUpdate])

  const markTechSourceStale = useCallback(async () => {
    applyWorkspaceUpdate(await simulateTechSourceStale())
  }, [applyWorkspaceUpdate])

  const upsertProviderOps = useCallback(async (input: ProviderOpsInput) => {
    applyWorkspaceUpdate(await saveProviderOpsOverlay(input))
  }, [applyWorkspaceUpdate])

  const upsertModelOps = useCallback(async (input: ModelOpsInput) => {
    applyWorkspaceUpdate(await saveModelOpsOverlay(input))
  }, [applyWorkspaceUpdate])

  const upsertResolverOps = useCallback(async (input: ResolverOpsOverlayInput) => {
    applyWorkspaceUpdate(await saveResolverOpsOverlay(input))
  }, [applyWorkspaceUpdate])

  const applyBulkOperation = useCallback(async (input: OpsBulkOperationInput) => {
    applyWorkspaceUpdate(await applyOpsBulkOperation(input))
  }, [applyWorkspaceUpdate])

  const value = useMemo<ProviderMetadataStoreValue>(() => {
    return {
      workspace,
      publishPreview: publishPreviewState,
      serviceRole,
      viewMode,
      setServiceRole,
      setViewMode,
      reload,
      enterEdit,
      runPublishPreview,
      upsertProvider,
      runProviderWizard,
      upsertModelRule,
      removeModelRule,
      removeProvider,
      upsertSelectionRule,
      upsertNickRule,
      removeNickRule,
      upsertOriginProviderAlias,
      removeOriginProviderAlias,
      upsertOriginMappingRule,
      removeOriginMappingRule,
      upsertResolverRule,
      upsertMetadataVariant,
      removeMetadataVariant,
      upsertMetadataVersionRule,
      removeMetadataVersionRule,
      upsertLogicalDirectory,
      applyDirectoryMounts,
      upsertDictionaryItem,
      removeDictionaryItem,
      applyDictionaryTag,
      importPlan,
      saveImportDraft,
      restoreImportDraft,
      discardImportDraft,
      simulateConflict,
      refreshPublishBase,
      completePublish,
      configureTechSource,
      testTechSource,
      syncSource,
      upsertProviderOps,
      upsertModelOps,
      upsertResolverOps,
      applyBulkOperation,
      markTechSourceStale,
    }
  }, [
    workspace,
    publishPreviewState,
    serviceRole,
    viewMode,
    reload,
    enterEdit,
    runPublishPreview,
    upsertProvider,
    runProviderWizard,
    upsertModelRule,
    removeModelRule,
    removeProvider,
    upsertSelectionRule,
    upsertNickRule,
    removeNickRule,
    upsertOriginProviderAlias,
    removeOriginProviderAlias,
    upsertOriginMappingRule,
    removeOriginMappingRule,
    upsertResolverRule,
    upsertMetadataVariant,
    removeMetadataVariant,
    upsertMetadataVersionRule,
    removeMetadataVersionRule,
    upsertLogicalDirectory,
    applyDirectoryMounts,
    upsertDictionaryItem,
    removeDictionaryItem,
    applyDictionaryTag,
    importPlan,
    saveImportDraft,
    restoreImportDraft,
    discardImportDraft,
    simulateConflict,
    refreshPublishBase,
    completePublish,
    configureTechSource,
    testTechSource,
    syncSource,
    markTechSourceStale,
    upsertProviderOps,
    upsertModelOps,
    upsertResolverOps,
    applyBulkOperation,
  ])

  return <StoreContext.Provider value={value}>{children}</StoreContext.Provider>
}

export function useProviderMetadataStore() {
  const context = useContext(StoreContext)
  if (!context) {
    throw new Error('useProviderMetadataStore must be used within ProviderMetadataStoreProvider')
  }
  return context
}
