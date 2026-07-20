use reap_polymarket_adapter::PmPublicRole;

fn cannot_mutate(role: PmPublicRole) {
    role.place();
    role.cancel_owned();
    let _ = role.execution();
}

fn main() {}
