use std::collections::HashMap;
use std::fmt::Debug;
use std::sync::Arc;
use serde_json::json;

// 定义处理上下文特征
pub trait Context: Debug {
    // 可以添加通用方法
}

// 使用类型别名定义Provider为一个接受Context的闭包
pub type ProviderFn = Arc<dyn Fn(&mut dyn Context) -> Result<(), String> + Send + Sync>;

// Provider工厂 - 返回闭包
pub type ProviderFnFactory = Arc<dyn Fn(&serde_json::Value) -> Result<ProviderFn, String> + Send + Sync>;

// 注册表 - 存储工厂闭包
pub struct ProviderFnRegistry {
    factories: HashMap<String, ProviderFnFactory>,
}

impl ProviderFnRegistry {
    pub fn new() -> Self {
        Self {
            factories: HashMap::new(),
        }
    }

    // 注册一个工厂闭包
    pub fn register<F>(&mut self, provider_type: &str, factory: F)
    where
        F: Fn(&serde_json::Value) -> Result<ProviderFn, String> + Send + Sync + 'static,
    {
        self.factories.insert(provider_type.to_string(), Arc::new(factory));
    }

    // 创建一个Provider闭包
    pub fn create_provider(&self, provider_type: &str, config: &serde_json::Value) -> Result<ProviderFn, String> {
        self.factories
            .get(provider_type)
            .ok_or_else(|| format!("未知的Provider类型: {}", provider_type))?
            (config)
    }
}

// 处理器 - 存储和执行Provider闭包
pub struct FnProcessor {
    providers: Vec<ProviderFn>,
}

impl FnProcessor {
    pub fn new() -> Self {
        Self {
            providers: Vec::new(),
        }
    }

    pub fn add_provider(&mut self, provider: ProviderFn) {
        self.providers.push(provider);
    }

    pub fn process(&self, context: &mut dyn Context) -> Result<(), Vec<String>> {
        let mut errors = Vec::new();
        
        for provider in &self.providers {
            if let Err(e) = provider(context) {
                errors.push(e);
            }
        }
        
        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }
}

// 解析配置的函数
pub fn parse_fn_process_config(
    registry: &ProviderFnRegistry,
    configs: &[serde_json::Value],
    processor: &mut FnProcessor,
) -> Result<(), Vec<String>> {
    let mut errors = Vec::new();
    
    for config in configs {
        let provider_type = config["type"].as_str().unwrap_or("unknown");
        
        match registry.create_provider(provider_type, config) {
            Ok(provider) => processor.add_provider(provider),
            Err(e) => errors.push(e),
        }
    }
    
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

// 全局注册表单例
lazy_static::lazy_static! {
    static ref GLOBAL_REGISTRY: std::sync::Mutex<ProviderFnRegistry> = std::sync::Mutex::new(ProviderFnRegistry::new());
}

// 全局函数用于注册工厂
pub fn register_provider_factory<F>(provider_type: &str, factory: F)
where
    F: Fn(&serde_json::Value) -> Result<ProviderFn, String> + Send + Sync + 'static,
{
    let mut registry = GLOBAL_REGISTRY.lock().unwrap();
    registry.register(provider_type, factory);
}

// 全局函数用于创建Provider
pub fn create_provider_by_config(provider_type: &str, config: &serde_json::Value) -> Result<ProviderFn, String> {
    let registry = GLOBAL_REGISTRY.lock().unwrap();
    registry.create_provider(provider_type, config)
}

#[cfg(test)]
mod test {
    use super::*;
    use serde_json::json;

    // 定义一个具体的上下文
    #[derive(Debug)]
    struct RequestContext {
        request_id: String,
        data: HashMap<String, String>,
    }

    impl Context for RequestContext {}

    #[test]
    fn test_provider() {
        // 注册日志Provider工厂
        register_provider_factory("log", |config| {
            // 从配置中提取参数
            let level = config["level"].as_str().unwrap_or("INFO").to_string();
            
            // 创建并返回Provider闭包
            let provider: ProviderFn = Arc::new(move |ctx| {
                println!("[{}] 处理请求: {:?}", level, ctx);
                Ok(())
            });
            
            Ok(provider)
        });
        
        // 注册数据库Provider工厂
        register_provider_factory("database", |config| {
            let conn_str = config["connection_string"].as_str()
                .ok_or_else(|| "缺少数据库连接字符串".to_string())?
                .to_string();
            
            let pool_size = config["pool_size"].as_u64().unwrap_or(10) as usize;
            
            // 创建并返回Provider闭包
            let provider: ProviderFn = Arc::new(move |ctx| {
                println!("数据库处理 (连接: {}, 池大小: {}): {:?}", conn_str, pool_size, ctx);
                Ok(())
            });
            
            Ok(provider)
        });
        
        // 创建处理器
        let mut processor = FnProcessor::new();
        
        // 创建配置
        let configs = vec![
            json!({
                "type": "log",
                "level": "DEBUG"
            }),
            json!({
                "type": "database",
                "connection_string": "postgres://user:pass@localhost/db",
                "pool_size": 5
            }),
        ];
        
        // 获取全局注册表
        let registry = GLOBAL_REGISTRY.lock().unwrap();
        
        // 解析配置并添加Provider
        parse_fn_process_config(&registry, &configs, &mut processor).unwrap();
        
        // 创建处理上下文
        let mut context = RequestContext {
            request_id: "req-123".to_string(),
            data: HashMap::new(),
        };
        
        // 处理请求
        processor.process(&mut context).unwrap();

    }
}