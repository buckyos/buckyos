use serde_json::Value;
use serde_json::json;
use std::collections::HashMap;
use serde::{Serialize,Deserialize};
#[derive(Debug,Clone,Serialize,Deserialize)]
pub enum JsonValueAction {
    Update(String),//完整更新
    Set(HashMap<String,Value>),//当成json设置其中的一个值,针对一个对象,set可以是一个数组
    Remove,//删除
}

pub fn set_json_by_path(data: &mut Value, path: &str, value: Option<&Value>) {
    // 将路径按 '/' 分割，移除可能的前导 '/'
    let parts: Vec<&str> = path.trim_start_matches('/').split('/').collect();
    
    // 如果路径为空，直接替换或删除整个 Value
    if parts.is_empty() {
        match value {
            Some(v) => *data = v.clone(),
            None => *data = json!(null),
        }
        return;
    }
    
    // 从根开始遍历和构建路径
    let mut current = data;
    for (i, &part) in parts.iter().enumerate() {
        // 最后一个部分：设置或删除值
        if i == parts.len() - 1 {
            if let Value::Object(map) = current {
                match value {
                    Some(v) => {
                        map.insert(part.to_string(), v.clone());
                    },
                    None => {
                        map.remove(part);
                    }
                }
            }
            break;
        }
        
        // 确保中间路径存在
        current = current
            .as_object_mut()
            .unwrap_or_else(|| panic!("Cannot create path"))
            .entry(part)
            .or_insert_with(|| json!({}));
    }
}

pub fn get_by_json_path(data: &Value, path: &str) -> Option<Value> {
    let parts: Vec<&str> = path.trim_start_matches('/').split('/').collect();
    let mut current = data;
    for part in parts {
        current = if let Ok(index) = part.parse::<usize>() {
            // 如果 part 可以解析为数字，则作为数组索引处理
            current.get(index).unwrap_or(&json!(null))
        } else {
            // 否则作为对象键处理
            current.get(part).unwrap_or(&json!(null))
        };
    }
    Some(current.clone())
}

pub fn extend_json_action_map(dest_map: &mut HashMap<String, JsonValueAction>, from_map: &HashMap<String, JsonValueAction>) {
    for (key, value) in from_map.iter() {
        let old_value = dest_map.get_mut(key);
        match old_value {
            Some(old_value) => {
                match value {
                    JsonValueAction::Update(new_value) => {
                        *old_value = JsonValueAction::Update(new_value.clone());
                    },
                    JsonValueAction::Set(new_value) => {
                        match old_value {
                            JsonValueAction::Set(old_value) => {
                                old_value.extend(new_value.clone());
                            }
                            _ => {
                                *old_value = JsonValueAction::Set(new_value.clone());
                            }
                        }
                    },
                    JsonValueAction::Remove => {
                        *old_value = JsonValueAction::Remove;
                    }
                }

            }
            None => {
                dest_map.insert(key.clone(), value.clone());
            }
        }
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_set_json_by_path() {
        let mut data = json!({
            "user": {
                "name": "Alice",
                "age": 30,
                "address": {
                    "city": "New York"
                }
            }
        });

        let data2 =  json!({
            "user": {
                "age": 30,
                "name": "Alice",
                "address": {
                    "city": "New York"
                }
            }
        });

        assert_eq!(data,data2);
    
        // 设置值
        set_json_by_path(&mut data, "/user/address/add/street", Some(&json!("Bob")));
        println!("{}", data);
        // 删除字段
        set_json_by_path(&mut data, "/user/age", None);
        println!("{}", data);
        // 删除嵌套字段
        set_json_by_path(&mut data, "/user/address/city", None);
        println!("{}", data);
        // 完全删除 address 对象
        set_json_by_path(&mut data, "/user/address", None);
        println!("{}", data);
    }


    #[test]
    fn test_get_by_json_path() {
        let data = json!({
            "user": {
                "name": "Alice",
                "age": 30,
                "address": {
                    "city": "New York"
                },
                "friends": [
                    {
                        "name": "Bob",
                        "age": 25
                    },
                    {
                        "name": "Charlie",
                        "age": 28
                    }
                ]
            }
        });

        let name = get_by_json_path(&data, "/user/friends/0/name").unwrap();
        assert_eq!(name.as_str().unwrap(),"Bob");

    }
}