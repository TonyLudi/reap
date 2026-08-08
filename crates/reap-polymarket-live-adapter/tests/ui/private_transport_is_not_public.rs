use reap_polymarket_live_adapter::private_http::{PmPrivateHttpTransport, PmPrivateRoute};

fn main() {
    let _ = std::mem::size_of::<PmPrivateHttpTransport>();
    let _ = std::mem::size_of::<PmPrivateRoute<'static>>();
}
