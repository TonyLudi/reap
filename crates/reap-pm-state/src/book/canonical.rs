use std::cmp::Ordering;

use reap_pm_core::{
    MAX_PM_BOOK_LEVELS, PmBookLevel, PmBookQuantity, PmBookSide, PmMarketMetadata, PmPrice,
};

use crate::readiness::PmPublicReadinessReason;

const MAX_LEVELS: usize = MAX_PM_BOOK_LEVELS as usize;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct BookKey {
    pub(super) side: PmBookSide,
    pub(super) price: PmPrice,
}

pub(super) fn validate_and_sort_levels(
    levels: &mut [PmBookLevel],
    metadata: PmMarketMetadata,
    snapshot: bool,
) -> Result<(), PmPublicReadinessReason> {
    if levels.len() > MAX_LEVELS {
        return Err(PmPublicReadinessReason::TooManyLevels);
    }
    for level in levels.iter() {
        if snapshot && level.quantity() == PmBookQuantity::Delete {
            return Err(PmPublicReadinessReason::DeleteInSnapshot);
        }
        if level.quantity() == PmBookQuantity::Delete {
            return Err(PmPublicReadinessReason::InvalidTransition);
        }
        if level.price().validate_tick(metadata.tick()).is_err() {
            return Err(PmPublicReadinessReason::PriceOffTick);
        }
    }
    levels.sort_unstable_by(compare_levels);
    if levels.windows(2).any(|pair| same_key(pair[0], pair[1])) {
        return Err(PmPublicReadinessReason::DuplicateLevel);
    }
    canonical_top(levels).map(|_| ())
}

pub(super) fn apply_change(
    levels: &mut Vec<PmBookLevel>,
    change: PmBookLevel,
) -> Result<(), PmPublicReadinessReason> {
    let key = BookKey {
        side: change.side(),
        price: change.price(),
    };
    match levels.binary_search_by(|candidate| compare_level_to_key(*candidate, key)) {
        Ok(index) => {
            if change.quantity() == PmBookQuantity::Delete {
                levels.remove(index);
            } else {
                levels[index] = change;
            }
        }
        Err(index) => {
            if change.quantity() == PmBookQuantity::Delete {
                return Err(PmPublicReadinessReason::MissingDeleteLevel);
            }
            if levels.len() == MAX_LEVELS {
                return Err(PmPublicReadinessReason::TooManyLevels);
            }
            levels.insert(index, change);
        }
    }
    Ok(())
}

pub(super) fn canonical_top(
    levels: &[PmBookLevel],
) -> Result<(PmPrice, PmPrice), PmPublicReadinessReason> {
    let bid = levels
        .iter()
        .find(|level| level.side() == PmBookSide::Bid)
        .map(|level| level.price());
    let ask = levels
        .iter()
        .find(|level| level.side() == PmBookSide::Ask)
        .map(|level| level.price());
    let (Some(bid), Some(ask)) = (bid, ask) else {
        return Err(PmPublicReadinessReason::EmptyBook);
    };
    if bid >= ask {
        Err(PmPublicReadinessReason::CrossedBook)
    } else {
        Ok((bid, ask))
    }
}

fn compare_levels(left: &PmBookLevel, right: &PmBookLevel) -> Ordering {
    compare_keys(
        BookKey {
            side: left.side(),
            price: left.price(),
        },
        BookKey {
            side: right.side(),
            price: right.price(),
        },
    )
}

fn compare_level_to_key(level: PmBookLevel, key: BookKey) -> Ordering {
    compare_keys(
        BookKey {
            side: level.side(),
            price: level.price(),
        },
        key,
    )
}

pub(super) fn compare_keys(left: BookKey, right: BookKey) -> Ordering {
    match (left.side, right.side) {
        (PmBookSide::Bid, PmBookSide::Ask) => Ordering::Less,
        (PmBookSide::Ask, PmBookSide::Bid) => Ordering::Greater,
        (PmBookSide::Bid, PmBookSide::Bid) => right.price.cmp(&left.price),
        (PmBookSide::Ask, PmBookSide::Ask) => left.price.cmp(&right.price),
    }
}

fn same_key(left: PmBookLevel, right: PmBookLevel) -> bool {
    left.side() == right.side() && left.price() == right.price()
}
