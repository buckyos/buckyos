use super::action::RuleAction;
use crate::error::*;
use boa_engine::JsArgs;
use boa_engine::JsString;
use boa_engine::{
    js_str, js_string, object::ObjectInitializer, property::Attribute, Context, JsResult, JsValue,
    NativeFunction, Source,
};
use boa_runtime::Console;
use regex::Regex;
use std::error::Error;
use std::sync::{Arc, Mutex};
use tokio::sync::Mutex as AsyncMutex;
use trust_dns_resolver::config::*;
use trust_dns_resolver::Resolver;

struct PacScriptExecutor {
    context: Arc<Mutex<Context>>,
}

impl PacScriptExecutor {
    pub fn new() -> RuleResult<Self> {
        let mut context = Context::default();

        // Register pac environment functions
        Self::register_env(&mut context)?;

        // Register console object
        let console = Console::init(&mut context);

        // Register the console as a global property to the context.
        context
            .register_global_property(js_string!(Console::NAME), console, Attribute::all())
            .expect("the console object shouldn't exist yet");

        let context = Arc::new(Mutex::new(context));

        Ok(Self { context })
    }

    // Evaluate the PAC script
    fn load(&self, src: &str) -> RuleResult<()> {
        let mut context = self.context.lock().unwrap();

        let src = Source::from_bytes(src.as_bytes());
        context.eval(src).map_err(|e| {
            let mut source = e.source();
            while let Some(err) = source {
                println!("Caused by: {:?}", err);
                source = err.source();
            }

            let msg = format!("failed to eval PAC script: {:?}, {:?}", e, e.source());
            error!("{}", msg);
            RuleError::InvalidScript(msg)
        })?;

        Ok(())
    }

    pub fn rule_select(&self, input: RuleInput) -> RuleResult<RuleOutput> {
        let mut context = self.context.lock().unwrap();

        // Call the RuleSelect(url, source_info) -> output function in the PAC script
        let func = context
            .global_object()
            .get(js_string!("RuleSelect"), &mut context)
            .map_err(|e| {
                let msg = format!("failed to get RuleSelect: {:?}", e);
                error!("{}", msg);
                RuleError::InvalidScript(msg)
            })?;

        // Check if the RuleSelect is a not none and a function
        if func.is_null_or_undefined() {
            let msg = format!("RuleSelect is not defined yet!");
            error!("{}", msg);
            return Err(RuleError::InvalidScript(msg));
        }

        // Prepare the dest info object
        let mut dest_builder = ObjectInitializer::new(&mut context);
        dest_builder.property(
            js_str!("url"),
            js_string!(input.dest.url.to_string()),
            Attribute::all(),
        );
        dest_builder.property(
            js_str!("host"),
            js_string!(input.dest.host),
            Attribute::all(),
        );
        dest_builder.property(
            js_str!("port"),
            JsValue::from(input.dest.port),
            Attribute::all(),
        );
        let dest_map = dest_builder.build();

        // Prepare the source info object
        let mut headers_builder = ObjectInitializer::new(&mut context);
        for (k, v) in input.source.http_headers.iter() {
            headers_builder.property(
                js_string!(k.clone()),
                js_string!(v.clone()),
                Attribute::all(),
            );
        }
        let headers_map = headers_builder.build();

        let mut source_builder = ObjectInitializer::new(&mut context);
        source_builder.property(js_str!("ip"), js_string!(input.source.ip), Attribute::all());
        source_builder.property(js_str!("http_headers"), headers_map, Attribute::all());
        source_builder.property(
            js_str!("protocol"),
            js_string!(input.source.protocol),
            Attribute::all(),
        );
        let source_map = source_builder.build();

        let select_func = func.as_callable().ok_or_else(|| {
            let msg = format!("RuleSelect is not a function");
            error!("{}", msg);
            RuleError::InvalidScript(msg)
        })?;

        let result = select_func
            .call(
                &JsValue::undefined(),
                &[JsValue::from(dest_map), JsValue::from(source_map)],
                &mut context,
            )
            .map_err(|e| {
                let msg = format!("failed to call RuleSelect: {:?}", e);
                error!("{}", msg);
                RuleError::InvalidFormat(msg)
            })?;

        // Parse the result
        let result = result.to_string(&mut context).map_err(|e| {
            let msg = format!("failed to convert result to string: {:?}", e);
            error!("{}", msg);
            RuleError::InvalidFormat(msg)
        })?;

        info!("RuleSelect result: {:?}", result);
        let actions = RuleAction::from_str_list(&result.to_std_string().unwrap())?;

        Ok(RuleOutput { actions })
    }

