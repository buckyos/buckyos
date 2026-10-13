# AICC E2E Acceptance Report

- Run: `aicc-t1-2026-10-10T15-11-10-593Z-fa0fd08a`
- Commit: `c3e1327677f9697ecdc26e51714621308d8ec86f`
- Capability baseline: `T1-mock`
- Real model calls: 0/0
- Planned maximum cost: $0.000000
- Known actual / unknown estimated exposure: $0.000000 / $0.000000
- Total exposure / budget: $0.000000 / $0.000000
- Unknown-cost calls: 0; budget exceeded: false
- Results: passed=0, failed=2, provider_restricted=0, skipped=0, not_applicable=0, review=0
- Manifest coverage: 0/130 (0.00%); passed=0, failed=0
- T1 requirement branches: 0/127 (0.00%); passed=0, failed=0, skipped=0
- T1 route exposure: 0/2 passed; failed=2, skipped=0

## Targeted retest

Run after fixing the reported defect; repeat `--case` to select additional cases:

```bash
pnpm run acceptance:t1 -- --config "aicc_acceptance.local.toml" --allow-config-mutation --case t1.route.exposure.minimax.image--.image.img2img --case t1.route.exposure.minimax.image--.image.txt2img
```

## Cases

| Case | Layer | Provider | Model | API | Status |
|---|---|---|---|---|---|
| t1.route.exposure.minimax.image--.image.img2img | T1 | minimax/dv-minimax-aicc-t1-2026-10-10T15-11-10-593Z-fa0fd08a | - | image.img2img | failed |
| t1.route.exposure.minimax.image--.image.txt2img | T1 | minimax/dv-minimax-aicc-t1-2026-10-10T15-11-10-593Z-fa0fd08a | - | image.txt2img | failed |

## Unexecuted manifest cases

