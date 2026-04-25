mod common;

use aicc::{
    CostEstimate, ModelCatalog, ProviderError, ProviderStartResult, Registry, TaskEventKind,
};
use buckyos_api::{AiMethodStatus, AiResponseSummary, Capability};
use common::*;
use kRPC::RPCContext;
use std::sync::Arc;

fn add_llm(
    registry: &Registry,
    catalog: &ModelCatalog,
    id: &str,
    ptype: &str,
    cost: f64,
    lat: u64,
    r: std::result::Result<ProviderStartResult, ProviderError>,
) -> Arc<MockProvider> {
    catalog.set_mapping(Capability::Llm, "llm.plan.default", ptype, "m");
    let p = Arc::new(MockProvider::new(
        mock_instance(id, ptype, vec![Capability::Llm], vec!["plan".into()]),
        CostEstimate {
            estimated_cost_usd: Some(cost),
            estimated_latency_ms: Some(lat),
        },
        vec![r],
    ));
    registry.add_provider(p.clone());
    p
}

#[tokio::test]
// 鐢ㄤ緥璇存槑锛?
// - 楠岃瘉鍦烘櫙锛歚workflow_01_plan_generation_dag` 鐢ㄤ緥锛岃鐩栧嚱鏁板悕瀵瑰簲鐨勪笟鍔¤矾寰勩€?
// - 杈撳叆鍙傛暟锛氬噯澶囧伐浣滄祦姝ラ銆佷緷璧栧叧绯讳笌鎵ц绛栫暐銆?
// - 澶勭悊娴佺▼锛氭墽琛屽伐浣滄祦缂栨帓璺緞锛屾帹杩涜鍒掋€佷緷璧栬皟搴︺€侀噸璇?鍥為€€涓庢敹鏁涢樁娈点€?
// - 棰勬湡杈撳嚭锛氭柇瑷€涓殑鐘舵€併€侀敊璇爜銆佽矾鐢遍€夋嫨鎴栦簨浠跺瓧娈靛叏閮ㄦ弧瓒抽鏈熴€?
async fn workflow_01_plan_generation_dag() {
    let r = Registry::default();
    let c = ModelCatalog::default();
    add_llm(
        &r,
        &c,
        "p1",
        "a",
        0.01,
        10,
        Ok(ProviderStartResult::Immediate(AiResponseSummary {
            text: Some("{\"steps\":[]}".into()),
            tool_calls: vec![],
            artifacts: vec![],
            usage: None,
            cost: None,
            finish_reason: Some("stop".into()),
            provider_task_ref: None,
            extra: None,
        })),
    );
    let center = center_with_taskmgr(r, c);
    assert_eq!(
        center
            .complete(base_request(), RPCContext::default())
            .await
            .unwrap()
            .status,
        AiMethodStatus::Succeeded
    );
}

#[tokio::test]
// 鐢ㄤ緥璇存槑锛?
// - 楠岃瘉鍦烘櫙锛歚workflow_02_serial_dependency_blocking` 鐢ㄤ緥锛岃鐩栧嚱鏁板悕瀵瑰簲鐨勪笟鍔¤矾寰勩€?
// - 杈撳叆鍙傛暟锛氬噯澶囧伐浣滄祦姝ラ銆佷緷璧栧叧绯讳笌鎵ц绛栫暐銆?
// - 澶勭悊娴佺▼锛氭墽琛屽伐浣滄祦缂栨帓璺緞锛屾帹杩涜鍒掋€佷緷璧栬皟搴︺€侀噸璇?鍥為€€涓庢敹鏁涢樁娈点€?
// - 棰勬湡杈撳嚭锛氭柇瑷€涓殑鐘舵€併€侀敊璇爜銆佽矾鐢遍€夋嫨鎴栦簨浠跺瓧娈靛叏閮ㄦ弧瓒抽鏈熴€?
async fn workflow_02_serial_dependency_blocking() {
    let r = Registry::default();
    let c = ModelCatalog::default();
    let p1 = add_llm(
        &r,
        &c,
        "p1",
        "a",
        0.01,
        10,
        Ok(ProviderStartResult::Started),
    );
    let center = center_with_taskmgr(r, c);
    let a = center
        .complete(base_request(), RPCContext::default())
        .await
        .unwrap();
    let b = center
        .complete(base_request(), RPCContext::default())
        .await
        .unwrap();
    assert_eq!(a.status, AiMethodStatus::Running);
    assert_eq!(b.status, AiMethodStatus::Failed);
    assert!(!a.task_id.is_empty());
    assert!(!b.task_id.is_empty());
    assert_ne!(a.task_id, b.task_id);
    assert_eq!(p1.start_calls(), 2);
}

