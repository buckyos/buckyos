use buckyos_api::msg_queue::*;
use kRPC::RPCContext;
use kmsg::sled_msg_queue::SledMsgQueue;
use std::collections::HashSet;
use std::sync::Arc;
use std::time::Instant;

fn make_message(text: &str) -> Message {
    Message::new(text.as_bytes().to_vec())
}

fn old_message(text: &str) -> Message {
    let mut message = make_message(text);
    message.created_at = 1;
    message
}

#[tokio::test(flavor = "current_thread")]
async fn persistence_reopen_preserves_queue_messages_and_cursor()
-> Result<(), Box<dyn std::error::Error>> {
    let temp = tempfile::TempDir::new()?;
    let path = temp.path().to_path_buf();

    let queue = SledMsgQueue::new_in_dir(&path)?;
    let config = QueueConfig {
        sync_write: true,
        ..QueueConfig::default()
    };
    let queue_urn = queue
        .handle_create_queue(
            Some("persist"),
            "app",
            "owner",
            config,
            RPCContext::default(),
        )
        .await?;
    let first = queue
        .handle_post_message(&queue_urn, make_message("first"), RPCContext::default())
        .await?;
    let second = queue
        .handle_post_message(&queue_urn, make_message("second"), RPCContext::default())
        .await?;
    let sub_id = queue
        .handle_subscribe(
            &queue_urn,
            "user",
            "app",
            Some("persist-sub".to_string()),
            SubPosition::Earliest,
            RPCContext::default(),
        )
        .await?;
    queue
        .handle_commit_ack(&sub_id, first, RPCContext::default())
        .await?;
    drop(queue);

    let reopened = SledMsgQueue::new_in_dir(&path)?;
    let stats = reopened
        .handle_get_queue_stats(&queue_urn, RPCContext::default())
        .await?;
    // The acknowledged message was dropped (default), durably.
    assert_eq!(stats.message_count, 1);
    assert_eq!(stats.first_index, second);
    assert_eq!(stats.last_index, second);

    let history = reopened
        .handle_read_message(&queue_urn, first, 10, RPCContext::default())
        .await?;
    assert_eq!(
        history.iter().map(|msg| msg.index).collect::<Vec<_>>(),
        vec![second]
    );

    let pending = reopened
        .handle_fetch_messages(&sub_id, 10, false, RPCContext::default())
        .await?;
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].index, second);
    assert_eq!(pending[0].payload, b"second".to_vec());

    Ok(())
}

#[tokio::test(flavor = "current_thread")]
async fn sync_write_cursor_survives_reopen() -> Result<(), Box<dyn std::error::Error>> {
    let temp = tempfile::TempDir::new()?;
    let path = temp.path().to_path_buf();

    let queue = SledMsgQueue::new_in_dir(&path)?;
    let config = QueueConfig {
        sync_write: true,
        ..QueueConfig::default()
    };
    let queue_urn = queue
        .handle_create_queue(Some("sync"), "app", "owner", config, RPCContext::default())
        .await?;
    let first = queue
        .handle_post_message(&queue_urn, make_message("first"), RPCContext::default())
        .await?;
    let second = queue
        .handle_post_message(&queue_urn, make_message("second"), RPCContext::default())
        .await?;
    let sub_id = queue
        .handle_subscribe(
            &queue_urn,
            "user",
            "app",
            Some("sync-sub".to_string()),
            SubPosition::Earliest,
            RPCContext::default(),
        )
        .await?;
    let messages = queue
        .handle_fetch_messages(&sub_id, 1, false, RPCContext::default())
        .await?;
    assert_eq!(messages[0].index, first);
    queue
        .handle_commit_ack(&sub_id, second, RPCContext::default())
        .await?;
    drop(queue);

    let reopened = SledMsgQueue::new_in_dir(&path)?;
    let pending = reopened
        .handle_fetch_messages(&sub_id, 1, false, RPCContext::default())
        .await?;
    assert!(pending.is_empty());

    Ok(())
}