- t1.route.exact_model_hits_instance
- t1.route.logical_model_selects_candidate
- t1.route.metadata_variant_expands_exact_model
- t1.route.version_exact_rule
- t1.route.version_pattern_rule
- t1.route.version_default_rule
- t1.route.legal_missing_model
- t1.route.invalid_exact_model
- t1.route.invalid_logical_path
- t1.route.missing_provider_instance
- t1.route.disabled_model
- t1.route.offline_model
- t1.route.unmounted_model
- t1.route.corrupt_metadata
- t1.route.strict_no_fallback
- t1.route.parent_fallback
- t1.route.target_logical_fallback
- t1.route.target_exact_fallback
- t1.route.exact_default_no_fallback
- t1.route.fallback_api_type_boundary
- t1.route.fallback_loop
- t1.route.fallback_max_depth
- t1.route.provider_allow
- t1.route.provider_deny
- t1.route.local_only
- t1.route.privacy_boundary
- t1.route.health_filter
- t1.route.quota_filter
- t1.route.budget_filter
- t1.route.context_limit_filter
- t1.route.output_limit_filter
- t1.route.locked_policy_cannot_override
- t1.route.missing_metadata_is_conservative
- t1.route.min_line_admission
- t1.route.disable_line_applied
- t1.route.auto_mount_admission
- t1.route.manual_mount_requires_mapping
- t1.route.global_exact_model_weight
- t1.route.logical_exact_model_weight
- t1.route.provider_instance_weight
- t1.route.system_config_then_request_overlay
- t1.custom.openai
- t1.custom.claude
- t1.custom.gemini
- t1.scheduler.profile.cost_first
- t1.scheduler.profile.latency_first
- t1.scheduler.profile.quality_first
- t1.scheduler.profile.balanced
- t1.scheduler.profile.local_first
- t1.scheduler.profile.strict_local
- t1.route.api_type.llm.chat.completions.create
- t1.route.api_type.embedding.text.embedding.text
- t1.route.api_type.embedding.multimodal.embedding.multimodal
- t1.route.api_type.decision.decision.evaluate
- t1.route.api_type.rerank.rerank
- t1.route.api_type.image.txt2img.images.generate
- t1.route.api_type.image.img2img.image.img2img
- t1.route.api_type.image.inpaint.image.inpaint
- t1.route.api_type.image.upscale.image.upscale
- t1.route.api_type.image.bg_remove.image.bg_remove
- t1.route.api_type.vision.ocr.vision.ocr
- t1.route.api_type.vision.caption.vision.caption
- t1.route.api_type.vision.detect.vision.detect
- t1.route.api_type.vision.segment.vision.segment
- t1.route.api_type.audio.tts.audio.tts
- t1.route.api_type.audio.asr.audio.asr
- t1.route.api_type.audio.music.audio.music
- t1.route.api_type.audio.enhance.audio.enhance
- t1.route.api_type.video.txt2video.video.txt2video
- t1.route.api_type.video.img2video.video.img2video
- t1.route.api_type.video.video2video.video.video2video
- t1.route.api_type.video.extend.video.extend
- t1.route.api_type.video.upscale.video.upscale
- t1.route.api_type.agent.computer_use.agent.computer_use
- t1.runtime_boundary.rate_limit_fallback
- t1.runtime_boundary.server_error_fallback
- t1.runtime_boundary.connection_failure_fallback
- t1.runtime_boundary.timeout_fallback
- t1.runtime_boundary.malformed_response_rejected
- t1.runtime_boundary.wrong_mime_rejected
- t1.runtime_boundary.missing_usage_rejected
- t1.history.same_session_reuses_exact_model
- t1.history.hard_constraint_overrides.api_type_changed
- t1.history.hard_constraint_overrides.required_capability_changed
- t1.history.hard_constraint_overrides.disabled_capability_changed
- t1.history.hard_constraint_overrides.provider_denied
- t1.history.hard_constraint_overrides.instance_unhealthy
- t1.history.hard_constraint_overrides.quota_exhausted
- t1.history.hard_constraint_overrides.budget_exhausted
- t1.history.hard_constraint_overrides.local_only_changed
- t1.history.hard_constraint_overrides.context_limit_exceeded
- t1.history.hard_constraint_overrides.output_limit_exceeded
- t1.history.hard_constraint_overrides.locked_policy_changed
- t1.history.sessions_do_not_leak
- t1.task.immediate_succeeded
- t1.task.running_succeeded
- t1.task.running_failed
- t1.task.cancelled
- t1.task.unknown
- t1.task.idempotency_conflict_different_body
- t1.task.concurrent_idempotency
- t1.task.concurrent_completion
- t1.task.terminal_idempotent
- t1.task.reload_recovery
- t1.task.restart_recovery
- t1.usage.success_once
- t1.usage.idempotent_no_double_charge
- t1.usage.fallback_attempts_attributed
- t1.security.no_token
- t1.security.invalid_token
- t1.security.expired_token
- t1.security.cross_tenant
- t1.security.cross_tenant_task_cancel
- t1.security.cross_tenant_usage
- t1.security.cross_tenant_message
- t1.security.cross_tenant_object
- t1.security.rbac_admin_method
- t1.config.reload_valid
- t1.config.reload_invalid_keeps_old
- t1.config.provider_instance_isolation
- t1.config.provider_add_refresh
- t1.config.provider_validate_rejects_duplicate
- t1.config.provider_delete_isolation
- t1.config.provider_update_rollback
- t1.config.cloud_update_dynamic_catalog
- t1.config.restart_consistency
- t1.observability.correlation
- t1.observability.redaction
- t1.embedding.large_batch_artifact
- t1.embedding.space_mismatch_rejected

## T1 requirement branch coverage