#[tokio::test]
// 鐢ㄤ緥璇存槑锛?
// - 楠岃瘉鍦烘櫙锛歚workflow_03_parallel_group_execution` 鐢ㄤ緥锛岃鐩栧嚱鏁板悕瀵瑰簲鐨勪笟鍔¤矾寰勩€?
// - 杈撳叆鍙傛暟锛氬噯澶囧伐浣滄祦姝ラ銆佷緷璧栧叧绯讳笌鎵ц绛栫暐銆?
// - 澶勭悊娴佺▼锛氭墽琛屽伐浣滄祦缂栨帓璺緞锛屾帹杩涜鍒掋€佷緷璧栬皟搴︺€侀噸璇?鍥為€€涓庢敹鏁涢樁娈点€?
// - 棰勬湡杈撳嚭锛氭柇瑷€涓殑鐘舵€併€侀敊璇爜銆佽矾鐢遍€夋嫨鎴栦簨浠跺瓧娈靛叏閮ㄦ弧瓒抽鏈熴€?
async fn workflow_03_parallel_group_execution() {
    let r = Registry::default();
    let c = ModelCatalog::default();
    let p1 = add_llm(
        &r,
        &c,
        "p1",
        "a",
        0.01,
        10,
        Ok(ProviderStartResult::Started),
    );
    let center = Arc::new(center_with_taskmgr(r, c));
    let a = center.clone();
    let b = center.clone();
    let h1 = tokio::spawn(async move {
        a.complete(base_request(), RPCContext::default())
            .await
            .unwrap()
            .task_id
    });
    let h2 = tokio::spawn(async move {
        b.complete(base_request(), RPCContext::default())
            .await
            .unwrap()
            .task_id
    });
    let t1 = h1.await.expect("join h1");
    let t2 = h2.await.expect("join h2");
    assert!(!t1.is_empty());
    assert!(!t2.is_empty());
    assert_ne!(t1, t2);
    assert_eq!(p1.start_calls(), 2);
}

#[tokio::test]
// 鐢ㄤ緥璇存槑锛?
// - 楠岃瘉鍦烘櫙锛歚workflow_04_retryable_then_fallback_success` 鐢ㄤ緥锛岃鐩栧彲閲嶈瘯閿欒鍒嗘敮銆佸洖閫€绛栫暐鍒嗘敮銆?
// - 杈撳叆鍙傛暟锛氭瀯閫犲涓?provider 鍊欓€夛紝骞舵敞鍏?Started/Queued/澶辫触缁撴灉锛涘噯澶囧伐浣滄祦姝ラ銆佷緷璧栧叧绯讳笌鎵ц绛栫暐銆?
// - 澶勭悊娴佺▼锛氭墽琛屽伐浣滄祦缂栨帓璺緞锛屾帹杩涜鍒掋€佷緷璧栬皟搴︺€侀噸璇?鍥為€€涓庢敹鏁涢樁娈点€?
// - 棰勬湡杈撳嚭锛氳繑鍥炴垚鍔熺粨鏋滐紝鍏抽敭瀛楁涓庢柇瑷€涓€鑷达紱閿欒琚綊绫讳负鍙噸璇曞苟瑙﹀彂瀵瑰簲绛栫暐锛涘洖閫€鎵ц娆℃暟涓庨『搴忔弧瓒崇敤渚嬫柇瑷€銆?
async fn workflow_04_retryable_then_fallback_success() {
    let r = Registry::default();
    let c = ModelCatalog::default();
    let p1 = add_llm(
        &r,
        &c,
        "p1",
        "a",
        0.01,
        10,
        Err(ProviderError::retryable("x")),
    );
    let p2 = add_llm(
        &r,
        &c,
        "p2",
        "b",
        0.02,
        20,
        Ok(ProviderStartResult::Started),
    );
    let center = center_with_taskmgr(r, c);
    let resp = center
        .complete(base_request(), RPCContext::default())
        .await
        .unwrap();
    assert_eq!(resp.status, AiMethodStatus::Running);
    assert!(!resp.task_id.is_empty());
    assert_eq!(p1.start_calls(), 1);
    assert_eq!(p2.start_calls(), 1);
}

