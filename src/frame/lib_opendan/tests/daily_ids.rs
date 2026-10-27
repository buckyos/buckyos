mod common;

use std::sync::Arc;

use async_trait::async_trait;
use common::*;
use libopendan::api::{create_sub_session, sub_session_id, SubSessionSpec};
use libopendan::channel::KmsgChannels;
use libopendan::protocol::*;
use libopendan::state::{
    ForwardingStateClient, GoalCaller, GoalCreate, KrpcAgentStateClient, StateTransport,
};
use libopendan::{create_session, AgentStateClient, FsAgentStateClient, SessionDir};
use serde_json::{json, Value};

fn daily_id(sd: &SessionDir, kind: SessionKind) -> u64 {
    let (prefix, rest) = sd.sid().split_once('-').unwrap();
    let (date, number) = rest.split_once('-').unwrap();
    assert_eq!(prefix, kind.id_prefix());
    assert_eq!(date.len(), 8);
    chrono::NaiveDate::parse_from_str(date, "%Y%m%d").unwrap();
    let number: u64 = number.parse().unwrap();
    assert!(number > 0);
    assert_eq!(sd.path().file_name().unwrap(), sd.sid());
    assert_eq!(sd.config().unwrap().session.session_id, sd.sid());
    assert_eq!(SessionDir::open(sd.path()).unwrap().sid(), sd.sid());
    number
}

#[tokio::test]
async fn automatic_session_kinds_keep_short_ids_as_directory_names() {
    let env = Env::new();
    let agent = env.agent();
    let channels = env.channels();
    let mut numbers = Vec::new();
    for kind in [SessionKind::Work, SessionKind::Ui, SessionKind::SelfCheck] {
        let mut spec = work_spec("short directory");
        spec.kind = kind;
        spec.class = kind.as_str().into();
        spec.route_key = (kind == SessionKind::Ui).then(|| "conversation".into());
        let sd = create_session(&env.app_dir, spec, agent.as_ref(), APP, channels.as_ref())
            .await
            .unwrap();
        numbers.push(daily_id(&sd, kind));
    }
    let goal = agent
        .goals()
        .create(
            &GoalCreate {
                objective: "short goal session".into(),
                distance_evaluation: "remaining work".into(),
                ..Default::default()
            },
            &GoalCaller::owner(APP),
        )
        .await
        .unwrap();
    let (sd, created) = libopendan::goal::ensure_goal_session(
        agent.as_ref(),
        &goal,
        goal.session.epoch,
        APP,
        channels.as_ref(),
        &env.app_dir,
        &json!({}),
        "goal",
    )
    .await
    .unwrap();
    assert!(created);
    numbers.push(daily_id(&sd, SessionKind::Goal));
    assert_eq!(numbers, vec![1, 2, 3, 4]);
}

#[tokio::test]
async fn reserved_identity_survives_restart_and_creation_retry() {
    let env = Env::new();
    let mut spec = work_spec("retry");
    spec.idempotency_key = Some("request-1".into());
    let key = libopendan::ids::session_identity_key(
        spec.kind,
        AGENT,
        APP,
        spec.idempotency_key.as_deref(),
        None,
    )
    .unwrap();
    let expected = {
        let agent = env.agent();
        let sid = agent
            .sessions()
            .reserve_id(spec.kind, &key, Some(&env.app_dir))
            .await
            .unwrap();
        assert!(agent.sessions().lookup(&sid).await.unwrap().is_none());
        sid
    };
    let first = env.create_work(spec.clone()).await;
    let retry = env.create_work(spec).await;
    assert_eq!(first.sid(), expected);
    assert_eq!(retry.path(), first.path());
    assert_eq!(read_worklog(&retry).len(), 1);
    assert_eq!(std::fs::read_dir(&env.app_dir).unwrap().count(), 1);
}

#[tokio::test]
async fn reserved_sub_session_id_is_the_created_and_retried_id() {
    let env = Env::new();
    let parent = env.create_work(work_spec("parent")).await;
    let sub = SubSessionSpec {
        objective: "child".into(),
        key: Some("child-request".into()),
        ..Default::default()
    };
    let reserved = sub_session_id(env.agent().as_ref(), APP, parent.sid(), &sub)
        .await
        .unwrap();
    let first = create_sub_session(
        env.agent().as_ref(),
        APP,
        parent.sid(),
        sub.clone(),
        env.channels().as_ref(),
    )
    .await
    .unwrap();
    let retry = create_sub_session(
        env.agent().as_ref(),
        APP,
        parent.sid(),
        sub,
        env.channels().as_ref(),
    )
    .await
    .unwrap();
    assert_eq!(first.sid(), reserved);
    assert_eq!(first.path(), retry.path());
    assert_eq!(daily_id(&first, SessionKind::Work), 2);
}