#[tokio::test(flavor = "current_thread")]
async fn max_messages_keeps_the_newest() -> Result<(), Box<dyn std::error::Error>> {
    let temp = tempfile::TempDir::new()?;
    let queue = SledMsgQueue::new_in_dir(temp.path())?;
    let config = QueueConfig {
        max_messages: Some(2),
        ..QueueConfig::default()
    };
    let queue_urn = queue
        .handle_create_queue(
            Some("max-messages"),
            "app",
            "owner",
            config,
            RPCContext::default(),
        )
        .await?;

    for seq in 0..3 {
        queue
            .handle_post_message(
                &queue_urn,
                make_message(&format!("m{}", seq)),
                RPCContext::default(),
            )
            .await?;
    }

    let stats = queue
        .handle_get_queue_stats(&queue_urn, RPCContext::default())
        .await?;
    assert_eq!(
        (stats.message_count, stats.first_index, stats.last_index),
        (2, 2, 3)
    );

    Ok(())
}

#[tokio::test(flavor = "current_thread")]
async fn retention_seconds_drops_expired_messages() -> Result<(), Box<dyn std::error::Error>> {
    let temp = tempfile::TempDir::new()?;
    let queue = SledMsgQueue::new_in_dir(temp.path())?;
    let config = QueueConfig {
        retention_seconds: Some(3600),
        ..QueueConfig::default()
    };
    let queue_urn = queue
        .handle_create_queue(
            Some("retention"),
            "app",
            "owner",
            config,
            RPCContext::default(),
        )
        .await?;
    queue
        .handle_post_message(&queue_urn, old_message("old"), RPCContext::default())
        .await?;
    queue
        .handle_post_message(&queue_urn, make_message("fresh"), RPCContext::default())
        .await?;

    let messages = queue
        .handle_read_message(&queue_urn, 1, 10, RPCContext::default())
        .await?;
    assert_eq!(messages.iter().map(|m| m.index).collect::<Vec<_>>(), vec![2]);
    assert_eq!(queue.sweep_retention()?, 0);

    Ok(())
}

async fn work_queue(
    queue: &SledMsgQueue,
    name: &str,
) -> Result<String, Box<dyn std::error::Error>> {
    // The default: acknowledged messages are not kept.
    Ok(queue
        .handle_create_queue(Some(name), "app", "owner", QueueConfig::default(), RPCContext::default())
        .await?)
}

async fn stats_of(
    queue: &SledMsgQueue,
    queue_urn: &str,
) -> Result<(u64, u64, u64), Box<dyn std::error::Error>> {
    let stats = queue
        .handle_get_queue_stats(queue_urn, RPCContext::default())
        .await?;
    Ok((stats.message_count, stats.first_index, stats.last_index))
}

