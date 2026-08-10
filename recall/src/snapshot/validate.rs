use crate::api::Feature;

use super::SnapshotError;

pub(super) fn features_equal(left: &[Feature], right: &[Feature]) -> bool {
    left.len() == right.len()
        && left
            .iter()
            .zip(right)
            .all(|(left, right)| match (left, right) {
                (
                    Feature::Categorical {
                        name: left_name,
                        value: left_value,
                    },
                    Feature::Categorical {
                        name: right_name,
                        value: right_value,
                    },
                ) => left_name == right_name && left_value == right_value,
                (
                    Feature::Numeric {
                        name: left_name,
                        value: left_value,
                    },
                    Feature::Numeric {
                        name: right_name,
                        value: right_value,
                    },
                ) => left_name == right_name && left_value.to_bits() == right_value.to_bits(),
                _ => false,
            })
}

pub(super) fn checked_sum<I>(values: I) -> Result<u64, SnapshotError>
where
    I: IntoIterator<Item = u64>,
{
    values.into_iter().try_fold(0_u64, |total, value| {
        total
            .checked_add(value)
            .ok_or(SnapshotError::Corrupt("count overflow"))
    })
}
