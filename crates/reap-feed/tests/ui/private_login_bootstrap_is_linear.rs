use reap_feed::PrivateLoginBootstrap;

fn require_clone<T: Clone>() {}

fn main() {
    require_clone::<PrivateLoginBootstrap>();
}