#[tokio::test(flavor = "current_thread")]
async fn ack_keeps_only_what_is_not_consumed() -> Result<(), Box<dyn std::error::Error>> {
    let temp = tempfile::TempDir::new()?;
    let queue = SledMsgQueue::new_in_dir(temp.path())?;
    let queue_urn = work_queue(&queue, "work").await?;
    for seq in 0..3 {
        queue
            .handle_post_message(&queue_urn, make_message(&format!("m{seq}")), RPCContext::default())
            .await?;
    }
    // Without a subscription nothing is consumed, nothing is dropped.
    assert_eq!(stats_of(&queue, &queue_urn).await?, (3, 1, 3));

    let fast = queue
        .handle_subscribe(&queue_urn, "user", "app", Some("fast".into()), SubPosition::Earliest, RPCContext::default())
        .await?;
    let slow = queue
        .handle_subscribe(&queue_urn, "user", "app", Some("slow".into()), SubPosition::Earliest, RPCContext::default())
        .await?;
    // Peek does not consume.
    let peeked = queue
        .handle_fetch_messages(&fast, 10, false, RPCContext::default())
        .await?;
    assert_eq!(peeked.len(), 3);
    assert_eq!(stats_of(&queue, &queue_urn).await?, (3, 1, 3));

    // Trimmed up to the slowest subscription.
    queue.handle_commit_ack(&fast, 3, RPCContext::default()).await?;
    assert_eq!(stats_of(&queue, &queue_urn).await?, (3, 1, 3));
    queue.handle_commit_ack(&slow, 1, RPCContext::default()).await?;
    assert_eq!(stats_of(&queue, &queue_urn).await?, (2, 2, 3));
    // Acks are cumulative and never move back.
    queue.handle_commit_ack(&slow, 1, RPCContext::default()).await?;
    queue.handle_commit_ack(&fast, 2, RPCContext::default()).await?;
    assert!(
        queue
            .handle_fetch_messages(&fast, 10, false, RPCContext::default())
            .await?
            .is_empty()
    );
    // An index not assigned yet is refused.
    assert!(queue.handle_commit_ack(&slow, 4, RPCContext::default()).await.is_err());

    // The slowest consumer leaving releases what the others consumed.
    queue.handle_unsubscribe(&slow, RPCContext::default()).await?;
    assert_eq!(stats_of(&queue, &queue_urn).await?, (0, 0, 3));
    assert!(queue.handle_read_message(&queue_urn, 1, 10, RPCContext::default()).await?.is_empty());

    // Indexes keep growing on the emptied queue; a new subscriber starts
    // at the next one, auto commit consumes as well.
    let index = queue
        .handle_post_message(&queue_urn, make_message("m3"), RPCContext::default())
        .await?;
    assert_eq!(index, 4);
    assert_eq!(stats_of(&queue, &queue_urn).await?, (1, 4, 4));
    let late = queue
        .handle_subscribe(&queue_urn, "user", "app", Some("late".into()), SubPosition::Latest, RPCContext::default())
        .await?;
    let msgs = queue
        .handle_fetch_messages(&fast, 10, true, RPCContext::default())
        .await?;
    assert_eq!(msgs.iter().map(|m| m.index).collect::<Vec<_>>(), vec![4]);
    assert_eq!(stats_of(&queue, &queue_urn).await?, (0, 0, 4));
    queue.handle_unsubscribe(&late, RPCContext::default()).await?;

    Ok(())
}

#[tokio::test(flavor = "current_thread")]
async fn keep_acked_separates_ack_from_deletion() -> Result<(), Box<dyn std::error::Error>> {
    let temp = tempfile::TempDir::new()?;
    let queue = SledMsgQueue::new_in_dir(temp.path())?;
    let config = QueueConfig {
        keep_acked: true,
        ..QueueConfig::default()
    };
    let queue_urn = queue
        .handle_create_queue(Some("log"), "app", "owner", config, RPCContext::default())
        .await?;
    for seq in 0..3 {
        queue
            .handle_post_message(&queue_urn, make_message(&format!("m{seq}")), RPCContext::default())
            .await?;
    }
    let sub = queue
        .handle_subscribe(&queue_urn, "user", "app", Some("reader".into()), SubPosition::Earliest, RPCContext::default())
        .await?;
    queue.handle_commit_ack(&sub, 3, RPCContext::default()).await?;
    assert_eq!(stats_of(&queue, &queue_urn).await?, (3, 1, 3));
    queue
        .handle_seek(&sub, SubPosition::Earliest, RPCContext::default())
        .await?;
    let replay = queue
        .handle_fetch_messages(&sub, 10, false, RPCContext::default())
        .await?;
    assert_eq!(replay.iter().map(|m| m.index).collect::<Vec<_>>(), vec![1, 2, 3]);

    Ok(())
}

