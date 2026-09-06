export const appServiceEn: Record<string, string> = {
  'app22.referrer': 'Referred by',
  'app22.error.TASK_TYPE_UNSUPPORTED':
    'This task is not an installation or upgrade task.',
  'sudo.account': 'Account',
  'sudo.audience': 'Scope',
  'sudo.password': 'Password',
  'sudo.hidePassword': 'Hide password',
  'sudo.showPassword': 'Show password',
  'sudo.tokenHint':
    'Authorization is used for this operation only and expires in about 3 minutes.',
  'sudo.requesting': 'Requesting…',
  'sudo.title': 'Administrator permission',
  'sudo.descriptionWithAudience':
    'Confirm your password to grant temporary sudo access for {{aud}}.',
  'sudo.description': 'Confirm your password to grant temporary sudo access.',
  'sudo.confirm': 'Grant sudo',

  'app22.title': 'Install application',
  'app22.draftHint':
    'Preparing an installation. No execution task has been created.',
  'app22.taskHint': 'This task can be reopened using its task ID.',
  'app22.source.intro':
    'Choose an application package, then review its identity and installation requirements.',
  'app22.source.choose': 'Choose application package',
  'app22.source.device': 'Choose from this device',
  'app22.source.server': 'Choose from Personal Server',
  'app22.source.mockFiles':
    'Prototype: Personal Server contains sample packages. Local import accepts the documented Mock PIKG fixture; real package upload is pending integration.',
  'app22.source.identifier': 'Enter a link or application identifier',
  'app22.source.check': 'Check source',
  'app22.source.advanced': 'Advanced import',
  'app22.source.advancedHint':
    'AppDoc v1 text and canonical Object IDs belong here. Import requires local content and independent verification; parsing text does not establish trust.',
  'app22.source.advancedInput': 'AppDoc text or Object ID',
  'app22.source.preparing': 'Preparing file reference…',
  'app22.source.importing': 'Importing application package…',
  'app22.source.continue': 'Inspect application',
  'app22.source.label': 'Source',
  'app22.source.kind.identifier': 'Application identifier',
  'app22.source.kind.local-pikg': 'Local package',
  'app22.source.kind.personal-server-pikg': 'Personal Server package',
  'app22.source.kind.url': 'Link import',
  'app22.source.kind.appdoc': 'AppDoc candidate',
  'app22.owner': 'Installation account',
  'app22.target': 'Target device',
  'app22.platform': 'Platform',
  'app22.address': 'Application address',
  'app22.assigned': 'Assigned by the system',
  'app22.unknown': 'Unknown / unavailable',
  'app22.check.publisher': 'Publication source',
  'app22.check.identity': 'Application identity details',
  'app22.check.objectOwner': 'Publication object owner',
  'app22.check.envelope':
    'Signature and publication are checked independently. The protocol specifies a detached envelope; the current PIKG reader also handles APPDOC.jwt. Format alignment is a later integration task.',
  'app22.check.evidence': 'Verification details',
  'app22.check.document': 'Document validity',
  'app22.check.signature': 'Signature verification',
  'app22.check.owner': 'Controller / owner constraints',
  'app22.check.authority': 'Authoritative publication',
  'app22.check.content': 'Required content',
  'app22.check.target': 'Target compatibility',
  'app22.check.config': 'Configuration',
  'app22.check.publication': 'Publication state',
  'app22.check.localAuthority':
    'Using local development authorization for this package. This does not establish public publication or public trust.',
  'app22.check.inspecting': 'Resolving identity and checking application…',
  'app22.document.Active': 'Published',
  'app22.document.Missing': 'Not publicly published',
  'app22.document.Expired': 'Publication expired',
  'app22.document.Revoked': 'Identity revoked',
  'app22.document.Tombstoned': 'Identity terminated',
  'app22.document.Migrated': 'Identity migrated',
  'app22.document.Unknown': 'Publication unknown',
  'app22.evidence.READY': 'Verified / ready',
  'app22.evidence.NOT_READY': 'Not ready',
  'app22.evidence.UNKNOWN': 'Unknown',
  'app22.readiness.OFFLINE_READY': 'Ready without network acquisition',
  'app22.readiness.CONTENT_DOWNLOAD_REQUIRED':
    'Download required before configuration is committed',
  'app22.readiness.BLOCKED': 'Resolve the following issues before continuing',
  'app22.readiness.UNKNOWN': 'Readiness unknown',
  'app22.fix.recheck': 'Check authoritative evidence again',
  'app22.fix.change-source': 'Choose a valid source',
  'app22.fix.edit': 'Adjust the indicated option and recheck',
  'app22.suggestions':
    'Caller-provided options are suggestions. Review and explicitly confirm the resulting plan.',
  'app22.satisfied': 'Already installed',
  'app22.satisfiedBody':
    'The same application content is installed for this account. No new task is needed.',
  'app22.upgrade': 'Upgrade available',
  'app22.upgradeImpact':
    'The new version replaces the current deployment and may interrupt service. Account-owned data remains isolated and stable across the upgrade.',
  'app22.configure': 'Review installation plan',
  'app22.changeSource': 'Change source',
  'app22.viewApp': 'View application',
  'app22.installPlan': 'Installation plan',
  'app22.upgradePlan': 'Upgrade plan',
  'app22.advancedTarget': 'Advanced target selection',
  'app22.offline': 'Offline mode: prohibit network acquisition',
  'app22.policy': 'Installation policy',
  'app22.policy.NORMAL': 'Public installation',
  'app22.policy.LOCAL_DEVELOPER': 'Local development installation',
  'app22.components': 'Components',
  'app22.required': 'Required',
  'app22.optional': 'Optional',
  'app22.services': 'Service access',
  'app22.exposure': 'Exposure scope',
  'app22.zoneOnly': 'Zone members',
  'app22.public': 'Public network',
  'app22.port': 'Exposed port',
  'app22.guest': 'Allow guests',
  'app22.shortcutLater':
    'Shortcut domains are a separate setting after installation. They are not included in this plan.',
  'app22.permissions': 'Declared permissions',
  'app22.storage': 'Storage and data ownership',
  'app22.mount.data': 'Persistent application data',
  'app22.mount.dataHint':
    'Owned by the installation account and isolated by account and App. The logical directory remains stable across upgrades.',
  'app22.mount.local_cache': 'Node cache',
  'app22.mount.local_cacheHint':
    'Disposable data on the selected device. It is not the durable application data directory.',
  'app22.mount.external': 'External directory',
  'app22.mount.externalHint':
    'Optional access to an existing user directory. Original ownership is preserved; access follows the declared permission.',
  'app22.access.read_only': 'Read only',
  'app22.access.read_write': 'Read and write',
  'app22.access.read_write_append': 'Read and append',
  'app22.environment': 'Environment variables',
  'app22.systemInjected': 'Injected by BuckyOS; cannot be overridden',
  'app22.risky': 'High risk launch parameters · read only',
  'app22.riskyHint':
    'Declared by the application. Editing is unavailable without a validated configuration contract.',
  'app22.autoStart': 'Start automatically after deployment',
  'app22.authorize': 'Administrator permission',
  'app22.authReason':
    'Authorize the reviewed installation plan for {{name}}. Mock password: prototype-admin.',
  'app22.authConfirm': 'Authorize this plan',
  'app22.recheck': 'Recheck installation plan',
  'app22.rechecking': 'Checking the updated plan…',
  'app22.dirty': 'Options changed. Recheck the plan before confirming.',
  'app22.finalSummary': 'Final confirmation',
  'app22.version': 'Version',
  'app22.download': 'Required download',
  'app22.yes': 'Yes',
  'app22.no': 'No',
  'app22.installImpact':
    'The selected services, permissions and storage mappings will be committed. Deployment and startup are checked separately.',
  'app22.diagnostics': 'Diagnostic details',
  'app22.submitting': 'Authorizing / submitting…',
  'app22.confirmInstall': 'Confirm and install',
  'app22.confirmUpgrade': 'Confirm and upgrade',
  'app22.installComplete': 'Installation configuration published',
  'app22.installFailed': 'Installation stopped',
  'app22.installCanceled': 'Installation canceled',
  'app22.installWaiting': 'Waiting to continue',
  'app22.executing': 'Installing application',
  'app22.previousAttempt': 'Previous attempt',
  'app22.waitingContent': 'Content preparation is paused. Resume when ready.',
  'app22.stage.acquire': 'Preparing application content',
  'app22.stage.verify': 'Checking content integrity',
  'app22.stage.prepare': 'Preparing the approved configuration',
  'app22.stage.deploy': 'Committing application configuration',
  'app22.stage.activate': 'Publishing deployment configuration to the device',
  'app22.stage.unknown': 'Waiting for a known execution stage',
  'app22.progress': 'Installation progress',
  'app22.committed':
    'Configuration has been committed and cannot be canceled. You can follow deployment progress in the background.',
  'app22.installResult': 'Installation submission result',
  'app22.runtimeResult': 'Deployment and runtime status',
  'app22.approvedPlan': 'Approved plan and application snapshot',
  'app22.cancelTask': 'Cancel installation',
  'app22.resume': 'Resume',
  'app22.copyDetails': 'Copy safe details',
  'app22.taskCenter': 'Open in Task Center',
  'app22.background': 'Run in background',
  'app22.viewTask': 'View task',
  'app22.restoreDraft': 'Inspect draft again',
  'app22.visibility.admin':
    'Administrator view: installations visible to this account, including the Mock management inventory. A complete Zone management API is pending integration.',
  'app22.visibility.user':
    'Your installations and authorized service information. Administrative installation and runtime controls are unavailable.',
  'app22.settingsReadOnly':
    'Settings are read only in this prototype. Saving and application of changes require a defined application settings API.',
  'app22.processHealth': 'Process health',
  'app22.webAccessible': 'Deployment accessibility',
  'app22.agentEnvironment': 'Runtime environment',
  'app22.noBinding':
    'There is no Agent binding yet. Installing this runtime does not create an Agent.',
  'app22.bindingCount': 'Agent bindings: {{count}}',
  'app22.runtimeType.docker': 'Docker application',
  'app22.runtimeType.script': 'Script application',
  'app22.runtimeType.web': 'Static Web application',
  'app22.runtimeType.agent': 'Agent runtime',
  'app22.runtimeType.unknown': 'Runtime type unavailable',
  'app22.status.installing': 'Preparing installation',
  'app22.status.deploying': 'Configuration submitted · deploying',
  'app22.status.starting': 'Installed · starting',
  'app22.status.running': 'Running normally',
  'app22.status.stopping': 'Stopping',
  'app22.status.stopped': 'Not started',
  'app22.status.activation_failed': 'Installed · startup failed',
  'app22.status.error': 'Operation failed',
  'app22.status.unknown': 'Runtime status unknown',
  'app22.error.UNKNOWN':
    'Information is unavailable. Inspect the application again.',
  'app22.error.INVALID_SOURCE':
    'Enter a supported application identifier or choose a package.',
  'app22.error.INVALID_PIKG':
    'The file content is not a valid Mock PIKG package. Renaming its extension does not make it valid.',
  'app22.error.PIKG_BRIDGE_REQUIRED':
    'Real PIKG upload and staging integration is pending. Use a documented Mock fixture for this prototype.',
  'app22.error.IMPORT_CANCELED':
    'File preparation was canceled. Choose a package to start again.',
  'app22.error.IMPORT_FAILED':
    'The package could not be imported. Choose the file again.',
  'app22.error.INVALID_URL':
    'Enter a valid HTTP(S) URL without embedded credentials.',
  'app22.error.URL_IMPORT_REQUIRED':
    'Link import is not connected. Control Panel does not fetch client URLs. Choose a package or application identifier.',
  'app22.error.APPDOC_IMPORT_REQUIRED':
    'AppDoc text import is pending integration. A parsed candidate is not trusted publication evidence.',
  'app22.error.INVALID_APPDOC':
    'The document is not a valid AppDoc v1 candidate.',
  'app22.error.INVALID_DID': 'Use a canonical lowercase hostname-form App DID.',
  'app22.error.OBJECT_NOT_LOCAL':
    'This Object ID has no local content in the prototype. Import the package first.',
  'app22.error.SOURCE_UNAVAILABLE':
    'This source has no Mock fixture. Choose a sample package or use a documented identifier.',
  'app22.error.REFERENCE_EXPIRED':
    'The file reference expired. Choose the package again.',
  'app22.error.LOCAL_DEVELOPER_REQUIRED':
    'This package is not publicly published. A local package requires explicit local development authorization.',
  'app22.error.LOCAL_PIKG_REQUIRED':
    'Local development authorization is restricted to LocalPikg sources.',
  'app22.error.TRUST_RESOLUTION_REQUIRED':
    'Authoritative identity evidence is unavailable. Recheck when the authority can be reached.',
  'app22.error.IDENTITY_REVOKED':
    'The application identity was revoked. Local development mode cannot override this state.',
  'app22.error.IDENTITY_TOMBSTONED':
    'The application identity was terminated. Installation is prohibited.',
  'app22.error.IDENTITY_MIGRATED':
    'The application identity migrated. Use its current authoritative identity.',
  'app22.error.SIGNATURE_INVALID':
    'The application signature could not be verified. Choose a valid package.',
  'app22.error.OWNER_MISMATCH':
    'The signer or controller does not satisfy the authoritative owner constraint.',
  'app22.error.UNSUPPORTED_TARGET':
    'The selected device does not support this application.',
  'app22.error.TARGET_OFFLINE':
    'The selected device is offline. Choose an available device.',
  'app22.error.OFFLINE_CONTENT_UNAVAILABLE':
    'Required content is unavailable offline. Supply a complete package or explicitly allow network acquisition.',
  'app22.error.UNKNOWN_TARGET':
    'Choose a device from the current node inventory.',
  'app22.error.REQUIRED_COMPONENT':
    'Required components must remain selected; undeclared components are not allowed.',
  'app22.error.DECLARED_PERMISSION':
    'Permissions must match the declaration exactly and include every required entry.',
  'app22.error.REQUIRED_ENDPOINT':
    'Keep required endpoints enabled and preserve their declared route type.',
  'app22.error.PORT_CONFLICT':
    'Two enabled endpoints cannot use the same exposed port.',
  'app22.error.GUEST_SCOPE':
    'Guest access requires public exposure. Disable guest access or change the scope.',
  'app22.error.DECLARED_MOUNT':
    'Use the declared directory mapping and access mode.',
  'app22.error.REQUIRED_MOUNT':
    'Required directory mappings cannot be removed.',
  'app22.error.SYSTEM_ENV':
    'Only declared environment variables can be edited. BuckyOS variables cannot be overridden.',
  'app22.error.REQUIRED_ENV':
    'Enter a value for this required environment variable.',
  'app22.error.CONFIG_CONFLICT':
    'The declared configuration conflicts with the target. Choose a corrected application package.',
  'app22.error.INVALID_FIELD':
    'Check the indicated field for a missing value, invalid format or range.',
  'app22.error.DOWNGRADE_NOT_SUPPORTED':
    'Downgrading from the installed version is not supported.',
  'app22.error.ADMIN_REQUIRED':
    'An administrator must authorize this operation.',
  'app22.error.INCORRECT_PASSWORD': 'Incorrect password. Please try again.',
  'app22.error.AUTH_EXPIRED':
    'Administrator authorization expired. Review the plan and authorize again.',
  'app22.error.PLAN_STALE':
    'Application information or installation conditions changed. Inspect again and confirm the updated plan.',
  'app22.error.DOWNLOAD_FAILED':
    'The content download failed. Retrying creates a linked new attempt.',
  'app22.error.SCHEDULING_FAILED':
    'Configuration was committed, but scheduling failed. Retry resumes deployment; cancellation is unavailable.',
  'app22.error.STARTUP_FAILED':
    'The installation is preserved, but startup failed. Review runtime diagnostics.',
  'app22.error.OFFLINE_NODE':
    'The device is offline. Runtime readiness is unknown.',
  'app22.error.RUNTIME_TIMEOUT':
    'No valid startup evidence arrived before the timeout.',
  'app22.error.RUNTIME_UNKNOWN':
    'No runtime report is available for this deployment.',
  'app22.error.STALE_EVIDENCE':
    'The runtime report belongs to a different deployment and cannot prove readiness.',
  'app22.error.OPERATION_FAILED':
    'The runtime operation failed. Refresh the runtime report before another action.',
  'app22.error.OPERATION_TIMEOUT':
    'The operation did not converge before timeout. Current runtime status is unknown.',
  'app22.error.TASK_NOT_FOUND':
    'Installation task not found. No replacement task was created.',
  'app22.error.TASK_FORBIDDEN':
    'You do not have permission to read this installation task.',
  'app22.error.DRAFT_NOT_FOUND':
    'This preparation draft is unavailable. Choose a source again.',
  'app22.error.duplicate_parameter':
    'A launch parameter was provided more than once.',
  'app22.error.unknown_parameter':
    'This entry does not accept the supplied parameter.',
  'app22.error.conflicting_parameters':
    'Provide exactly one identifier or task_id.',
  'app22.error.invalid_task_id':
    'The task ID must be a nonempty opaque string without whitespace.',
  'app22.error.invalid_identifier':
    'The public entry requires an application identifier; file contents and internal references are not accepted.',
  'app22.error.invalid_options': 'Initial options are invalid or too large.',
  'app22.error.invalid_target':
    'The target is missing or its node ID and DID do not match.',
  'app22.description.nextcloud':
    'Private file sync, calendar and collaboration for your Zone.',
  'app22.description.paperless':
    'Organize and search your private document library.',
  'app22.description.home-dashboard':
    'A static dashboard hosted on your Personal Server.',
  'app22.description.backup-script':
    'Run a local backup process for application data.',
  'app22.description.opendan':
    'An environment for running Agents with separate bindings.',
  'app22.description.nostr-relay': 'Decentralized social relay for this Zone.',
  'app22.description.home-assistant':
    'Local automation for devices and routines in your home.',
  'app22.description.gateway': 'Zone routing and HTTPS termination.',
  'app22.description.scheduler':
    'Derives deployment configuration for this Zone.',
  'app22.description.verify-hub': 'Unified sign-in and session token issuance.',
  'app22.description.node-daemon':
    'Converges the node to its assigned configuration.',
  'app22.description.kmsg': 'Kernel message queue.',
}