| Branch | Planned cases | Executed cases | Status |
|---|---:|---:|---|
| route.exact_model_hits_instance | 1 | 0 | unexecuted |
| route.logical_model_selects_candidate | 1 | 0 | unexecuted |
| route.metadata_variant_expands_exact_model | 1 | 0 | unexecuted |
| route.version_exact_rule | 1 | 0 | unexecuted |
| route.version_pattern_rule | 1 | 0 | unexecuted |
| route.version_default_rule | 1 | 0 | unexecuted |
| route.legal_missing_model | 1 | 0 | unexecuted |
| route.invalid_exact_model | 1 | 0 | unexecuted |
| route.invalid_logical_path | 1 | 0 | unexecuted |
| route.missing_provider_instance | 1 | 0 | unexecuted |
| route.disabled_model | 1 | 0 | unexecuted |
| route.offline_model | 1 | 0 | unexecuted |
| route.unmounted_model | 1 | 0 | unexecuted |
| route.corrupt_metadata | 1 | 0 | unexecuted |
| route.strict_no_fallback | 1 | 0 | unexecuted |
| route.parent_fallback | 1 | 0 | unexecuted |
| route.target_logical_fallback | 1 | 0 | unexecuted |
| route.target_exact_fallback | 1 | 0 | unexecuted |
| route.exact_default_no_fallback | 1 | 0 | unexecuted |
| route.fallback_api_type_boundary | 1 | 0 | unexecuted |
| route.fallback_loop | 1 | 0 | unexecuted |
| route.fallback_max_depth | 1 | 0 | unexecuted |
| route.provider_allow | 1 | 0 | unexecuted |
| route.provider_deny | 1 | 0 | unexecuted |
| route.local_only | 1 | 0 | unexecuted |
| route.privacy_boundary | 1 | 0 | unexecuted |
| route.health_filter | 1 | 0 | unexecuted |
| route.quota_filter | 1 | 0 | unexecuted |
| route.budget_filter | 1 | 0 | unexecuted |
| route.context_limit_filter | 1 | 0 | unexecuted |
| route.output_limit_filter | 1 | 0 | unexecuted |
| route.locked_policy_cannot_override | 1 | 0 | unexecuted |
| route.missing_metadata_is_conservative | 1 | 0 | unexecuted |
| route.min_line_admission | 1 | 0 | unexecuted |
| route.disable_line_applied | 1 | 0 | unexecuted |
| route.auto_mount_admission | 1 | 0 | unexecuted |
| route.manual_mount_requires_mapping | 1 | 0 | unexecuted |
| route.global_exact_model_weight | 1 | 0 | unexecuted |
| route.logical_exact_model_weight | 1 | 0 | unexecuted |
| route.provider_instance_weight | 1 | 0 | unexecuted |
| route.system_config_then_request_overlay | 1 | 0 | unexecuted |
| route.api_type.llm.chat.completions.create | 1 | 0 | unexecuted |
| route.api_type.embedding.text.embedding.text | 1 | 0 | unexecuted |
| route.api_type.embedding.multimodal.embedding.multimodal | 1 | 0 | unexecuted |
| route.api_type.decision.decision.evaluate | 1 | 0 | unexecuted |
| route.api_type.rerank.rerank | 1 | 0 | unexecuted |
| route.api_type.image.txt2img.images.generate | 1 | 0 | unexecuted |
| route.api_type.image.img2img.image.img2img | 1 | 0 | unexecuted |
| route.api_type.image.inpaint.image.inpaint | 1 | 0 | unexecuted |
| route.api_type.image.upscale.image.upscale | 1 | 0 | unexecuted |
| route.api_type.image.bg_remove.image.bg_remove | 1 | 0 | unexecuted |
| route.api_type.vision.ocr.vision.ocr | 1 | 0 | unexecuted |
| route.api_type.vision.caption.vision.caption | 1 | 0 | unexecuted |
| route.api_type.vision.detect.vision.detect | 1 | 0 | unexecuted |
| route.api_type.vision.segment.vision.segment | 1 | 0 | unexecuted |
| route.api_type.audio.tts.audio.tts | 1 | 0 | unexecuted |
| route.api_type.audio.asr.audio.asr | 1 | 0 | unexecuted |
| route.api_type.audio.music.audio.music | 1 | 0 | unexecuted |
| route.api_type.audio.enhance.audio.enhance | 1 | 0 | unexecuted |
| route.api_type.video.txt2video.video.txt2video | 1 | 0 | unexecuted |
| route.api_type.video.img2video.video.img2video | 1 | 0 | unexecuted |
| route.api_type.video.video2video.video.video2video | 1 | 0 | unexecuted |
| route.api_type.video.extend.video.extend | 1 | 0 | unexecuted |
| route.api_type.video.upscale.video.upscale | 1 | 0 | unexecuted |
| route.api_type.agent.computer_use.agent.computer_use | 1 | 0 | unexecuted |
| scheduler.profile.cost_first | 1 | 0 | unexecuted |
| scheduler.profile.latency_first | 1 | 0 | unexecuted |
| scheduler.profile.quality_first | 1 | 0 | unexecuted |
| scheduler.profile.balanced | 1 | 0 | unexecuted |
| scheduler.profile.local_first | 1 | 0 | unexecuted |
| scheduler.profile.strict_local | 1 | 0 | unexecuted |
| history.same_session_reuses_exact_model | 1 | 0 | unexecuted |
| history.hard_constraint_overrides.api_type_changed | 1 | 0 | unexecuted |
| history.hard_constraint_overrides.required_capability_changed | 1 | 0 | unexecuted |
| history.hard_constraint_overrides.disabled_capability_changed | 1 | 0 | unexecuted |
| history.hard_constraint_overrides.provider_denied | 1 | 0 | unexecuted |
| history.hard_constraint_overrides.instance_unhealthy | 1 | 0 | unexecuted |
| history.hard_constraint_overrides.quota_exhausted | 1 | 0 | unexecuted |
| history.hard_constraint_overrides.budget_exhausted | 1 | 0 | unexecuted |
| history.hard_constraint_overrides.local_only_changed | 1 | 0 | unexecuted |
| history.hard_constraint_overrides.context_limit_exceeded | 1 | 0 | unexecuted |
| history.hard_constraint_overrides.output_limit_exceeded | 1 | 0 | unexecuted |
| history.hard_constraint_overrides.locked_policy_changed | 1 | 0 | unexecuted |
| history.sessions_do_not_leak | 1 | 0 | unexecuted |
| runtime_boundary.rate_limit_fallback | 1 | 0 | unexecuted |
| runtime_boundary.server_error_fallback | 1 | 0 | unexecuted |
| runtime_boundary.connection_failure_fallback | 1 | 0 | unexecuted |
| runtime_boundary.timeout_fallback | 1 | 0 | unexecuted |
| runtime_boundary.malformed_response_rejected | 1 | 0 | unexecuted |
| runtime_boundary.wrong_mime_rejected | 1 | 0 | unexecuted |
| runtime_boundary.missing_usage_rejected | 1 | 0 | unexecuted |
| task.immediate_succeeded | 1 | 0 | unexecuted |
| task.running_succeeded | 1 | 0 | unexecuted |
| task.running_failed | 1 | 0 | unexecuted |
| task.cancelled | 1 | 0 | unexecuted |
| task.unknown | 1 | 0 | unexecuted |
| task.idempotency_conflict_different_body | 1 | 0 | unexecuted |
| task.concurrent_idempotency | 1 | 0 | unexecuted |
| task.concurrent_completion | 1 | 0 | unexecuted |
| task.terminal_idempotent | 1 | 0 | unexecuted |
| task.reload_recovery | 1 | 0 | unexecuted |
| task.restart_recovery | 1 | 0 | unexecuted |
| usage.success_once | 1 | 0 | unexecuted |
| usage.idempotent_no_double_charge | 1 | 0 | unexecuted |
| usage.fallback_attempts_attributed | 1 | 0 | unexecuted |
| security.no_token | 1 | 0 | unexecuted |
| security.invalid_token | 1 | 0 | unexecuted |
| security.expired_token | 1 | 0 | unexecuted |
| security.cross_tenant | 1 | 0 | unexecuted |
| security.cross_tenant_task_cancel | 1 | 0 | unexecuted |
| security.cross_tenant_usage | 1 | 0 | unexecuted |
| security.cross_tenant_message | 1 | 0 | unexecuted |
| security.cross_tenant_object | 1 | 0 | unexecuted |
| security.rbac_admin_method | 1 | 0 | unexecuted |
| config.reload_valid | 1 | 0 | unexecuted |
| config.reload_invalid_keeps_old | 1 | 0 | unexecuted |
| config.provider_instance_isolation | 1 | 0 | unexecuted |
| config.provider_add_refresh | 1 | 0 | unexecuted |
| config.provider_validate_rejects_duplicate | 1 | 0 | unexecuted |
| config.provider_delete_isolation | 1 | 0 | unexecuted |
| config.provider_update_rollback | 1 | 0 | unexecuted |
| config.cloud_update_dynamic_catalog | 1 | 0 | unexecuted |
| config.restart_consistency | 1 | 0 | unexecuted |
| observability.correlation | 1 | 0 | unexecuted |
| observability.redaction | 1 | 0 | unexecuted |
| embedding.large_batch_artifact | 1 | 0 | unexecuted |
| embedding.space_mismatch_rejected | 1 | 0 | unexecuted |

## T1 combination coverage

| Combination group | Planned cells | Executed cells | Passed cells | Coverage |
|---|---:|---:|---:|---:|
| route_constraints | 65 | 0 | 0 | 0.00% |
| scheduler_profiles | 6 | 0 | 0 | 0.00% |
| history_constraints | 13 | 0 | 0 | 0.00% |
| canonical_api_routes | 24 | 0 | 0 | 0.00% |
| runtime_boundaries | 7 | 0 | 0 | 0.00% |
| cross_cutting | 34 | 0 | 0 | 0.00% |
| embedding_boundaries | 2 | 0 | 0 | 0.00% |

## Confirmed product defects

None.

## Cleanup

Status: passed

- services/aicc/settings restored byte-for-byte
- mock Provider uses zero real model calls
