
use serde::{Serialize,Deserialize};
use std::collections::HashMap;
use super::object::ObjId;
use super::mtree::MerkleTreeObject;

pub struct ObjectMap {

    pub objects: HashMap<ObjId,Vec<String>>,
}