#[tokio::test]
async fn custom_parent_uses_largest_daily_number_across_kinds() {
    let env = Env::new();
    let date = chrono::Local::now().format("%Y%m%d");
    for name in [
        format!("work-{date}-4"),
        format!("ui-{date}-22"),
        "work-19990101-99".into(),
    ] {
        std::fs::create_dir(env.app_dir.join(name)).unwrap();
    }
    let sd = env
        .create_work(work_spec("after existing directories"))
        .await;
    assert_eq!(daily_id(&sd, SessionKind::Work), 23);
    assert_eq!(sd.sid(), format!("work-{date}-23"));
    assert!(env.app_dir.join(format!("ui-{date}-22")).is_dir());
}

#[tokio::test]
async fn agents_with_the_same_driver_and_daily_id_have_isolated_queues() {
    let tmp = tempfile::tempdir().unwrap();
    let channels = KmsgChannels::dir(tmp.path().join("queues")).unwrap();
    let first_root = tmp.path().join("first");
    let second_root = tmp.path().join("second");
    let first_agent = FsAgentStateClient::open(
        &first_root,
        "did:bns:shared.alice",
        Some(channels.client()),
        None,
    )
    .unwrap();
    let second_agent = FsAgentStateClient::open(
        &second_root,
        "did:web:shared.alice",
        Some(channels.client()),
        None,
    )
    .unwrap();
    assert_eq!(first_agent.agent_id(), second_agent.agent_id());
    let first = create_session(
        &first_root.join("sessions"),
        work_spec("one"),
        &first_agent,
        APP,
        &channels,
    )
    .await
    .unwrap();
    let second = create_session(
        &second_root.join("sessions"),
        work_spec("two"),
        &second_agent,
        APP,
        &channels,
    )
    .await
    .unwrap();
    assert_eq!(first.sid(), second.sid());
    let first_cfg = first.config().unwrap();
    let second_cfg = second.config().unwrap();
    let (_, first_queue, first_sub) = first_cfg.channels.kmsg().unwrap();
    let (_, second_queue, second_sub) = second_cfg.channels.kmsg().unwrap();
    assert_ne!(first_queue, second_queue);
    assert_ne!(first_sub, second_sub);
    first_agent
        .sessions()
        .post_input(first.sid(), &msg("first only"))
        .await
        .unwrap();
    assert_eq!(
        channels
            .client()
            .get_queue_stats(first_queue)
            .await
            .unwrap()
            .message_count,
        1
    );
    assert_eq!(
        channels
            .client()
            .get_queue_stats(second_queue)
            .await
            .unwrap()
            .message_count,
        0
    );
}

struct Loopback(Arc<FsAgentStateClient>);

#[async_trait]
impl StateTransport for Loopback {
    async fn call(&self, method: &str, params: Value) -> libopendan::Result<Value> {
        libopendan::state::krpc::serve_call(self.0.as_ref(), APP, method, &params)
            .await
            .unwrap()
    }
}

#[tokio::test]
async fn rpc_and_forwarding_clients_share_the_persisted_reservation() {
    let env = Env::new();
    let fs = env.agent();
    let forwarding = ForwardingStateClient::new(fs.clone());
    let rpc = KrpcAgentStateClient::new(AGENT, Arc::new(Loopback(fs.clone())));
    let first = forwarding
        .sessions()
        .reserve_id(SessionKind::Work, "same-request", Some(&env.app_dir))
        .await
        .unwrap();
    assert_eq!(
        rpc.sessions()
            .reserve_id(SessionKind::Work, "same-request", Some(&env.app_dir))
            .await
            .unwrap(),
        first
    );
    assert_eq!(
        fs.sessions()
            .reserve_id(SessionKind::Work, "same-request", Some(&env.app_dir))
            .await
            .unwrap(),
        first
    );
    let next = rpc
        .sessions()
        .reserve_id(SessionKind::Work, "next-request", Some(&env.app_dir))
        .await
        .unwrap();
    assert_ne!(next, first);
    assert!(next.ends_with("-2"));
}

#[tokio::test]
async fn abandoned_temporary_directory_does_not_block_or_get_overwritten() {
    let env = Env::new();
    let mut spec = work_spec("temporary recovery");
    spec.idempotency_key = Some("orphan".into());
    let key = libopendan::ids::session_identity_key(
        spec.kind,
        AGENT,
        APP,
        spec.idempotency_key.as_deref(),
        None,
    )
    .unwrap();
    let sid = env
        .agent()
        .sessions()
        .reserve_id(spec.kind, &key, Some(&env.app_dir))
        .await
        .unwrap();
    let abandoned = env.app_dir.join(format!(".tmp-{sid}-0"));
    std::fs::create_dir(&abandoned).unwrap();
    std::fs::write(abandoned.join("sentinel"), b"keep").unwrap();
    let sd = env.create_work(spec).await;
    assert_eq!(sd.sid(), sid);
    assert_eq!(std::fs::read(abandoned.join("sentinel")).unwrap(), b"keep");
    assert!(!env.app_dir.join(format!(".tmp-{sid}-1")).exists());
    daily_id(&sd, SessionKind::Work);
}