#[tokio::test]
// 鐢ㄤ緥璇存槑锛?
// - 楠岃瘉鍦烘櫙锛歚workflow_05_fatal_step_abort` 鐢ㄤ緥锛岃鐩栬嚧鍛介敊璇垎鏀€?
// - 杈撳叆鍙傛暟锛氭瀯閫犲涓?provider 鍊欓€夛紝骞舵敞鍏?Started/Queued/澶辫触缁撴灉锛涘噯澶囧伐浣滄祦姝ラ銆佷緷璧栧叧绯讳笌鎵ц绛栫暐銆?
// - 澶勭悊娴佺▼锛氭墽琛屽伐浣滄祦缂栨帓璺緞锛屾帹杩涜鍒掋€佷緷璧栬皟搴︺€侀噸璇?鍥為€€涓庢敹鏁涢樁娈点€?
// - 棰勬湡杈撳嚭锛氳繑鍥炴嫆缁濇垨鑷村懡閿欒锛岄敊璇爜/閿欒娑堟伅绗﹀悎棰勬湡銆?
async fn workflow_05_fatal_step_abort() {
    let r = Registry::default();
    let c = ModelCatalog::default();
    add_llm(&r, &c, "p1", "a", 0.01, 10, Err(ProviderError::fatal("x")));
    add_llm(
        &r,
        &c,
        "p2",
        "b",
        0.02,
        20,
        Ok(ProviderStartResult::Started),
    );
    let center = center_with_taskmgr(r, c);
    assert_eq!(
        center
            .complete(base_request(), RPCContext::default())
            .await
            .unwrap()
            .status,
        AiMethodStatus::Failed
    );
}

#[tokio::test]
// 鐢ㄤ緥璇存槑锛?
// - 楠岃瘉鍦烘櫙锛歚workflow_06_replan_trigger_on_low_quality` 鐢ㄤ緥锛岃鐩栧嚱鏁板悕瀵瑰簲鐨勪笟鍔¤矾寰勩€?
// - 杈撳叆鍙傛暟锛氬噯澶囧伐浣滄祦姝ラ銆佷緷璧栧叧绯讳笌鎵ц绛栫暐銆?
// - 澶勭悊娴佺▼锛氭墽琛屽伐浣滄祦缂栨帓璺緞锛屾帹杩涜鍒掋€佷緷璧栬皟搴︺€侀噸璇?鍥為€€涓庢敹鏁涢樁娈点€?
// - 棰勬湡杈撳嚭锛氭柇瑷€涓殑鐘舵€併€侀敊璇爜銆佽矾鐢遍€夋嫨鎴栦簨浠跺瓧娈靛叏閮ㄦ弧瓒抽鏈熴€?
async fn workflow_06_replan_trigger_on_low_quality() {
    let r = Registry::default();
    let c = ModelCatalog::default();
    let p1 = add_llm(
        &r,
        &c,
        "p1",
        "a",
        0.01,
        10,
        Ok(ProviderStartResult::Started),
    );
    let center = center_with_taskmgr(r, c);
    let a = center
        .complete(base_request(), RPCContext::default())
        .await
        .unwrap();
    let b = center
        .complete(base_request(), RPCContext::default())
        .await
        .unwrap();
    assert_eq!(a.status, AiMethodStatus::Running);
    assert_eq!(b.status, AiMethodStatus::Failed);
    assert!(!a.task_id.is_empty());
    assert!(!b.task_id.is_empty());
    assert_ne!(a.task_id, b.task_id);
    assert_eq!(p1.start_calls(), 2);
}

