
use std::collections::HashMap;
use serde::{Serialize,Deserialize};
use serde_json::Value;
use serde_json::json;

#[derive(Debug,Clone,Serialize,Deserialize)]
pub enum KVAction {
    Create(String),//创建一个节点并设置值
    Update(String),//完整更新
    Append(String),//追加
    SetByJsonPath(HashMap<String,Option<Value>>),//当成json设置其中的一个值,针对一个对象,set可以是一个数组
    Remove,//删除
    //Create(String),
}

pub fn split_json_path(path: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;
    let mut escaped = false;

    for c in path.chars() {
        match c {
            '\\' if !escaped => escaped = true,
            '"' if !escaped => in_quotes = !in_quotes,
            '/' if !in_quotes && !escaped => {
                if !current.is_empty() {
                    parts.push(current.trim().to_string());
                    current = String::new();
                }
            },
            _ => {
                if escaped && c != '"' && c != '\\' {
                    current.push('\\');
                }
                current.push(c);
                escaped = false;
            }
        }
    }
    
    if !current.is_empty() {
        parts.push(current.trim().to_string());
    }
    
    parts.into_iter()
        .filter(|s| !s.is_empty())
        .collect()
}

// pub fn set_json_by_path(data: &mut Value, path: &str, value: Option<&Value>) {
//     if value.is_some() {
//         let _ = data.merge_in(path, &value.unwrap());
//     } else {
//         let _ = data.merge_in(path, &json!(null));
//     }
// } 

pub fn set_json_by_path(data: &mut Value, path: &str, value: Option<&Value>) {
    // 使用新的路径解析方法
    let parts = split_json_path(path);
    
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
    for (i, part) in parts.iter().enumerate() {
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

pub fn extend_kv_action_map(dest_map: &mut HashMap<String, KVAction>, from_map: &HashMap<String, KVAction>) {
    for (key, value) in from_map.iter() {
        let old_value = dest_map.get_mut(key);
        match old_value {
            Some(old_value) => {
                match value {
                    KVAction::Create(new_value) => {
                        *old_value = KVAction::Create(new_value.clone());
                    },
                    KVAction::Update(new_value) => {
                        *old_value = KVAction::Update(new_value.clone());
                    },
                    KVAction::Append(new_value) => {
                        *old_value = KVAction::Append(new_value.clone());
                    },
                    KVAction::SetByJsonPath(new_value) => {
                        match old_value {
                            KVAction::SetByJsonPath(old_value) => {
                                old_value.extend(new_value.clone());
                            }
                            _ => {
                                *old_value = KVAction::SetByJsonPath(new_value.clone());
                            }
                        }
                    },
                    KVAction::Remove => {
                        *old_value = KVAction::Remove;
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
    fn test_hash_map_option_value() {
        let mut test_map:HashMap<String,Option<Value>> = HashMap::new();
 
        test_map.insert("state".to_string(),None);
        test_map.insert("abc".to_string(),Some(json!("123")));
        let test_value = serde_json::to_value(test_map).unwrap();
        let test_str = serde_json::to_string(&test_value).unwrap();
        let test_value2 : HashMap<String,Option<Value>> = serde_json::from_str(&test_str).unwrap();
        for (key,value) in test_value2.iter() {
            println!("key:{},value:{:?}",key,value);
        }
    }

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
        let json_path = format!("servers/main_http_server/hosts/*/routes/\"/kapi/{}\"","ood1");
        set_json_by_path(&mut data,json_path.as_str(),Some(&json!({
            "upstream":format!("http://127.0.0.1:{}",3200),
        })));

        // 设置值
        set_json_by_path(&mut data, "state", Some(&json!("Normal")));
        println!("{}", data);
        // 设置值
        set_json_by_path(&mut data, "/user/name", Some(&json!("Bob")));
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

    #[test]
    fn test_split_json_path() {
        assert_eq!(
            split_json_path(r#"/state/"space add"/value"#),
            vec!["state", "space add", "value"]
        );
        assert_eq!(
            split_json_path(r#"/path/with\ space/value"#),
            vec!["path", r#"with\ space"#, "value"]
        );
    }

    #[test]
    fn test_set_json_by_path_with_spaces() {
        let mut data = json!({});
        set_json_by_path(&mut data, r#"/state/"space add"/value"#, Some(&json!("test")));
        assert_eq!(
            data,
            json!({
                "state": {
                    "space add": {
                        "value": "test"
                    }
                }
            })
        );
    }
}