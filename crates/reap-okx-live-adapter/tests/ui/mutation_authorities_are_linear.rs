use reap_okx_live_adapter::{
    BoundRegularOrderGateway, LiveSafety, PrivateStateSessionFactory,
};

fn require_clone<T: Clone>() {}

fn main() {
    require_clone::<BoundRegularOrderGateway>();
    require_clone::<LiveSafety>();
    require_clone::<PrivateStateSessionFactory>();
}