    fn register_env(context: &mut Context) -> RuleResult<()> {
        // Register the isPlainHostName function
        context
            .register_global_builtin_callable(
                js_string!("isPlainHostName"),
                1,
                NativeFunction::from_fn_ptr(PACEnvFunctions::api_is_plain_host_name),
            )
            .map_err(|e| {
                let msg = format!("failed to register isPlainHostName: {:?}", e);
                error!("{}", msg);
                RuleError::InvalidFormat(msg)
            })?;

        // Register the dnsDomainIs function
        context
            .register_global_builtin_callable(
                js_string!("dnsDomainIs"),
                2,
                NativeFunction::from_fn_ptr(PACEnvFunctions::api_dns_domain_is),
            )
            .map_err(|e| {
                let msg = format!("failed to register dnsDomainIs: {:?}", e);
                error!("{}", msg);
                RuleError::InvalidFormat(msg)
            })?;

        // Register the dnsDomainLevels function
        context
            .register_global_builtin_callable(
                js_string!("dnsDomainLevels"),
                1,
                NativeFunction::from_fn_ptr(PACEnvFunctions::api_dns_domain_levels),
            )
            .map_err(|e| {
                let msg = format!("failed to register dnsDomainLevels: {:?}", e);
                error!("{}", msg);
                RuleError::InvalidFormat(msg)
            })?;

        // Register the dnsResolve function
        context
            .register_global_builtin_callable(
                js_string!("dnsResolve"),
                1,
                NativeFunction::from_fn_ptr(PACEnvFunctions::api_dns_resolve),
            )
            .map_err(|e| {
                let msg = format!("failed to register dnsResolve: {:?}", e);
                error!("{}", msg);
                RuleError::InvalidFormat(msg)
            })?;

        // Register the isResolvable function
        context
            .register_global_builtin_callable(
                js_string!("isResolvable"),
                1,
                NativeFunction::from_fn_ptr(PACEnvFunctions::api_is_resolvable),
            )
            .map_err(|e| {
                let msg = format!("failed to register isResolvable: {:?}", e);
                error!("{}", msg);
                RuleError::InvalidFormat(msg)
            })?;

        // Register the localHostOrDomainIs function
        context
            .register_global_builtin_callable(
                js_string!("localHostOrDomainIs"),
                2,
                NativeFunction::from_fn_ptr(PACEnvFunctions::api_local_host_or_domain_is),
            )
            .map_err(|e| {
                let msg = format!("failed to register localHostOrDomainIs: {:?}", e);
                error!("{}", msg);
                RuleError::InvalidFormat(msg)
            })?;

        // Register the shExpMatch function
        context
            .register_global_builtin_callable(
                js_string!("shExpMatch"),
                2,
                NativeFunction::from_fn_ptr(PACEnvFunctions::api_sh_exp_match),
            )
            .map_err(|e| {
                let msg = format!("failed to register shExpMatch: {:?}", e);
                error!("{}", msg);
                RuleError::InvalidFormat(msg)
            })?;

        Ok(())
    }
}

use super::selector::*;

pub struct PacScriptManager {
    script: String,
}

impl PacScriptManager {
    pub fn new(script: String) -> RuleResult<Self> {
        // Check if the script is valid at the first
        Self::check_valid(&script)?;

        let ret = Self { script };

        Ok(ret)
    }

    pub fn check_valid(script: &str) -> RuleResult<()> {
        let executor = PacScriptExecutor::new()?;

        let start = chrono::Utc::now();
        executor.load(script)?;
        let end = chrono::Utc::now();
        let duration = end - start;
        info!("PAC script loaded in {:?} ms", duration.num_milliseconds());

        Ok(())
    }
}

#[async_trait::async_trait]
impl RuleSelector for PacScriptManager {
    async fn select(&self, input: RuleInput) -> RuleResult<RuleOutput> {
        // info!("Begin select pac rule for: {:?}", input);

        let start = chrono::Utc::now();
        let executor = PacScriptExecutor::new()?;
        executor.load(&self.script)?;

        let end = chrono::Utc::now();
        let duration = end - start;
        info!("PAC script loaded in {:?} ms", duration.num_milliseconds());

        // info!("PAC script loaded");

        let ret = executor.rule_select(input)?;

        let end = chrono::Utc::now();
        let duration = end - start;
        info!("PAC script select in {:?} ms", duration.num_milliseconds());

        Ok(ret)
    }
}

