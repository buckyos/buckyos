//! The FeedObject shell of a personal-site article (`doc/opendan/protocol/Personal Site Protocol.md` §3.4):
//! the vectors `src/tools/buckyos-agent/lib/site/feed.ts` is checked against give the same verdict and ObjId here.

use homestation::protocol::{obj_id_of, validate_feed_object, FeedObject, OBJ_TYPE_FEED};
use serde_json::Value;

enum Refused {
    Shape(String),
    Rule(String),
}

fn accept(object: &Value) -> Result<String, Refused> {
    let parsed: FeedObject = serde_json::from_value(object.clone()).map_err(|e| Refused::Shape(e.to_string()))?;
    validate_feed_object(&parsed).map_err(Refused::Rule)?;
    obj_id_of(OBJ_TYPE_FEED, object).map(|(id, _)| id).map_err(Refused::Rule)
}

#[test]
fn site_feed_object_vectors_get_the_same_verdict_and_obj_id() {
    let vectors: Value = serde_json::from_str(include_str!("../../../../doc/opendan/protocol/fixtures/site/feed_object_vectors.json")).unwrap();
    let (valid, invalid) = (vectors["valid"].as_array().unwrap(), vectors["invalid"].as_array().unwrap());
    assert!(!valid.is_empty() && !invalid.is_empty());
    for vector in valid {
        match accept(&vector["object"]) {
            Ok(id) => assert_eq!(id, vector["object_id"].as_str().unwrap(), "{}", vector["name"]),
            Err(Refused::Shape(why)) | Err(Refused::Rule(why)) => panic!("{}: {why}", vector["name"]),
        }
    }
    for vector in invalid {
        match accept(&vector["object"]) {
            Ok(id) => panic!("{} is accepted as {id}", vector["name"]),
            Err(Refused::Rule(why)) => assert!(why.contains(vector["problem"].as_str().unwrap()), "{}: {why}", vector["name"]),
            Err(Refused::Shape(_)) => {}
        }
    }
}
