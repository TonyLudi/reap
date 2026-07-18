use reap_feed::PrivateLoginBootstrap;

fn main() {
    let _ = PrivateLoginBootstrap {
        payload: "raw-login-or-command".to_string(),
    };
}
