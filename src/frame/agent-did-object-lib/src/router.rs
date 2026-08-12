use serde::{Deserialize, Serialize};
use url::Url;

use crate::config::{ObjectRoute, ObjectRouteConfig};
use crate::error::AgentDIDObjectError;
use crate::types::ObjectRef;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RouteMethod {
    Read,
    XCall,
    SubscribeEvent,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RouteMatchType {
    Exact,
    UrlPrefix,
    Scheme,
    Glob,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RouteTrace {
    pub route_id: String,
    pub adapter: String,
    pub method: RouteMethod,
}

#[derive(Clone, Debug)]
pub struct RouteMatch {
    pub route: ObjectRoute,
    pub order: usize,
    pub trace: RouteTrace,
}

#[derive(Clone, Debug)]
pub struct ObjectRouter {
    config: ObjectRouteConfig,
}

impl ObjectRouter {
    pub fn new(config: ObjectRouteConfig) -> Self {
        Self { config }
    }

    pub fn config(&self) -> &ObjectRouteConfig {
        &self.config
    }

    pub fn route(
        &self,
        method: RouteMethod,
        object_ref: &ObjectRef,
    ) -> Result<RouteMatch, AgentDIDObjectError> {
        self.config
            .routes
            .iter()
            .enumerate()
            .filter(|(_, route)| route.allows_method(method))
            .filter(|(_, route)| route_matches(route, object_ref))
            .max_by(|(left_idx, left), (right_idx, right)| {
                left.priority
                    .cmp(&right.priority)
                    .then_with(|| right_idx.cmp(left_idx))
            })
            .map(|(order, route)| RouteMatch {
                route: route.clone(),
                order,
                trace: RouteTrace {
                    route_id: route.id.clone(),
                    adapter: route.adapter.clone(),
                    method,
                },
            })
            .ok_or_else(|| {
                AgentDIDObjectError::RouteNotFound(format!(
                    "no route for {:?} {}",
                    method, object_ref.normalized
                ))
            })
    }
}

fn route_matches(route: &ObjectRoute, object_ref: &ObjectRef) -> bool {
    match route.match_type {
        RouteMatchType::Exact => object_ref.normalized == route.pattern,
        RouteMatchType::UrlPrefix => {
            object_ref.is_url() && object_ref.normalized.starts_with(&route.pattern)
        }
        RouteMatchType::Scheme => Url::parse(&object_ref.normalized)
            .map(|url| url.scheme() == route.pattern)
            .unwrap_or(false),
        RouteMatchType::Glob => {
            if let Some(prefix) = route.pattern.strip_suffix('*') {
                object_ref.normalized.starts_with(prefix)
            } else {
                object_ref.normalized == route.pattern
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::config::{AdapterConfig, AdapterType};
    use serde_json::json;

    use super::*;

    fn config(routes: Vec<ObjectRoute>) -> ObjectRouteConfig {
        ObjectRouteConfig {
            version: 1,
            adapters: vec![
                AdapterConfig {
                    id: "a".to_string(),
                    adapter_type: AdapterType::Web,
                    endpoint: None,
                    auth_token_env: None,
                    options: json!({}),
                },
                AdapterConfig {
                    id: "b".to_string(),
                    adapter_type: AdapterType::Web,
                    endpoint: None,
                    auth_token_env: None,
                    options: json!({}),
                },
            ],
            routes,
        }
    }

    #[test]
    fn keeps_config_order_for_equal_priority() {
        let router = ObjectRouter::new(config(vec![
            ObjectRoute {
                id: "first".to_string(),
                priority: 1,
                match_type: RouteMatchType::Scheme,
                pattern: "https".to_string(),
                adapter: "a".to_string(),
                methods: vec![],
                options: json!({}),
            },
            ObjectRoute {
                id: "second".to_string(),
                priority: 1,
                match_type: RouteMatchType::Scheme,
                pattern: "https".to_string(),
                adapter: "b".to_string(),
                methods: vec![],
                options: json!({}),
            },
        ]));
        let found = router
            .route(
                RouteMethod::Read,
                &ObjectRef::parse("https://example.com").unwrap(),
            )
            .unwrap();
        assert_eq!(found.route.id, "first");
    }

    #[test]
    fn higher_priority_wins() {
        let router = ObjectRouter::new(config(vec![
            ObjectRoute {
                id: "low".to_string(),
                priority: 1,
                match_type: RouteMatchType::Scheme,
                pattern: "https".to_string(),
                adapter: "a".to_string(),
                methods: vec![],
                options: json!({}),
            },
            ObjectRoute {
                id: "high".to_string(),
                priority: 9,
                match_type: RouteMatchType::UrlPrefix,
                pattern: "https://example.com".to_string(),
                adapter: "b".to_string(),
                methods: vec![],
                options: json!({}),
            },
        ]));
        let found = router
            .route(
                RouteMethod::Read,
                &ObjectRef::parse("https://example.com/a").unwrap(),
            )
            .unwrap();
        assert_eq!(found.route.id, "high");
    }

    #[test]
    fn method_filter_is_applied() {
        let router = ObjectRouter::new(config(vec![ObjectRoute {
            id: "read-only".to_string(),
            priority: 1,
            match_type: RouteMatchType::Scheme,
            pattern: "https".to_string(),
            adapter: "a".to_string(),
            methods: vec![RouteMethod::Read],
            options: json!({}),
        }]));
        assert!(router
            .route(
                RouteMethod::XCall,
                &ObjectRef::parse("https://example.com").unwrap()
            )
            .is_err());
    }

    #[test]
    fn object_ref_accepts_only_hierarchical_urls() {
        assert_eq!(
            ObjectRef::parse("https://example.com/a").unwrap().scheme(),
            Some("https".to_string())
        );
        assert_eq!(
            ObjectRef::parse("obj://a/b").unwrap().scheme(),
            Some("obj".to_string())
        );
        assert!(ObjectRef::parse("did:web:example.com").is_err());
        assert!(ObjectRef::parse("camera.home").is_err());
    }
}