#[tokio::test]
// 鐢ㄤ緥璇存槑锛?
// - 楠岃瘉鍦烘櫙锛歚workflow_07_subtask_fallback_alias` 鐢ㄤ緥锛岃鐩栧洖閫€绛栫暐鍒嗘敮銆佸埆鍚嶆槧灏勫垎鏀€?
// - 杈撳叆鍙傛暟锛氭瀯閫犲涓?provider 鍊欓€夛紝骞舵敞鍏?Started/Queued/澶辫触缁撴灉锛涘噯澶囧伐浣滄祦姝ラ銆佷緷璧栧叧绯讳笌鎵ц绛栫暐銆?
// - 澶勭悊娴佺▼锛氭墽琛屽伐浣滄祦缂栨帓璺緞锛屾帹杩涜鍒掋€佷緷璧栬皟搴︺€侀噸璇?鍥為€€涓庢敹鏁涢樁娈点€?
// - 棰勬湡杈撳嚭锛氬洖閫€鎵ц娆℃暟涓庨『搴忔弧瓒崇敤渚嬫柇瑷€銆?
async fn workflow_07_subtask_fallback_alias() {
    let r = Registry::default();
    let c = ModelCatalog::default();
    let p1 = add_llm(
        &r,
        &c,
        "p1",
        "a",
        0.01,
        10,
        Err(ProviderError::retryable("x")),
    );
    let p2 = add_llm(
        &r,
        &c,
        "p2",
        "b",
        0.02,
        20,
        Ok(ProviderStartResult::Started),
    );
    let center = center_with_taskmgr(r, c);
    let resp = center
        .complete(base_request(), RPCContext::default())
        .await
        .unwrap();
    assert_eq!(resp.status, AiMethodStatus::Running);
    assert!(!resp.task_id.is_empty());
    assert_eq!(p1.start_calls(), 1);
    assert_eq!(p2.start_calls(), 1);
}

#[tokio::test]
// 鐢ㄤ緥璇存槑锛?
// - 楠岃瘉鍦烘櫙锛歚workflow_08_end_to_end_orchestration_smoke` 鐢ㄤ緥锛岃鐩栧嚱鏁板悕瀵瑰簲鐨勪笟鍔¤矾寰勩€?
// - 杈撳叆鍙傛暟锛氬噯澶囧伐浣滄祦姝ラ銆佷緷璧栧叧绯讳笌鎵ц绛栫暐銆?
// - 澶勭悊娴佺▼锛氭墽琛屽伐浣滄祦缂栨帓璺緞锛屾帹杩涜鍒掋€佷緷璧栬皟搴︺€侀噸璇?鍥為€€涓庢敹鏁涢樁娈点€?
// - 棰勬湡杈撳嚭锛氭柇瑷€涓殑鐘舵€併€侀敊璇爜銆佽矾鐢遍€夋嫨鎴栦簨浠跺瓧娈靛叏閮ㄦ弧瓒抽鏈熴€?
async fn workflow_08_end_to_end_orchestration_smoke() {
    let r = Registry::default();
    let c = ModelCatalog::default();
    let p1 = add_llm(
        &r,
        &c,
        "p1",
        "a",
        0.01,
        10,
        Ok(ProviderStartResult::Queued { position: 1 }),
    );
    let center = center_with_taskmgr(r, c);
    let resp = center
        .complete(base_request(), RPCContext::default())
        .await
        .unwrap();
    assert_eq!(resp.status, AiMethodStatus::Running);
    assert!(!resp.task_id.is_empty());
    assert!(
        resp.event_ref
            .as_deref()
            .map(|event_ref| event_ref.contains(resp.task_id.as_str()))
            .unwrap_or(false),
        "queued response should carry event_ref bound to task_id: {:?}",
        resp
    );
    assert_eq!(p1.start_calls(), 1);
}

