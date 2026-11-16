//! The agent's way to the browser's DevTools endpoint, one lease at a time
//! (compatibility TODO BC7). `browser.acquire` gives the agent the address
//! `…/browser/cdp/<lease_id>` of this service instead of the browser's own
//! port; its Playwright connects through it, and when the lease ends —
//! released, run out, taken by the user taking over, or given up when the
//! agent asks the user to — its connections are closed: what the agent was
//! doing stops at once, and no command of it reaches the browser any more.
//! This keeps the parties apart for the browser's sake; a process of the
//! agent's runtime that looks for the browser's port itself is not stopped.

use std::sync::{Arc, Mutex};

use futures::{SinkExt, StreamExt};
use tokio::sync::watch;
use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::WebSocketStream;

/// The connections of the leases that have them.
#[derive(Default)]
pub struct Proxies {
    open: Mutex<Vec<(String, watch::Sender<bool>)>>,
}

impl Proxies {
    /// A connection of `lease_id`: closed when `cut` says so.
    fn add(&self, lease_id: &str) -> watch::Receiver<bool> {
        let (cut, until) = watch::channel(false);
        let mut open = self.open.lock().expect("proxies");
        open.retain(|(_, cut)| !cut.is_closed());
        open.push((lease_id.to_string(), cut));
        until
    }

    /// Close the connections of the leases `valid` says are over. Returns how many were closed.
    pub fn cut(&self, valid: impl Fn(&str) -> bool) -> usize {
        let mut open = self.open.lock().expect("proxies");
        let before = open.len();
        open.retain(|(lease_id, cut)| {
            if cut.is_closed() {
                return false;
            }
            if valid(lease_id) {
                return true;
            }
            let _ = cut.send(true);
            false
        });
        before - open.len()
    }

    /// Connections that are open (tests).
    pub fn count(&self) -> usize {
        let mut open = self.open.lock().expect("proxies");
        open.retain(|(_, cut)| !cut.is_closed());
        open.len()
    }
}

/// `GET …/cdp/<lease_id>/json/version`: what Playwright reads before it
/// connects, pointing at this service's WebSocket of the lease.
pub fn version(base: &str, lease_id: &str, browser_version: Option<&str>) -> serde_json::Value {
    let ws = base.replacen("http://", "ws://", 1);
    serde_json::json!({
        "Browser": format!("Chrome/{}", browser_version.unwrap_or_default()),
        "Protocol-Version": "1.3",
        "webSocketDebuggerUrl": format!("{ws}/browser/cdp/{lease_id}/devtools/browser"),
    })
}

/// Carry the messages between the agent and the browser until either side
/// closes or the lease ends.
pub async fn carry<S>(proxies: Arc<Proxies>, lease_id: String, agent: WebSocketStream<S>, browser_url: String)
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send,
{
    let mut config = WebSocketConfig::default();
    config.max_message_size = None;
    config.max_frame_size = None;
    let browser = match tokio_tungstenite::connect_async_with_config(&browser_url, Some(config), true).await {
        Ok((browser, _)) => browser,
        Err(e) => {
            log::warn!("browser: the agent's connection of lease {lease_id} could not reach the browser: {e}");
            return;
        }
    };
    let mut cut = proxies.add(&lease_id);
    let (mut to_agent, mut from_agent) = agent.split();
    let (mut to_browser, mut from_browser) = browser.split();
    loop {
        tokio::select! {
            message = from_agent.next() => match message {
                Some(Ok(message @ (Message::Text(_) | Message::Binary(_)))) => {
                    if to_browser.send(message).await.is_err() {
                        break;
                    }
                }
                Some(Ok(Message::Close(_))) | Some(Err(_)) | None => break,
                Some(Ok(_)) => {}
            },
            message = from_browser.next() => match message {
                Some(Ok(message @ (Message::Text(_) | Message::Binary(_)))) => {
                    if to_agent.send(message).await.is_err() {
                        break;
                    }
                }
                Some(Ok(Message::Close(_))) | Some(Err(_)) | None => break,
                Some(Ok(_)) => {}
            },
            ended = cut.changed() => {
                if ended.is_ok() && !*cut.borrow_and_update() {
                    continue;
                }
                log::info!("browser: the lease {lease_id} ended; its connection to the browser is closed");
                break;
            }
        }
    }
    let _ = to_agent.close().await;
    let _ = to_browser.close().await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio_tungstenite::tungstenite::protocol::Role;

    /// A browser that answers every message with it, reversed.
    async fn browser() -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                tokio::spawn(async move {
                    let mut ws = tokio_tungstenite::accept_async(stream).await.unwrap();
                    while let Some(Ok(Message::Text(text))) = ws.next().await {
                        let back: String = text.as_str().chars().rev().collect();
                        if ws.send(Message::text(back)).await.is_err() {
                            break;
                        }
                    }
                });
            }
        });
        format!("ws://127.0.0.1:{port}/devtools/browser/x")
    }

    #[tokio::test]
    async fn the_agent_reaches_the_browser_until_its_lease_ends() {
        let proxies = Arc::new(Proxies::default());
        let (near, far) = tokio::io::duplex(1 << 16);
        let far = WebSocketStream::from_raw_socket(far, Role::Server, None).await;
        let carrying = tokio::spawn(carry(proxies.clone(), "lease-1".into(), far, browser().await));
        let mut agent = WebSocketStream::from_raw_socket(near, Role::Client, None).await;
        agent.send(Message::text("{\"id\":1}")).await.unwrap();
        assert_eq!(agent.next().await.unwrap().unwrap(), Message::text("}1:\"di\"{"));
        // The lease ends: the connection closes, whatever the agent was doing.
        let started = std::time::Instant::now();
        while proxies.count() == 0 {
            assert!(started.elapsed() < std::time::Duration::from_secs(5));
            tokio::task::yield_now().await;
        }
        assert_eq!(proxies.cut(|_| false), 1);
        tokio::time::timeout(std::time::Duration::from_secs(5), carrying).await.unwrap().unwrap();
        let closed = tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                match agent.next().await {
                    Some(Ok(Message::Close(_))) | None | Some(Err(_)) => return true,
                    Some(Ok(_)) => {}
                }
            }
        })
        .await
        .unwrap();
        assert!(closed);
    }

    #[test]
    fn connections_of_leases_that_ended_are_cut() {
        let proxies = Proxies::default();
        let (a, b) = (proxies.add("lease-a"), proxies.add("lease-b"));
        assert_eq!(proxies.count(), 2);
        assert_eq!(proxies.cut(|lease| lease == "lease-a"), 1);
        assert!(!*a.borrow() && *b.borrow());
        assert_eq!(proxies.count(), 1);
        drop(a);
        assert_eq!(proxies.count(), 0, "a connection that closed itself is forgotten");
        let v = version("http://127.0.0.1:4060/kapi/jarvis", "lease-1", Some("159.0.8154.6"));
        assert_eq!(v["webSocketDebuggerUrl"], "ws://127.0.0.1:4060/kapi/jarvis/browser/cdp/lease-1/devtools/browser");
        assert_eq!(v["Browser"], "Chrome/159.0.8154.6");
    }
}
