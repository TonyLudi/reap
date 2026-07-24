use reap_pm_live::PmProductRun;
use reap_pm_strategy::PmQuoteModel;

fn escape<M: PmQuoteModel>(run: &mut PmProductRun<M>) {
    let _ = run.public_capture_mut();
}

fn main() {}
