use reap_pm_live::{PmProduct, PmProductRun};
use reap_pm_strategy::{PmQuoteModel, PmQuoteModelRequirements};

fn escalate_product<M: PmQuoteModelRequirements>(product: &PmProduct<M>) {
    let _ = product.signer();
    let _ = product.authenticated_http_session();
    let _ = product.authenticated_ws_session();
    let _ = product.request_executor();
    let _ = product.mutation_owner();
    let _ = product.quote_mutation_request();
    let _ = product.cancel_mutation_request();
}

fn escalate_run<M: PmQuoteModel>(run: &PmProductRun<M>) {
    let _ = run.signer();
    let _ = run.authenticated_http_session();
    let _ = run.authenticated_ws_session();
    let _ = run.request_executor();
    let _ = run.mutation_owner();
    let _ = run.quote_mutation_request();
    let _ = run.cancel_mutation_request();
}

fn main() {}
