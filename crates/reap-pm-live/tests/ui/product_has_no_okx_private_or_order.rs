use reap_pm_live::PmProduct;

fn escalate<M>(product: &PmProduct<M>) {
    let _ = product.okx_private();
    let _ = product.okx_order();
    let _ = product.okx_account();
}

fn main() {}