use tokio::sync::{mpsc, oneshot};

struct AsyncRuleSelectRequest {
    input: RuleInput,
    response_tx: oneshot::Sender<RuleResult<RuleOutput>>,
}

pub struct AsyncPacScriptManager {
    tx: AsyncMutex<mpsc::Sender<AsyncRuleSelectRequest>>,
}

impl AsyncPacScriptManager {
    pub fn new(script: String) -> RuleResult<Self> {
        PacScriptManager::check_valid(&script)?;

        let (tx, rx) = mpsc::channel(1024);
        let ret = Self {
            tx: AsyncMutex::new(tx),
        };

        // Start a new thread to run the script
        let _thread = std::thread::spawn(move || {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(async {
                    match Self::run(script, rx).await {
                        Ok(_) => {}
                        Err(e) => {
                            error!("Failed to run PAC script: {:?}", e);
                        }
                    }
                });
        });
    
        Ok(ret)
    }

    async fn run(
        script: String,
        mut rx: mpsc::Receiver<AsyncRuleSelectRequest>,
    ) -> RuleResult<()> {
        let executor = PacScriptExecutor::new()?;
        executor.load(&script)?;

        while let Some(req) = rx.recv().await {
            let ret = executor.rule_select(req.input);
            if let Err(e) = req.response_tx.send(ret) {
                error!("Failed to send rule select result: {:?}", e);
                break; // stop the loop
            }
        }

        Ok(())
    }
}

#[async_trait::async_trait]
impl RuleSelector for AsyncPacScriptManager {
    async fn select(&self, input: RuleInput) -> RuleResult<RuleOutput> {
        let (response_tx, response_rx) = oneshot::channel();

        let req = AsyncRuleSelectRequest { input, response_tx };
        let tx = self.tx.lock().await;
        if let Err(e) = tx.send(req).await {
            return Err(RuleError::InternalError(format!(
                "Failed to send rule input: {:?}",
                e
            )));
        }

        let ret = response_rx.await.map_err(|e| {
            RuleError::InternalError(format!("Failed to receive rule output: {:?}", e))
        })?;
        ret
    }
}

// PACEnvFunctions is a struct that provides the environment functions
// for the PAC script.
struct PACEnvFunctions {}

impl PACEnvFunctions {
    pub fn api_is_plain_host_name(
        _this: &JsValue,
        args: &[JsValue],
        ctx: &mut Context,
    ) -> JsResult<JsValue> {
        // Get the host argument
        let host = args.get_or_undefined(0).to_string(ctx)?;
        let is_plain = Self::is_plain_host_name(&host.to_std_string().unwrap());

        // Return the result
        Ok(JsValue::from(!is_plain))
    }

    // Check if the host is a plain host name (no dots)
    fn is_plain_host_name(host: &str) -> bool {
        !host.contains('.')
    }

    pub fn api_dns_domain_is(
        _this: &JsValue,
        args: &[JsValue],
        ctx: &mut Context,
    ) -> JsResult<JsValue> {
        // Get the host and domain arguments
        let host = args.get_or_undefined(0).to_string(ctx)?;
        let domain = args.get_or_undefined(1).to_string(ctx)?;

        let is_domain = Self::dns_domain_is(
            &host.to_std_string().unwrap(),
            &domain.to_std_string().unwrap(),
        );

        // Return the result
        Ok(JsValue::from(is_domain))
    }

    // Check if the host is in the domain
    fn dns_domain_is(host: &str, domain: &str) -> bool {
        host.ends_with(domain)
    }

    pub fn api_dns_domain_levels(
        _this: &JsValue,
        args: &[JsValue],
        ctx: &mut Context,
    ) -> JsResult<JsValue> {
        // Get the host argument
        let host = args.get_or_undefined(0).to_string(ctx)?;
        let levels = Self::dns_domain_levels(&host.to_std_string().unwrap());

        // Return the result
        Ok(JsValue::from(levels))
    }

    fn dns_domain_levels(host: &str) -> i32 {
        // Get the domain levels
        host.split('.').count() as i32
    }