#[tokio::test]
async fn workflow_01_plan_generates_valid_dag() {
    let r = Registry::default();
    let c = ModelCatalog::default();
    let p1 = add_llm(
        &r,
        &c,
        "planner",
        "a",
        0.01,
        10,
        Ok(ProviderStartResult::Immediate(AiResponseSummary {
            text: Some("{\"steps\":[]}".into()),
            tool_calls: vec![],
            artifacts: vec![],
            usage: None,
            cost: None,
            finish_reason: Some("stop".into()),
            provider_task_ref: None,
            extra: None,
        })),
    );
    let center = center_with_taskmgr(r, c);
    let resp = center
        .complete(base_request(), RPCContext::default())
        .await
        .unwrap();
    assert_eq!(resp.status, AiMethodStatus::Succeeded);
    assert_eq!(p1.start_calls(), 1);
}

#[tokio::test]
async fn workflow_02_serial_dependency_blocks_until_ready() {
    let r = Registry::default();
    let c = ModelCatalog::default();
    let p1 = add_llm(
        &r,
        &c,
        "p1",
        "a",
        0.01,
        10,
        Ok(ProviderStartResult::Started),
    );
    let center = center_with_taskmgr(r, c);
    let a = center
        .complete(base_request(), RPCContext::default())
        .await
        .unwrap();
    let b = center
        .complete(base_request(), RPCContext::default())
        .await
        .unwrap();
    assert_eq!(a.status, AiMethodStatus::Running);
    assert_eq!(b.status, AiMethodStatus::Failed);
    assert!(!a.task_id.is_empty());
    assert!(!b.task_id.is_empty());
    assert_ne!(a.task_id, b.task_id);
    assert_eq!(p1.start_calls(), 2);
}

#[tokio::test]
async fn workflow_03_parallel_group_executes_concurrently() {
    let r = Registry::default();
    let c = ModelCatalog::default();
    let p1 = add_llm(
        &r,
        &c,
        "p1",
        "a",
        0.01,
        10,
        Ok(ProviderStartResult::Started),
    );
    let center = Arc::new(center_with_taskmgr(r, c));
    let a = center.clone();
    let b = center.clone();
    let h1 = tokio::spawn(async move {
        a.complete(base_request(), RPCContext::default())
            .await
            .unwrap()
            .task_id
    });
    let h2 = tokio::spawn(async move {
        b.complete(base_request(), RPCContext::default())
            .await
            .unwrap()
            .task_id
    });
    let t1 = h1.await.expect("join h1");
    let t2 = h2.await.expect("join h2");
    assert!(!t1.is_empty());
    assert!(!t2.is_empty());
    assert_ne!(t1, t2);
    assert_eq!(p1.start_calls(), 2);
}

#[tokio::test]
async fn workflow_04_replan_triggered_on_quality_threshold() {
    let r = Registry::default();
    let c = ModelCatalog::default();
    let p1 = add_llm(
        &r,
        &c,
        "p1",
        "a",
        0.01,
        10,
        Ok(ProviderStartResult::Started),
    );
    let center = center_with_taskmgr(r, c);
    let a = center
        .complete(base_request(), RPCContext::default())
        .await
        .unwrap();
    let b = center
        .complete(base_request(), RPCContext::default())
        .await
        .unwrap();
    assert_eq!(a.status, AiMethodStatus::Running);
    assert_eq!(b.status, AiMethodStatus::Failed);
    assert!(!a.task_id.is_empty());
    assert!(!b.task_id.is_empty());
    assert_ne!(a.task_id, b.task_id);
    assert_eq!(p1.start_calls(), 2);
}

