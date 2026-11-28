use super::*;
use buckyos_api::{AiccUsecaseRoute, LockedValue, OnUnavailable};

async fn db() -> AiccStorage {
    AiccStorage::open("sqlite::memory:", RdbBackend::Sqlite)
        .await
        .unwrap()
}

fn routes(model: &str) -> AiccUsecaseRoutes {
    AiccUsecaseRoutes::from([(
        "llm.code".to_string(),
        AiccUsecaseRoute {
            model: Some(LockedValue::new(model.to_string())),
            on_unavailable: Some(LockedValue::new(OnUnavailable::SameModel)),
            enabled: None,
        },
    )])
}

#[tokio::test]
async fn the_user_layer_is_versioned_per_subject() {
    let db = db().await;
    assert_eq!(
        db.routing_layer("user:bob").await.unwrap(),
        (AiccUsecaseRoutes::new(), 0)
    );
    assert_eq!(
        db.put_routing_layer("user:bob", &routes("a@x"), 0, 1)
            .await
            .unwrap(),
        1
    );
    assert!(matches!(
        db.put_routing_layer("user:bob", &routes("b@x"), 0, 2).await,
        Err(PolicyWriteError::VersionConflict {
            expected: 0,
            actual: 1
        })
    ));
    assert_eq!(
        db.put_routing_layer("user:bob", &routes("b@x"), 1, 3)
            .await
            .unwrap(),
        2
    );
    let (stored, version) = db.routing_layer("user:bob").await.unwrap();
    assert_eq!(version, 2);
    assert_eq!(stored, routes("b@x"));
    assert_eq!(db.routing_layer("user:alice").await.unwrap().1, 0);
}

#[tokio::test]
async fn groups_belong_to_their_owner_and_take_their_bindings_with_them() {
    let db = db().await;
    let group = db
        .create_routing_group("user:bob", "Coding", &routes("a@x"), 1)
        .await
        .unwrap();
    assert_eq!(group.version, 1);
    assert!(group.group_id.starts_with("rg_"));
    db.set_routing_binding("user:bob", "jarvis.test", &group.group_id, 2)
        .await
        .unwrap();
    assert_eq!(
        db.routing_binding("user:bob", "jarvis.test")
            .await
            .unwrap()
            .as_deref(),
        Some(group.group_id.as_str())
    );
    // Another user neither sees nor changes it.
    assert!(db.routing_groups("user:alice").await.unwrap().is_empty());
    assert!(db
        .routing_binding("user:alice", "jarvis.test")
        .await
        .unwrap()
        .is_none());
    assert!(matches!(
        db.update_routing_group("user:alice", &group.group_id, 1, Some("x"), None, 3)
            .await,
        Err(PolicyWriteError::NotFound)
    ));
    assert!(matches!(
        db.delete_routing_group("user:alice", &group.group_id, 1)
            .await,
        Err(PolicyWriteError::NotFound)
    ));

    let listed = db.routing_groups("user:bob").await.unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].app_ids, vec!["jarvis.test".to_string()]);
    let updated = db
        .update_routing_group(
            "user:bob",
            &group.group_id,
            1,
            Some("Code"),
            Some(&routes("b@x")),
            4,
        )
        .await
        .unwrap();
    assert_eq!((updated.version, updated.name.as_str()), (2, "Code"));
    assert_eq!(updated.app_ids, vec!["jarvis.test".to_string()]);
    assert!(matches!(
        db.update_routing_group("user:bob", &group.group_id, 1, None, None, 5)
            .await,
        Err(PolicyWriteError::VersionConflict {
            expected: 1,
            actual: 2
        })
    ));
    let (owner, stored) = db.routing_group(&group.group_id).await.unwrap().unwrap();
    assert_eq!(owner, "user:bob");
    assert_eq!(stored.usecases, routes("b@x"));

    db.delete_routing_group("user:bob", &group.group_id, 2)
        .await
        .unwrap();
    assert!(db.routing_group(&group.group_id).await.unwrap().is_none());
    assert!(db.routing_bindings("user:bob").await.unwrap().is_empty());
    assert!(!db
        .remove_routing_binding("user:bob", "jarvis.test")
        .await
        .unwrap());
}

#[tokio::test]
async fn a_binding_names_one_group_per_app() {
    let db = db().await;
    let first = db
        .create_routing_group("user:bob", "A", &routes("a@x"), 1)
        .await
        .unwrap();
    let second = db
        .create_routing_group("user:bob", "B", &routes("b@x"), 1)
        .await
        .unwrap();
    db.set_routing_binding("user:bob", "app-1", &first.group_id, 1)
        .await
        .unwrap();
    db.set_routing_binding("user:bob", "app-1", &second.group_id, 2)
        .await
        .unwrap();
    assert_eq!(
        db.routing_bindings("user:bob").await.unwrap(),
        vec![AiccRoutingBinding {
            app_id: "app-1".into(),
            group_id: second.group_id.clone()
        }]
    );
    assert!(db
        .remove_routing_binding("user:bob", "app-1")
        .await
        .unwrap());
}
