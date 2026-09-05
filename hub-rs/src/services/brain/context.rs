use std::any::Any;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// 插件接口 (Everything is a plugin)
pub trait Plugin: Send + Sync {
    fn name(&self) -> &'static str;
    fn install(&self, ctx: &mut Context);
}

/// 简化的微型 IoC 容器，吸收 Cordis 核心思想
#[derive(Default, Clone)]
pub struct Context {
    // 用于存放各种 Service 提供者 (Capability Providers)
    services: Arc<Mutex<HashMap<String, Arc<dyn Any + Send + Sync>>>>,
}

impl Context {
    pub fn new() -> Self {
        Self::default()
    }

    /// 注册一个服务提供者
    pub fn provide<T: 'static + Send + Sync>(&self, name: &str, service: Arc<T>) {
        let mut lock = self.services.lock().unwrap();
        lock.insert(name.to_string(), service);
    }

    /// 解析一个服务消费者
    pub fn resolve<T: 'static + Send + Sync>(&self, name: &str) -> Option<Arc<T>> {
        let lock = self.services.lock().unwrap();
        if let Some(any_service) = lock.get(name) {
            any_service.clone().downcast::<T>().ok()
        } else {
            None
        }
    }

    /// 挂载插件
    pub fn install_plugin(&mut self, plugin: Box<dyn Plugin>) {
        eprintln!("[Cordis-Lite Context] Installing plugin: {}", plugin.name());
        plugin.install(self);
    }
}