#[tokio::test(flavor = "current_thread")]
async fn ack_trimming_subscriptions_survive_reopen() -> Result<(), Box<dyn std::error::Error>> {
    let temp = tempfile::TempDir::new()?;
    let path = temp.path().to_path_buf();
    let queue = SledMsgQueue::new_in_dir(&path)?;
    let queue_urn = work_queue(&queue, "reopen").await?;
    for seq in 0..2 {
        queue
            .handle_post_message(&queue_urn, make_message(&format!("m{seq}")), RPCContext::default())
            .await?;
    }
    for sub in ["a", "b"] {
        queue
            .handle_subscribe(&queue_urn, "user", "app", Some(sub.into()), SubPosition::Earliest, RPCContext::default())
            .await?;
    }
    queue.handle_commit_ack("a", 2, RPCContext::default()).await?;
    drop(queue);

    let queue = SledMsgQueue::new_in_dir(&path)?;
    assert_eq!(stats_of(&queue, &queue_urn).await?, (2, 1, 2));
    queue.handle_commit_ack("b", 2, RPCContext::default()).await?;
    assert_eq!(stats_of(&queue, &queue_urn).await?, (0, 0, 2));
    queue.handle_delete_queue(&queue_urn, RPCContext::default()).await?;
    assert!(queue.handle_commit_ack("a", 2, RPCContext::default()).await.is_err());

    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn delete_before_never_loses_a_concurrent_post() -> Result<(), Box<dyn std::error::Error>> {
    let temp = tempfile::TempDir::new()?;
    let queue = Arc::new(SledMsgQueue::new_in_dir(temp.path())?);
    let queue_urn = queue
        .handle_create_queue(Some("race"), "app", "owner", QueueConfig::default(), RPCContext::default())
        .await?;

    let mut producers = Vec::new();
    for p in 0..4 {
        let queue = queue.clone();
        let queue_urn = queue_urn.clone();
        producers.push(tokio::spawn(async move {
            let mut indexes = Vec::new();
            for seq in 0..200 {
                indexes.push(
                    queue
                        .handle_post_message(&queue_urn, make_message(&format!("p{p}-{seq}")), RPCContext::default())
                        .await
                        .unwrap(),
                );
            }
            indexes
        }));
    }
    let trimmer = {
        let queue = queue.clone();
        let queue_urn = queue_urn.clone();
        tokio::spawn(async move {
            let mut bound = 1;
            for _ in 0..200 {
                let stats = queue
                    .handle_get_queue_stats(&queue_urn, RPCContext::default())
                    .await
                    .unwrap();
                bound = bound.max(stats.last_index.saturating_sub(5));
                queue
                    .handle_delete_message_before(&queue_urn, bound, RPCContext::default())
                    .await
                    .unwrap();
                tokio::task::yield_now().await;
            }
            bound
        })
    };

    let mut all = HashSet::new();
    for producer in producers {
        for index in producer.await? {
            assert!(all.insert(index), "index {index} assigned twice");
        }
    }
    let bound = trimmer.await?;
    assert_eq!(all.len(), 800);
    // Everything at or above the last bound is still there, once.
    let kept = queue
        .handle_read_message(&queue_urn, bound, 1000, RPCContext::default())
        .await?;
    assert_eq!(kept.len() as u64, 800 - bound + 1);
    let (count, first, last) = stats_of(&queue, &queue_urn).await?;
    assert_eq!((count, first, last), (800 - bound + 1, bound, 800));

    Ok(())
}

#[tokio::test(flavor = "current_thread")]
async fn permission_config_currently_ignores_rpc_context() -> Result<(), Box<dyn std::error::Error>>
{
    let temp = tempfile::TempDir::new()?;
    let queue = SledMsgQueue::new_in_dir(temp.path())?;
    let config = QueueConfig {
        other_app_can_read: false,
        other_app_can_write: false,
        other_user_can_read: false,
        other_user_can_write: false,
        ..QueueConfig::default()
    };
    let queue_urn = queue
        .handle_create_queue(
            Some("acl"),
            "owner-app",
            "owner-user",
            config,
            RPCContext::default(),
        )
        .await?;

    let other_ctx = RPCContext {
        token: Some("other-user-session".to_string()),
        ..RPCContext::default()
    };
    let index = queue
        .handle_post_message(&queue_urn, make_message("foreign-write"), other_ctx.clone())
        .await?;
    assert_eq!(index, 1, "write permission fields are ignored");

    let messages = queue
        .handle_read_message(&queue_urn, 1, 1, other_ctx)
        .await?;
    assert_eq!(messages.len(), 1, "read permission fields are ignored");

    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_post_indexes_are_unique_and_stats_are_correct()
-> Result<(), Box<dyn std::error::Error>> {
    let temp = tempfile::TempDir::new()?;
    let queue = Arc::new(SledMsgQueue::new_in_dir(temp.path())?);
    let queue_urn = queue
        .handle_create_queue(
            Some("concurrent"),
            "app",
            "owner",
            QueueConfig::default(),
            RPCContext::default(),
        )
        .await?;

    let mut handles = Vec::new();
    for worker in 0..10 {
        let queue = queue.clone();
        let queue_urn = queue_urn.clone();
        handles.push(tokio::spawn(async move {
            let mut indexes = Vec::new();
            for seq in 0..100 {
                let index = queue
                    .handle_post_message(
                        &queue_urn,
                        make_message(&format!("w{}-{}", worker, seq)),
                        RPCContext::default(),
                    )
                    .await?;
                indexes.push(index);
            }
            Ok::<_, kRPC::RPCErrors>(indexes)
        }));
    }

    let mut indexes = Vec::new();
    for handle in handles {
        indexes.extend(handle.await??);
    }
    indexes.sort_unstable();
    assert_eq!(indexes.len(), 1000);
    assert_eq!(indexes.iter().copied().collect::<HashSet<_>>().len(), 1000);
    assert_eq!(indexes.first().copied(), Some(1));
    assert_eq!(indexes.last().copied(), Some(1000));

    let stats = queue
        .handle_get_queue_stats(&queue_urn, RPCContext::default())
        .await?;
    assert_eq!(stats.message_count, 1000);
    assert_eq!(stats.first_index, 1);
    assert_eq!(stats.last_index, 1000);

    Ok(())
}

#[tokio::test(flavor = "current_thread")]
#[ignore]
async fn post_and_fetch_baseline() -> Result<(), Box<dyn std::error::Error>> {
    let temp = tempfile::TempDir::new()?;
    let queue = SledMsgQueue::new_in_dir(temp.path())?;
    let queue_urn = queue
        .handle_create_queue(
            Some("baseline"),
            "app",
            "owner",
            QueueConfig::default(),
            RPCContext::default(),
        )
        .await?;

    let start = Instant::now();
    for seq in 0..10_000 {
        queue
            .handle_post_message(
                &queue_urn,
                Message::new(vec![seq as u8; 1024]),
                RPCContext::default(),
            )
            .await?;
    }
    let post_elapsed = start.elapsed();

    let sub_id = queue
        .handle_subscribe(
            &queue_urn,
            "user",
            "app",
            Some("baseline-sub".to_string()),
            SubPosition::Earliest,
            RPCContext::default(),
        )
        .await?;
    let start = Instant::now();
    let mut fetched = 0usize;
    while fetched < 10_000 {
        let batch = queue
            .handle_fetch_messages(&sub_id, 100, true, RPCContext::default())
            .await?;
        if batch.is_empty() {
            break;
        }
        fetched += batch.len();
    }
    let fetch_elapsed = start.elapsed();

    assert_eq!(fetched, 10_000);
    eprintln!(
        "{{\"kmsg_post_10k_ms\":{},\"kmsg_fetch_10k_ms\":{}}}",
        post_elapsed.as_millis(),
        fetch_elapsed.as_millis()
    );

    Ok(())
}

#[tokio::test(flavor = "current_thread")]
#[ignore]
async fn fetch_without_auto_commit_ack_baseline() -> Result<(), Box<dyn std::error::Error>> {
    let temp = tempfile::TempDir::new()?;
    let queue = SledMsgQueue::new_in_dir(temp.path())?;
    let queue_urn = queue
        .handle_create_queue(
            Some("ack-baseline"),
            "app",
            "owner",
            QueueConfig::default(),
            RPCContext::default(),
        )
        .await?;

    for seq in 0..10_000 {
        queue
            .handle_post_message(
                &queue_urn,
                Message::new(vec![seq as u8; 256]),
                RPCContext::default(),
            )
            .await?;
    }

    let sub_id = queue
        .handle_subscribe(
            &queue_urn,
            "user",
            "app",
            Some("ack-baseline-sub".to_string()),
            SubPosition::Earliest,
            RPCContext::default(),
        )
        .await?;
    let start = Instant::now();
    let mut fetched = 0usize;
    let mut last_index = 0;
    while fetched < 10_000 {
        let batch = queue
            .handle_fetch_messages(&sub_id, 100, false, RPCContext::default())
            .await?;
        if batch.is_empty() {
            break;
        }
        last_index = batch.last().unwrap().index;
        queue
            .handle_commit_ack(&sub_id, last_index, RPCContext::default())
            .await?;
        fetched += batch.len();
    }
    let elapsed = start.elapsed();

    assert_eq!(fetched, 10_000);
    assert_eq!(last_index, 10_000);
    eprintln!(
        "{{\"kmsg_fetch_ack_10k_ms\":{},\"kmsg_fetch_ack_count\":{}}}",
        elapsed.as_millis(),
        fetched
    );

    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore]
async fn concurrent_post_10k_baseline() -> Result<(), Box<dyn std::error::Error>> {
    let temp = tempfile::TempDir::new()?;
    let queue = Arc::new(SledMsgQueue::new_in_dir(temp.path())?);
    let queue_urn = queue
        .handle_create_queue(
            Some("concurrent-baseline"),
            "app",
            "owner",
            QueueConfig::default(),
            RPCContext::default(),
        )
        .await?;

    let start = Instant::now();
    let mut handles = Vec::new();
    for worker in 0..10 {
        let queue = queue.clone();
        let queue_urn = queue_urn.clone();
        handles.push(tokio::spawn(async move {
            let mut indexes = Vec::new();
            for seq in 0..1000 {
                let index = queue
                    .handle_post_message(
                        &queue_urn,
                        make_message(&format!("worker-{}-{}", worker, seq)),
                        RPCContext::default(),
                    )
                    .await?;
                indexes.push(index);
            }
            Ok::<_, kRPC::RPCErrors>(indexes)
        }));
    }

    let mut indexes = Vec::new();
    for handle in handles {
        indexes.extend(handle.await??);
    }
    let elapsed = start.elapsed();

    indexes.sort_unstable();
    assert_eq!(indexes.len(), 10_000);
    assert_eq!(
        indexes.iter().copied().collect::<HashSet<_>>().len(),
        10_000
    );
    assert_eq!(indexes.first().copied(), Some(1));
    assert_eq!(indexes.last().copied(), Some(10_000));

    eprintln!(
        "{{\"kmsg_concurrent_post_10k_ms\":{},\"kmsg_concurrent_post_count\":{}}}",
        elapsed.as_millis(),
        indexes.len()
    );

    Ok(())
}

#[tokio::test(flavor = "current_thread")]
#[ignore]
async fn sync_write_post_cost_baseline() -> Result<(), Box<dyn std::error::Error>> {
    let sync_false_ms = post_1k_with_sync_write(false).await?;
    let sync_true_ms = post_1k_with_sync_write(true).await?;

    eprintln!(
        "{{\"kmsg_sync_write_false_post_1k_ms\":{},\"kmsg_sync_write_true_post_1k_ms\":{}}}",
        sync_false_ms, sync_true_ms
    );

    Ok(())
}

async fn post_1k_with_sync_write(sync_write: bool) -> Result<u128, Box<dyn std::error::Error>> {
    let temp = tempfile::TempDir::new()?;
    let queue = SledMsgQueue::new_in_dir(temp.path())?;
    let config = QueueConfig {
        sync_write,
        ..QueueConfig::default()
    };
    let queue_urn = queue
        .handle_create_queue(
            Some(if sync_write {
                "sync-true"
            } else {
                "sync-false"
            }),
            "app",
            "owner",
            config,
            RPCContext::default(),
        )
        .await?;

    let start = Instant::now();
    for seq in 0..1000 {
        queue
            .handle_post_message(
                &queue_urn,
                Message::new(vec![seq as u8; 1024]),
                RPCContext::default(),
            )
            .await?;
    }

    Ok(start.elapsed().as_millis())
}