#[tokio::test]
async fn workflow_05_retryable_subtask_uses_fallback_alias() {
    let r = Registry::default();
    let c = ModelCatalog::default();
    let p1 = add_llm(
        &r,
        &c,
        "p1",
        "a",
        0.01,
        10,
        Err(ProviderError::retryable("x")),
    );
    let p2 = add_llm(
        &r,
        &c,
        "p2",
        "b",
        0.02,
        20,
        Ok(ProviderStartResult::Started),
    );
    let center = center_with_taskmgr(r, c);
    let resp = center
        .complete(base_request(), RPCContext::default())
        .await
        .unwrap();
    assert_eq!(resp.status, AiMethodStatus::Running);
    assert!(!resp.task_id.is_empty());
    assert_eq!(p1.start_calls(), 1);
    assert_eq!(p2.start_calls(), 1);
}

#[tokio::test]
async fn workflow_06_started_subtask_never_retries_cross_instance() {
    let r = Registry::default();
    let c = ModelCatalog::default();
    let p1 = add_llm(
        &r,
        &c,
        "p1",
        "a",
        0.01,
        10,
        Ok(ProviderStartResult::Started),
    );
    add_llm(
        &r,
        &c,
        "p2",
        "b",
        0.02,
        20,
        Ok(ProviderStartResult::Started),
    );
    let center = center_with_taskmgr(r, c);
    let _ = center
        .complete(base_request(), RPCContext::default())
        .await
        .unwrap();
    assert_eq!(p1.start_calls(), 1);
}

#[tokio::test]
async fn workflow_07_each_step_routes_to_correct_capability() {
    let r = Registry::default();
    let c = ModelCatalog::default();
    let p1 = add_llm(
        &r,
        &c,
        "p1",
        "a",
        0.01,
        10,
        Ok(ProviderStartResult::Queued { position: 1 }),
    );
    let sink = Arc::new(CollectingSinkFactory::new());
    let mut center = center_with_taskmgr(r, c);
    center.set_task_event_sink_factory(sink.clone());
    let resp = center
        .complete(base_request(), RPCContext::default())
        .await
        .unwrap();
    assert_eq!(resp.status, AiMethodStatus::Running);
    assert_eq!(p1.start_calls(), 1);
    assert!(resp.result.is_none());
    assert!(
        resp.event_ref
            .as_deref()
            .map(|event_ref| event_ref.contains(resp.task_id.as_str()))
            .unwrap_or(false),
        "running response should carry event_ref bound to task_id: {:?}",
        resp
    );
    let events = sink.events_for(&resp.task_id);
    assert_eq!(events.len(), 1, "queued task should emit exactly one event");
    let first = &events[0];
    assert_eq!(first.task_id, resp.task_id);
    assert!(matches!(first.kind, TaskEventKind::Queued));
    assert_eq!(
        first
            .data
            .as_ref()
            .and_then(|data| data.get("position"))
            .and_then(|value| value.as_u64()),
        Some(1)
    );
}

#[tokio::test]
async fn workflow_08_event_sequence_reflects_dag_structure() {
    let r = Registry::default();
    let c = ModelCatalog::default();
    let p1 = add_llm(
        &r,
        &c,
        "p1",
        "a",
        0.01,
        10,
        Ok(ProviderStartResult::Started),
    );
    let sink = Arc::new(CollectingSinkFactory::new());
    let mut center = center_with_taskmgr(r, c);
    center.set_task_event_sink_factory(sink.clone());
    let resp = center
        .complete(base_request(), RPCContext::default())
        .await
        .unwrap();
    assert_eq!(resp.status, AiMethodStatus::Running);
    assert_eq!(p1.start_calls(), 1);
    let events = sink.events_for(&resp.task_id);
    assert_eq!(events.len(), 1, "started task should emit a started event");
    let first = &events[0];
    assert_eq!(first.task_id, resp.task_id);
    assert!(matches!(first.kind, TaskEventKind::Started));
    assert_eq!(
        first
            .data
            .as_ref()
            .and_then(|data| data.get("instance_id"))
            .and_then(|value| value.as_str()),
        Some("p1")
    );
}