export const appServiceZh: Record<string, string> = {
  'app22.referrer': '推荐来源',
  'app22.error.TASK_TYPE_UNSUPPORTED': '此任务不是应用安装或升级任务。',
  'sudo.account': '账号',
  'sudo.audience': '授权范围',
  'sudo.password': '密码',
  'sudo.hidePassword': '隐藏密码',
  'sudo.showPassword': '显示密码',
  'sudo.tokenHint': '授权仅用于本次操作，约 3 分钟后过期。',
  'sudo.requesting': '正在请求…',
  'sudo.title': '管理员授权',
  'sudo.descriptionWithAudience': '确认密码，为 {{aud}} 授予临时 sudo 权限。',
  'sudo.description': '确认密码以授予临时 sudo 权限。',
  'sudo.confirm': '授予 sudo 权限',

  'app22.title': '安装应用',
  'app22.draftHint': '正在准备安装，尚未创建执行任务。',
  'app22.taskHint': '可通过任务编号重新打开此任务。',
  'app22.source.intro': '选择应用包，再检查应用身份与安装条件。',
  'app22.source.choose': '选择应用包',
  'app22.source.device': '从本机选择',
  'app22.source.server': '从 Personal Server 选择',
  'app22.source.mockFiles':
    '原型：Personal Server 提供示例包。本地导入接受文档约定的 Mock PIKG 样本，真实包上传待接入。',
  'app22.source.identifier': '输入链接或应用标识',
  'app22.source.check': '检查来源',
  'app22.source.advanced': '高级导入',
  'app22.source.advancedHint':
    '在此输入 AppDoc v1 文本或 canonical Object ID。导入需本地内容及独立验证，文本可解析不代表可信。',
  'app22.source.advancedInput': 'AppDoc 文本或 Object ID',
  'app22.source.preparing': '正在准备文件引用…',
  'app22.source.importing': '正在导入应用包…',
  'app22.source.continue': '检查应用',
  'app22.source.label': '来源',
  'app22.source.kind.identifier': '应用标识',
  'app22.source.kind.local-pikg': '本地应用包',
  'app22.source.kind.personal-server-pikg': 'Personal Server 应用包',
  'app22.source.kind.url': '链接导入',
  'app22.source.kind.appdoc': 'AppDoc 候选文档',
  'app22.owner': '安装归属账号',
  'app22.target': '目标设备',
  'app22.platform': '平台',
  'app22.address': '应用地址',
  'app22.assigned': '由系统分配',
  'app22.unknown': '未知 / 不可用',
  'app22.check.publisher': '发布来源',
  'app22.check.identity': '应用身份详情',
  'app22.check.objectOwner': '发布对象所有者',
  'app22.check.envelope':
    '签名与权威发布分别检查。协议规定 detached envelope，当前 PIKG reader 也处理 APPDOC.jwt，格式对齐留待后续集成。',
  'app22.check.evidence': '校验详情',
  'app22.check.document': '文档有效性',
  'app22.check.signature': '签名校验',
  'app22.check.owner': 'controller / owner 约束',
  'app22.check.authority': '权威发布',
  'app22.check.content': '所需内容',
  'app22.check.target': '目标兼容性',
  'app22.check.config': '配置',
  'app22.check.publication': '发布状态',
  'app22.check.localAuthority':
    '此包使用本地开发授权，不代表已公开发布或获得公开信任。',
  'app22.check.inspecting': '正在解析身份并检查应用…',
  'app22.document.Active': '已发布',
  'app22.document.Missing': '未公开发布',
  'app22.document.Expired': '发布已过期',
  'app22.document.Revoked': '身份已撤销',
  'app22.document.Tombstoned': '身份已终止',
  'app22.document.Migrated': '身份已迁移',
  'app22.document.Unknown': '发布状态未知',
  'app22.evidence.READY': '已验证 / 已就绪',
  'app22.evidence.NOT_READY': '未就绪',
  'app22.evidence.UNKNOWN': '未知',
  'app22.readiness.OFFLINE_READY': '无需网络获取，已就绪',
  'app22.readiness.CONTENT_DOWNLOAD_REQUIRED': '提交配置前需要下载内容',
  'app22.readiness.BLOCKED': '请先处理以下问题',
  'app22.readiness.UNKNOWN': '就绪状态未知',
  'app22.fix.recheck': '重新检查权威证据',
  'app22.fix.change-source': '选择有效来源',
  'app22.fix.edit': '调整对应选项后重新检查',
  'app22.suggestions': '调用方选项仅为建议，请检查并明确确认最终计划。',
  'app22.satisfied': '已安装',
  'app22.satisfiedBody': '此账号已安装相同应用内容，无需创建新任务。',
  'app22.upgrade': '可以升级',
  'app22.upgradeImpact':
    '新版本将替换当前部署，服务可能短暂中断。账号数据保持隔离，跨升级稳定。',
  'app22.configure': '查看安装计划',
  'app22.changeSource': '更换来源',
  'app22.viewApp': '查看应用',
  'app22.installPlan': '安装计划',
  'app22.upgradePlan': '升级计划',
  'app22.advancedTarget': '高级目标选择',
  'app22.offline': '离线模式：禁止网络获取',
  'app22.policy': '安装策略',
  'app22.policy.NORMAL': '公开安装',
  'app22.policy.LOCAL_DEVELOPER': '本地开发安装',
  'app22.components': '组件',
  'app22.required': '必需',
  'app22.optional': '可选',
  'app22.services': '服务访问',
  'app22.exposure': '暴露范围',
  'app22.zoneOnly': 'Zone 成员',
  'app22.public': '公共网络',
  'app22.port': '暴露端口',
  'app22.guest': '允许访客访问',
  'app22.shortcutLater': '快捷域名在安装后独立设置，不属于本次安装计划。',
  'app22.permissions': '声明的权限',
  'app22.storage': '存储与数据归属',
  'app22.mount.data': '持久应用数据',
  'app22.mount.dataHint':
    '归安装账号所有，按用户与 App 隔离；逻辑目录跨升级稳定。',
  'app22.mount.local_cache': '节点缓存',
  'app22.mount.local_cacheHint':
    '所选设备上的可清理缓存，不用于持久保存应用数据。',
  'app22.mount.external': '外部目录',
  'app22.mount.externalHint':
    '可选访问已有用户目录，保留原归属，按声明权限访问。',
  'app22.access.read_only': '只读',
  'app22.access.read_write': '读写',
  'app22.access.read_write_append': '读取和追加',
  'app22.environment': '环境变量',
  'app22.systemInjected': '由 BuckyOS 注入，不可覆盖',
  'app22.risky': '高风险启动参数 · 只读',
  'app22.riskyHint': '由应用声明，未建立校验和编辑契约前不可修改。',
  'app22.autoStart': '部署后自动启动',
  'app22.authorize': '管理员授权',
  'app22.authReason':
    '授权已检查的 {{name}} 安装计划。Mock 密码：prototype-admin。',
  'app22.authConfirm': '授权此计划',
  'app22.recheck': '重新检查安装计划',
  'app22.rechecking': '正在检查更新后的计划…',
  'app22.dirty': '选项已变化，请重新检查计划后确认。',
  'app22.finalSummary': '最终确认',
  'app22.version': '版本',
  'app22.download': '所需下载',
  'app22.yes': '是',
  'app22.no': '否',
  'app22.installImpact':
    '将提交所选服务、权限和存储映射，部署与启动状态单独检查。',
  'app22.diagnostics': '诊断详情',
  'app22.submitting': '正在授权 / 提交…',
  'app22.confirmInstall': '确认并安装',
  'app22.confirmUpgrade': '确认并升级',
  'app22.installComplete': '安装配置已发布',
  'app22.installFailed': '安装已停止',
  'app22.installCanceled': '安装已取消',
  'app22.installWaiting': '等待继续执行',
  'app22.executing': '正在安装应用',
  'app22.previousAttempt': '上一次尝试',
  'app22.waitingContent': '内容准备已暂停，可在条件就绪后恢复。',
  'app22.stage.acquire': '正在准备应用内容',
  'app22.stage.verify': '正在校验内容完整性',
  'app22.stage.prepare': '正在准备已批准的配置',
  'app22.stage.deploy': '正在提交应用配置',
  'app22.stage.activate': '正在向设备发布部署配置',
  'app22.stage.unknown': '等待可识别的执行阶段',
  'app22.progress': '安装进度',
  'app22.committed': '配置已提交，无法取消，可后台查看部署进度。',
  'app22.installResult': '安装提交结果',
  'app22.runtimeResult': '部署与运行状态',
  'app22.approvedPlan': '已批准计划与应用快照',
  'app22.cancelTask': '取消安装',
  'app22.resume': '恢复',
  'app22.copyDetails': '复制安全诊断信息',
  'app22.taskCenter': '在任务中心查看',
  'app22.background': '后台执行',
  'app22.viewTask': '查看任务',
  'app22.restoreDraft': '重新检查草稿',
  'app22.visibility.admin':
    '管理员视图：本账号可见的安装，包含 Mock 管理清单。Zone 全量管理接口待接入。',
  'app22.visibility.user':
    '显示你的安装及获授权的服务信息，不提供管理员安装和运行控制。',
  'app22.settingsReadOnly':
    '本原型中的设置为只读，保存和生效需后续明确应用设置接口。',
  'app22.processHealth': '进程健康',
  'app22.webAccessible': '部署可访问状态',
  'app22.agentEnvironment': '运行环境',
  'app22.noBinding': '尚无 Agent binding；安装运行时不会创建 Agent。',
  'app22.bindingCount': 'Agent binding 数量：{{count}}',
  'app22.runtimeType.docker': 'Docker 应用',
  'app22.runtimeType.script': 'Script 应用',
  'app22.runtimeType.web': '静态 Web 应用',
  'app22.runtimeType.agent': 'Agent 运行时',
  'app22.runtimeType.unknown': '运行类型不可用',
  'app22.status.installing': '正在准备安装',
  'app22.status.deploying': '配置已提交 · 部署中',
  'app22.status.starting': '已安装 · 启动中',
  'app22.status.running': '运行正常',
  'app22.status.stopping': '正在停止',
  'app22.status.stopped': '未启动',
  'app22.status.activation_failed': '已安装 · 启动失败',
  'app22.status.error': '操作失败',
  'app22.status.unknown': '运行状态未知',
  'app22.error.UNKNOWN': '信息不可用，请重新检查应用。',
  'app22.error.INVALID_SOURCE': '请输入受支持的应用标识或选择应用包。',
  'app22.error.INVALID_PIKG':
    '文件内容不是有效的 Mock PIKG 包，修改后缀不能使其有效。',
  'app22.error.PIKG_BRIDGE_REQUIRED':
    '真实 PIKG 上传及 staging 桥接待实现，请使用文档提供的 Mock 样本。',
  'app22.error.IMPORT_CANCELED': '已取消文件准备，请重新选择应用包。',
  'app22.error.IMPORT_FAILED': '应用包导入失败，请重新选择文件。',
  'app22.error.INVALID_URL': '请输入不含内嵌凭据的有效 HTTP(S) 链接。',
  'app22.error.URL_IMPORT_REQUIRED':
    '链接导入尚未接入，Control Panel 不抓取客户端 URL。请选择应用包或输入应用标识。',
  'app22.error.APPDOC_IMPORT_REQUIRED':
    'AppDoc 文本导入待接入，可解析的候选文档不代表可信发布证据。',
  'app22.error.INVALID_APPDOC': '文档不是有效的 AppDoc v1 候选。',
  'app22.error.INVALID_DID': '请使用符合 hostname-form 规则的小写 App DID。',
  'app22.error.OBJECT_NOT_LOCAL':
    '原型本地没有此 Object ID 的内容，请先导入应用包。',
  'app22.error.SOURCE_UNAVAILABLE':
    '此来源没有 Mock 样本，请选择示例包或使用文档中的标识。',
  'app22.error.REFERENCE_EXPIRED': '文件引用已过期，请重新选择应用包。',
  'app22.error.LOCAL_DEVELOPER_REQUIRED':
    '此包尚未公开发布，本地包需明确选择本地开发授权。',
  'app22.error.LOCAL_PIKG_REQUIRED': '本地开发授权仅适用于 LocalPikg 来源。',
  'app22.error.TRUST_RESOLUTION_REQUIRED':
    '无法取得权威身份依据，请在权威源可达后重新检查。',
  'app22.error.IDENTITY_REVOKED':
    '应用身份已撤销，本地开发模式不能绕过此状态。',
  'app22.error.IDENTITY_TOMBSTONED': '应用身份已终止，禁止安装。',
  'app22.error.IDENTITY_MIGRATED': '应用身份已迁移，请使用当前权威身份。',
  'app22.error.SIGNATURE_INVALID': '应用签名校验失败，请选择有效应用包。',
  'app22.error.OWNER_MISMATCH': '签名者或 controller 不满足权威 owner 约束。',
  'app22.error.UNSUPPORTED_TARGET': '所选设备不支持此应用。',
  'app22.error.TARGET_OFFLINE': '所选设备离线，请选择可用设备。',
  'app22.error.OFFLINE_CONTENT_UNAVAILABLE':
    '离线模式缺少所需内容，请提供完整包或明确允许网络获取。',
  'app22.error.UNKNOWN_TARGET': '请从当前节点清单中选择设备。',
  'app22.error.REQUIRED_COMPONENT': '必需组件不可取消，不允许选择未声明组件。',
  'app22.error.DECLARED_PERMISSION': '权限需与声明完全一致，并包含所有必需项。',
  'app22.error.REQUIRED_ENDPOINT':
    '必需 endpoint 不可关闭，路由类型需遵循声明。',
  'app22.error.PORT_CONFLICT': '两个已启用的 endpoint 不能使用相同暴露端口。',
  'app22.error.GUEST_SCOPE': '访客访问需要公共暴露，请关闭访客访问或调整范围。',
  'app22.error.DECLARED_MOUNT': '请使用已声明的目录映射及访问模式。',
  'app22.error.REQUIRED_MOUNT': '不能移除必需目录映射。',
  'app22.error.SYSTEM_ENV': '仅可编辑已声明的环境变量，不可覆盖 BuckyOS 变量。',
  'app22.error.REQUIRED_ENV': '请填写此必需环境变量。',
  'app22.error.CONFIG_CONFLICT': '声明配置与目标冲突，请选择已修正的应用包。',
  'app22.error.INVALID_FIELD': '请检查对应字段的必填值、格式或范围。',
  'app22.error.DOWNGRADE_NOT_SUPPORTED': '不支持从当前已安装版本降级。',
  'app22.error.ADMIN_REQUIRED': '此操作需要管理员授权。',
  'app22.error.INCORRECT_PASSWORD': '密码错误，请重试。',
  'app22.error.AUTH_EXPIRED': '管理员授权已过期，请检查计划并重新授权。',
  'app22.error.PLAN_STALE':
    '应用信息或安装条件已变化，请重新检查并确认更新后的计划。',
  'app22.error.DOWNLOAD_FAILED': '内容下载失败，重试将创建关联的新尝试。',
  'app22.error.SCHEDULING_FAILED':
    '配置已提交，但调度失败。重试将恢复部署，不能取消。',
  'app22.error.STARTUP_FAILED': '安装结果已保留，但启动失败，请检查运行诊断。',
  'app22.error.OFFLINE_NODE': '设备离线，运行就绪状态未知。',
  'app22.error.RUNTIME_TIMEOUT': '超时前未收到有效启动证据。',
  'app22.error.RUNTIME_UNKNOWN': '此部署尚无可用运行报告。',
  'app22.error.STALE_EVIDENCE':
    '运行报告属于其他部署，不能证明此次部署已就绪。',
  'app22.error.OPERATION_FAILED': '运行操作失败，再次操作前需刷新运行报告。',
  'app22.error.OPERATION_TIMEOUT': '操作未在超时前收敛，当前运行状态未知。',
  'app22.error.TASK_NOT_FOUND': '未找到安装任务，未创建替代任务。',
  'app22.error.TASK_FORBIDDEN': '你没有读取此安装任务的权限。',
  'app22.error.DRAFT_NOT_FOUND': '此准备草稿不可用，请重新选择来源。',
  'app22.error.duplicate_parameter': '启动参数重复。',
  'app22.error.unknown_parameter': '此入口不接受该参数。',
  'app22.error.conflicting_parameters':
    'identifier 和 task_id 必须且只能提供一个。',
  'app22.error.invalid_task_id': '任务编号必须为非空、不含空白的不透明字符串。',
  'app22.error.invalid_identifier':
    '公开入口需要应用标识，不接受文件内容及内部引用。',
  'app22.error.invalid_options': '初始选项无效或过大。',
  'app22.error.invalid_target': '目标不存在，或节点 ID 与 DID 不匹配。',
  'app22.description.nextcloud': '为 Zone 提供私有文件同步、日历及协作。',
  'app22.description.paperless': '整理和搜索私有文档库。',
  'app22.description.home-dashboard': '托管在 Personal Server 上的静态仪表盘。',
  'app22.description.backup-script': '为应用数据执行本地备份进程。',
  'app22.description.opendan': '通过独立 binding 运行 Agent 的环境。',
  'app22.description.nostr-relay': '为此 Zone 提供去中心化社交中继。',
  'app22.description.home-assistant': '为家庭设备和日常流程提供本地自动化。',
  'app22.description.gateway': 'Zone 路由与 HTTPS 终止。',
  'app22.description.scheduler': '推导此 Zone 的部署配置。',
  'app22.description.verify-hub': '统一登录与会话凭据签发。',
  'app22.description.node-daemon': '使节点收敛到分配的配置。',
  'app22.description.kmsg': '内核消息队列。',
}
