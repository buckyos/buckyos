//! DV: the kmsg input path against a running BuckyOS zone (the real kmsg
//! service over kRPC). Ignored by default; on a DV Test OOD, as root:
//!
//! ```text
//! cargo test -p libopendan --test dv_kmsg -- --ignored --nocapture
//! ```
//!
//! Login follows xllm's `ensure_buckyos_runtime` (device key → verify-hub).

mod common;

use std::sync::Arc;

use buckyos_api::msg_queue::MsgQueueClient;
use common::*;
use libopendan::channel::KmsgChannels;
use libopendan::protocol::*;
use libopendan::runner::{drive, StopWhen};
use serde_json::json;

async fn real_kmsg() -> Arc<MsgQueueClient> {
    agent_tool::local_llm_context::ensure_buckyos_runtime()
        .await
        .expect("login to the local zone");
    let rt = buckyos_api::get_buckyos_api_runtime().expect("runtime");
    Arc::new(rt.get_msg_queue_client().await.expect("kmsg client"))
}

#[tokio::test]
#[ignore = "needs a running BuckyOS DV zone with kmsg"]
async fn dv_real_kmsg_drives_a_work_session() {
    let client = real_kmsg().await;
    let env = Env::new();
    let channels = Arc::new(KmsgChannels::new(client.clone()));
    let agent = Arc::new(
        libopendan::FsAgentStateClient::open(&env.agent_root, AGENT, Some(client.clone()), None)
            .unwrap(),
    );
    // Queue + subscription are created in the real kmsg (twice: idempotent).
    let mut spec = work_spec("dv: answer the message");
    spec.idempotency_key = Some(format!("dv-{}", uuid::Uuid::new_v4().simple()));
    let sd = libopendan::create_session(&env.app_dir, spec.clone(), agent.as_ref(), APP, channels.as_ref())
        .await
        .unwrap();
    let again = libopendan::create_session(&env.app_dir, spec, agent.as_ref(), APP, channels.as_ref())
        .await
        .unwrap();
    assert_eq!(sd.sid(), again.sid());
    let queue = queue_of(&sd);
    println!("queue {queue}");
    libopendan::post_input(agent.as_ref(), sd.sid(), &Input::msg("dv-m1", "ping from DV"), APP)
        .await
        .unwrap();
    let llm = ScriptedLlm::new(|req, _| {
        assert!(last_user_text(req).contains("ping from DV"));
        text("pong")
    });
    let mut deps = env.deps(llm.clone());
    deps.inputs = channels.clone();
    deps.agent = agent.clone();
    let r = drive(&sd, &deps, StopWhen::Finished).await;
    assert!(r.is_finished(), "{r:?}");
    assert_eq!(llm.count(), 1);
    let st = sd.state().unwrap();
    assert_eq!(st.source("q").acked_index, 1);
    // The subscription cursor was acknowledged past the consumed message.
    let sub = match &sd.config().unwrap().channels.inputs[0] {
        InputSourceConfig::Kmsg { subscriber, .. } => subscriber.clone(),
        _ => unreachable!(),
    };
    let left = client.fetch_messages(&sub, 10, false).await.unwrap();
    assert!(left.is_empty(), "acked: {left:?}");
    // A late message after finish is rejected and acknowledged too.
    libopendan::channel::kmsg::post_to_queue(&client, &queue, &Input::msg("dv-m2", "late"), APP)
        .await
        .unwrap();
    assert!(drive(&sd, &deps, StopWhen::Idle).await.is_finished());
    assert_eq!(sd.state().unwrap().source("q").acked_index, 2);
    assert!(client.fetch_messages(&sub, 10, false).await.unwrap().is_empty());
    let _ = client.delete_queue(&queue).await;
    let _ = json!(null);
}
