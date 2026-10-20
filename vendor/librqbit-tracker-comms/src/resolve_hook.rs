// RIPTOR PATCH: optional process-wide resolver (Riptor's VPN tunnel).

use std::net::SocketAddr;
use std::sync::{Arc, RwLock};

use futures::future::BoxFuture;

pub type Resolver =
    Arc<dyn Fn(String, u16) -> BoxFuture<'static, std::io::Result<Vec<SocketAddr>>> + Send + Sync>;

static RESOLVER: RwLock<Option<Resolver>> = RwLock::new(None);

pub fn set_resolver(r: Option<Resolver>) {
    *RESOLVER.write().unwrap_or_else(|e| e.into_inner()) = r;
}

pub async fn lookup(host: &str, port: u16) -> std::io::Result<Vec<SocketAddr>> {
    let hook = RESOLVER.read().unwrap_or_else(|e| e.into_inner()).clone();
    match hook {
        Some(r) => r(host.to_string(), port).await,
        None => Ok(tokio::net::lookup_host((host, port)).await?.collect()),
    }
}
