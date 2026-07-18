use reap_feed::{BootstrapFactory, SocketPlan};

fn require_clone<T: Clone>() {}

fn steal_signed_bootstrap(factory: BootstrapFactory, plan: &SocketPlan) {
    require_clone::<BootstrapFactory>();
    let _ = factory(plan);
    let _ = factory.generate(plan, "wss://ws.okx.com:8443/ws/v5/private");
    let _ = BootstrapFactory::bind_private_websocket(
        "wss://ws.okx.com:8443/ws/v5/private",
        |_| Ok(vec!["raw-login-or-command".to_string()]),
    );
}

fn main() {}
