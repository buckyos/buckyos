//! Task ACL policy computation (doc §8).
//!
//! Policy v1 is additive allow-grants only — no deny, no ordering, no
//! condition expressions — so the result is order-independent: relations are
//! computed, the task's `policy_preset` is expanded, explicit grants are
//! collected along the parent chain (stopping at a `permission_boundary`),
//! and matching actions are unioned. DataScope is the max over matching
//! rows. A principal without `ReadMeta` sees `task_not_found`, never
//! `permission_denied`, to avoid existence probing.

use crate::task_store::TaskStore;
use buckyos_api::*;
use kRPC::Result;
use serde_json::Value;
use std::collections::HashSet;

/// System role implicitly held by zone-trusted callers (tokens signed by the
/// zone owner or a device key). Explicit `SystemRole` grants may reference
/// it; the trusted control-plane commands additionally require it directly.
pub const SYSTEM_ROLE_ZONE_TRUSTED: &str = "zone-trusted";

#[derive(Debug, Clone)]
pub struct Principal {
    pub user_id: String,
    pub app_id: String,
    pub roles: Vec<String>,
}

impl Principal {
    pub fn actor_ref(&self) -> ActorRef {
        ActorRef {
            user_id: self.user_id.clone(),
            app_id: self.app_id.clone(),
            app_instance_id: None,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct TaskPermission {
    pub actions: HashSet<TaskAction>,
    pub data_scope: Option<TaskDataScope>,
}

impl TaskPermission {
    pub fn allows(&self, action: TaskAction) -> bool {
        self.actions.contains(&action)
    }

    pub fn visible(&self) -> bool {
        self.allows(TaskAction::ReadMeta)
    }

    fn add(&mut self, actions: &[TaskAction], scope: TaskDataScope) {
        for action in actions {
            self.actions.insert(*action);
        }
        self.data_scope = Some(match self.data_scope {
            Some(existing) => existing.max(scope),
            None => scope,
        });
    }
}

/// Compute the caller's effective permission on `task` following the fixed
/// order of doc §8.4.
pub async fn compute_permission(
    store: &TaskStore,
    principal: &Principal,
    task: &Task,
) -> Result<TaskPermission> {
    let mut permission = TaskPermission::default();

    // Step 2/4 prerequisite: the parent chain with boundary flags, truncated
    // at the first permission boundary (the boundary node's own grants still
    // apply; its ancestors' do not).
    let chain = store.get_task_chain(&task.task_id).await?;
    let mut inheritable_ids: Vec<TaskId> = Vec::new();
    let mut root_reachable = false;
    for (task_id, parent_id, boundary) in chain.iter() {
        // A node's own grants always apply to it; its boundary flag blocks
        // its *ancestors* from this point up.
        inheritable_ids.push(task_id.clone());
        if parent_id.is_none() {
            root_reachable = true;
            break;
        }
        if *boundary {
            break;
        }
    }

    // Relations on the current task.
    let is_creator =
        task.creator.user_id == principal.user_id && task.creator.app_id == principal.app_id;
    let is_runner = matches!(
        &task.executor,
        TaskExecutor::App { app_id, .. } if *app_id == principal.app_id
    );
    let assignees = match task.assignees.clone() {
        Some(list) => list,
        None if task.executor.kind() == TaskExecutorKind::HumanSet => {
            store.active_assignees(&task.task_id).await?
        }
        None => Vec::new(),
    };
    let is_assignee = assignees.iter().any(|user| user == &principal.user_id);

    let is_root_creator = if root_reachable {
        if task.parent_id.is_none() {
            is_creator
        } else {
            match store.get_task(&task.root_id).await? {
                Some(root) => {
                    root.creator.user_id == principal.user_id
                        && root.creator.app_id == principal.app_id
                }
                None => false,
            }
        }
    } else {
        false
    };

    // Step 3: expand the preset. v1 ships one preset; unknown preset names
    // fall back to it rather than opening access.
    expand_collaborative_tree_preset(
        &mut permission,
        is_root_creator,
        is_creator,
        is_runner,
        is_assignee,
    );

    // Tree-wide MetaOnly row for participants of any task in the tree,
    // blocked for boundary-cut subtrees together with the rest of the
    // inherited rows.
    if root_reachable && !permission.visible() {
        if store
            .is_tree_participant(&task.root_id, &principal.user_id, &principal.app_id)
            .await?
        {
            permission.add(&[TaskAction::ReadMeta], TaskDataScope::MetaOnly);
        }
    }

    // Step 4/5: explicit grants along the (boundary-truncated) chain.
    let grants = store.active_grants_for(&inheritable_ids).await?;
    for grant in grants {
        let anchored_here = grant.task_id == task.task_id;
        let scope_covers = anchored_here
            || matches!(grant.scope, TaskGrantScope::Subtree | TaskGrantScope::WholeTree);
        if !scope_covers {
            continue;
        }
        let subject_matches = match &grant.subject {
            TaskGrantSubject::RootCreator => is_root_creator,
            TaskGrantSubject::Creator => is_creator,
            TaskGrantSubject::Runner => is_runner,
            TaskGrantSubject::Assignees => is_assignee,
            TaskGrantSubject::User { user_id } => *user_id == principal.user_id,
            TaskGrantSubject::App { app_id } => *app_id == principal.app_id,
            TaskGrantSubject::Principal { user_id, app_id } => {
                *user_id == principal.user_id && *app_id == principal.app_id
            }
            TaskGrantSubject::SystemRole { role } => principal.roles.iter().any(|r| r == role),
        };
        if subject_matches {
            permission.add(&grant.actions, grant.data_scope);
        }
    }

    Ok(permission)
}

fn expand_collaborative_tree_preset(
    permission: &mut TaskPermission,
    is_root_creator: bool,
    is_creator: bool,
    is_runner: bool,
    is_assignee: bool,
) {
    if is_root_creator {
        permission.add(
            &[
                TaskAction::ReadMeta,
                TaskAction::ReadInput,
                TaskAction::ReadResult,
                TaskAction::Control,
                TaskAction::CreateChild,
                TaskAction::Reassign,
                TaskAction::Grant,
                TaskAction::Archive,
            ],
            TaskDataScope::Full,
        );
    }
    if is_creator {
        permission.add(
            &[
                TaskAction::ReadMeta,
                TaskAction::ReadInput,
                TaskAction::ReadResult,
            ],
            TaskDataScope::Full,
        );
    }
    if is_runner {
        permission.add(
            &[
                TaskAction::ReadMeta,
                TaskAction::ReadInput,
                TaskAction::ReportProgress,
                TaskAction::Commit,
                TaskAction::CreateChild,
            ],
            TaskDataScope::Full,
        );
    }
    if is_assignee {
        permission.add(
            &[
                TaskAction::ReadMeta,
                TaskAction::ReadInput,
                TaskAction::Commit,
                TaskAction::Reassign,
            ],
            TaskDataScope::Full,
        );
    }
}

/// Server-side field trimming (doc §8.4 step 6): payload visibility needs
/// both the matching read action and a data scope above MetaOnly; the
/// idempotency/audit envelope is Full-only.
pub fn trim_task(mut task: Task, permission: &TaskPermission) -> Task {
    let scope = permission.data_scope.unwrap_or(TaskDataScope::MetaOnly);
    let payload_scope = scope != TaskDataScope::MetaOnly;
    if !(payload_scope && permission.allows(TaskAction::ReadInput)) {
        task.input = Value::Null;
    }
    if !(payload_scope && permission.allows(TaskAction::ReadResult)) {
        task.result = None;
        task.error = task.error.map(|e| TaskError {
            code: e.code,
            message: String::new(),
            detail: None,
        });
    }
    if scope != TaskDataScope::Full {
        task.idempotency_key = String::new();
        task.origin_ref = None;
    }
    if scope == TaskDataScope::MetaOnly {
        task.progress = None;
    }
    task.data_scope = Some(scope);
    task
}

/// Metadata projection used by list/tree endpoints.
pub fn summarize_task(task: &Task) -> TaskSummary {
    TaskSummary {
        task_id: task.task_id.clone(),
        name: task.name.clone(),
        parent_id: task.parent_id.clone(),
        root_id: task.root_id.clone(),
        schema_id: task.schema_id.clone(),
        schema_version: task.schema_version,
        creator: ActorRef {
            user_id: task.creator.user_id.clone(),
            app_id: task.creator.app_id.clone(),
            app_instance_id: None,
        },
        executor_kind: task.executor.kind(),
        phase: task.phase,
        wait_reason: task.wait_reason.clone(),
        pending_control_action: task.pending_control.as_ref().map(|c| c.action),
        outcome: task.outcome,
        message: task.message.clone(),
        revision: task.revision,
        created_at: task.created_at,
        updated_at: task.updated_at,
        completed_at: task.completed_at,
        archived_at: task.archived_at,
    }
}