    pub fn api_dns_resolve(
        _this: &JsValue,
        args: &[JsValue],
        ctx: &mut Context,
    ) -> JsResult<JsValue> {
        // Get the host argument
        let host = args.get_or_undefined(0).to_string(ctx)?;

        let ip = Self::dns_resolve(&host.to_std_string().unwrap());

        match ip {
            None => Ok(JsValue::undefined()),
            Some(ip) => Ok(JsValue::new(JsString::from(ip))),
        }
    }

    fn dns_resolve(host: &str) -> Option<String> {
        let resolver = Resolver::new(ResolverConfig::default(), ResolverOpts::default()).unwrap();
        match resolver.lookup_ip(host) {
            Ok(response) => response.iter().next().map(|ip| ip.to_string()),
            Err(e) => {
                warn!("failed to resolve host: {:?}", e);
                None
            }
        }
    }

    pub fn api_is_resolvable(
        _this: &JsValue,
        args: &[JsValue],
        ctx: &mut Context,
    ) -> JsResult<JsValue> {
        // Get the host argument
        let host = args.get_or_undefined(0).to_string(ctx)?;

        let is_resolvable = Self::is_resolvable(&host.to_std_string().unwrap());

        // Return the result
        Ok(JsValue::from(is_resolvable))
    }

    fn is_resolvable(host: &str) -> bool {
        Self::dns_resolve(host).is_some()
    }

    pub fn api_local_host_or_domain_is(
        _this: &JsValue,
        args: &[JsValue],
        ctx: &mut Context,
    ) -> JsResult<JsValue> {
        // Get the host and hostdom arguments
        let host = args.get_or_undefined(0).to_string(ctx)?;
        let hostdom = args.get_or_undefined(1).to_string(ctx)?;

        let is_local = Self::local_host_or_domain_is(
            &host.to_std_string().unwrap(),
            &hostdom.to_std_string().unwrap(),
        );

        // Return the result
        Ok(JsValue::from(is_local))
    }

    fn local_host_or_domain_is(host: &str, hostdom: &str) -> bool {
        let parts: Vec<&str> = host.split('.').collect();
        let domparts: Vec<&str> = hostdom.split('.').collect();

        for (part, dompart) in parts.iter().zip(domparts.iter()) {
            if part != dompart {
                return false;
            }
        }

        true
    }

    pub fn api_sh_exp_match(
        _this: &JsValue,
        args: &[JsValue],
        ctx: &mut Context,
    ) -> JsResult<JsValue> {
        // Get the string and shexp arguments
        let s = args.get_or_undefined(0).to_string(ctx)?;
        let sh_exp = args.get_or_undefined(1).to_string(ctx)?;

        let matched = Self::sh_exp_match(
            &s.to_std_string().unwrap(),
            &sh_exp.to_std_string().unwrap(),
        );

        // Return the result
        Ok(JsValue::from(matched))
    }

    fn sh_exp_match(s: &str, sh_exp: &str) -> bool {
        let re = Self::to_regex(sh_exp);
        re.is_match(s)
    }

    fn to_regex(s: &str) -> Regex {
        let s = s.replace(".", "\\.").replace("?", ".").replace("*", ".*");
        Regex::new(&("^".to_owned() + &s)).unwrap()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_pac_script_manager() {
        buckyos_kit::init_logging("test_pac_script_manager");

        let src = r#"
            Math.abs(-1);
            console.info("PAC script loaded");

            function RuleSelect(dest, source) {
                console.info(`RuleSelect called: ${dest.url}, ${source.ip}, ${JSON.stringify(source)}`);
                console.info("RuleSelect called");
                return "PROXY 127.0.0.1:8080; DIRECT";
            }
        "#;

        let manager = PacScriptExecutor::new().unwrap();
        match manager.load(src) {
            Ok(_) => {}
            Err(e) => {
                panic!("failed to load PAC script: {:?}", e);
            }
        }

        // Test rule_select method
        let input = RuleInput {
            source: RequestSourceInfo {
                ip: "127.0.0.1".to_owned(),
                http_headers: vec![("User-Agent".to_string(), "Mozilla".to_string())],
                protocol: "http".to_string(),
            },
            dest: RequestDestInfo {
                url: url::Url::parse("http://www.google.com").unwrap(),
                host: "www.google.com".to_string(),
                port: 80,
            },
        };

        let ret = manager.rule_select(input).unwrap();
        info!("rule_select result: {:?}", ret);
    }
}
