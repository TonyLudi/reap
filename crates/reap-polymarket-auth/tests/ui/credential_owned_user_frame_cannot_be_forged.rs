use reap_polymarket_auth::CredentialOwnedUserFrame;
use reap_polymarket_wire::PmLiveUserFrame;

fn forge(frame: PmLiveUserFrame) -> CredentialOwnedUserFrame {
    CredentialOwnedUserFrame(frame)
}

fn main() {}
