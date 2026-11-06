//! In-process FunctionObject registry (TODO §2.1 D18).
//!
//! Built-in functions are registered at startup; their [`FunctionObject`]s are
//! published to the object store so every `fun_id` the compiler binds can be
//! read back as a named object. `/tool/<name>@<version>` semantic links resolve
//! here. Zone-level registries and Agent / pikg publication are later work.

use crate::error::{WorkflowError, WorkflowResult};
use crate::native::NativeFunction;
use crate::object_store::WorkflowObjectStore;
use buckyos_api::{parse_tool_ref, FunctionObject, FunctionRegistration};
use ndn_lib::ObjId;
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

#[derive(Clone)]
pub struct RegisteredFunction {
    pub object: FunctionObject,
    pub registration: FunctionRegistration,
    pub canonical: String,
    pub native: Option<Arc<dyn NativeFunction>>,
}

impl RegisteredFunction {
    pub fn fun_id(&self) -> &ObjId {
        &self.registration.fun_id
    }

    pub fn to_value(&self) -> Value {
        serde_json::json!({
            "fun_id": self.registration.fun_id.to_string(),
            "registration": self.registration,
            "object": self.object,
        })
    }
}

/// Everything a registration needs besides the hashed object.
#[derive(Debug, Clone)]
pub struct RegistrationSpec {
    pub name: String,
    pub version: String,
    pub description: String,
    pub timeout: Option<u64>,
    pub requirements: BTreeMap<String, u64>,
    pub best_run_weight: BTreeMap<String, u64>,
    pub affinity_type: buckyos_api::AffinityType,
}

impl RegistrationSpec {
    pub fn new(name: &str, version: &str) -> Self {
        Self {
            name: name.to_string(),
            version: version.to_string(),
            description: String::new(),
            timeout: None,
            requirements: BTreeMap::new(),
            best_run_weight: BTreeMap::new(),
            affinity_type: buckyos_api::AffinityType::Input,
        }
    }

    pub fn description(mut self, description: &str) -> Self {
        self.description = description.to_string();
        self
    }

    pub fn timeout(mut self, seconds: u64) -> Self {
        self.timeout = Some(seconds);
        self
    }

    pub fn requirement(mut self, key: &str, min: u64) -> Self {
        self.requirements.insert(key.to_string(), min);
        self
    }
}

#[derive(Default)]
pub struct FunctionRegistry {
    by_id: HashMap<String, Arc<RegisteredFunction>>,
    by_name: HashMap<(String, String), String>,
}

impl FunctionRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register_native(
        &mut self,
        spec: RegistrationSpec,
        object: FunctionObject,
        native: Arc<dyn NativeFunction>,
    ) -> WorkflowResult<ObjId> {
        self.register(spec, object, Some(native))
    }

    pub fn register(
        &mut self,
        spec: RegistrationSpec,
        object: FunctionObject,
        native: Option<Arc<dyn NativeFunction>>,
    ) -> WorkflowResult<ObjId> {
        object.validate().map_err(|err| {
            WorkflowError::FunctionRegistration(format!("{}@{}: {err}", spec.name, spec.version))
        })?;
        let (fun_id, canonical) = object.calc_obj_id().map_err(|err| {
            WorkflowError::FunctionRegistration(format!("{}@{}: {err}", spec.name, spec.version))
        })?;
        let key = (spec.name.clone(), spec.version.clone());
        if let Some(existing) = self.by_name.get(&key) {
            if existing != &fun_id.to_string() {
                return Err(WorkflowError::FunctionRegistration(format!(
                    "{}@{} is already registered as {existing}",
                    spec.name, spec.version
                )));
            }
        }
        let registration = FunctionRegistration {
            name: spec.name,
            version: spec.version,
            fun_id: fun_id.clone(),
            description: spec.description,
            timeout: spec.timeout,
            requirements: spec.requirements,
            best_run_weight: spec.best_run_weight,
            affinity_type: spec.affinity_type,
        };
        self.by_name.insert(key, fun_id.to_string());
        self.by_id.insert(
            fun_id.to_string(),
            Arc::new(RegisteredFunction {
                object,
                registration,
                canonical,
                native,
            }),
        );
        Ok(fun_id)
    }

    pub fn get(&self, fun_id: &str) -> Option<Arc<RegisteredFunction>> {
        self.by_id.get(fun_id).cloned()
    }

    pub fn resolve(&self, name: &str, version: &str) -> Option<Arc<RegisteredFunction>> {
        self.by_name
            .get(&(name.to_string(), version.to_string()))
            .and_then(|id| self.by_id.get(id))
            .cloned()
    }

    /// Resolves an executor string (`/tool/<name>@<version>` or
    /// `func::<obj_id>`) to a registered function.
    pub fn resolve_executor(&self, executor: &str) -> Option<Arc<RegisteredFunction>> {
        if let Some((name, version)) = parse_tool_ref(executor) {
            return self.resolve(name, version);
        }
        executor
            .strip_prefix("func::")
            .and_then(|id| self.get(id))
    }

    pub fn list(&self) -> Vec<Arc<RegisteredFunction>> {
        let mut out: Vec<_> = self.by_id.values().cloned().collect();
        out.sort_by(|a, b| {
            (&a.registration.name, &a.registration.version)
                .cmp(&(&b.registration.name, &b.registration.version))
        });
        out
    }

    pub fn is_empty(&self) -> bool {
        self.by_id.is_empty()
    }

    /// Writes every FunctionObject into the object store under its real id.
    pub async fn publish(&self, store: &dyn WorkflowObjectStore) -> WorkflowResult<()> {
        for function in self.by_id.values() {
            let value: Value = serde_json::from_str(&function.canonical)
                .map_err(|err| WorkflowError::Serialization(err.to_string()))?;
            store
                .put_json_with_id(&function.registration.fun_id.to_string(), &value)
                .await?;
        }
        Ok(())
    }
}
