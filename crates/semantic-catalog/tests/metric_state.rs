use semantic_catalog::{
    ExactDistinctState, ExactMean, METRIC_STATE_VERSION, MergeMode, MetricStateContract,
    MetricStateError, MetricStateKind, PopulationRelation, SnapshotBalanceState, SumCountState,
    WeightedState, ZeroWeight,
};

fn contract(state: MetricStateKind) -> MetricStateContract {
    MetricStateContract {
        version: METRIC_STATE_VERSION,
        state,
        merge_dimensions: ["region".into()].into(),
    }
}

#[test]
fn average_merges_sufficient_state_instead_of_averaging_group_means() {
    let rule = contract(MetricStateKind::SumCountAverage);
    assert_eq!(
        rule.check_merge(
            &["region".into()].into(),
            PopulationRelation::ProvenDisjoint,
            false
        ),
        Ok(MergeMode::AddComponents)
    );
    assert_eq!(
        rule.check_merge(&Default::default(), PopulationRelation::MayOverlap, false),
        Err(MetricStateError::Overlap)
    );
    let small = SumCountState { sum: 10, count: 1 };
    let large = SumCountState { sum: 60, count: 3 };
    assert_eq!(
        small.merge(large).unwrap().finalize().unwrap(),
        Some(ExactMean(17_500_000_000_000_000_000))
    );
    assert_eq!(
        large.merge(small).unwrap().finalize().unwrap(),
        Some(ExactMean(17_500_000_000_000_000_000))
    );
    assert_eq!(SumCountState { sum: 0, count: 0 }.finalize().unwrap(), None);
    assert_eq!(
        SumCountState {
            sum: i128::MAX,
            count: 1
        }
        .merge(small),
        Err(MetricStateError::Overflow)
    );
    assert_eq!(
        SumCountState {
            sum: i128::MAX,
            count: 1
        }
        .finalize(),
        Err(MetricStateError::Overflow)
    );
}

#[test]
fn weighted_average_uses_weighted_components_and_declared_zero_behavior() {
    let rule = contract(MetricStateKind::WeightedAverage {
        weight_field: "weight".into(),
        zero: ZeroWeight::Null,
    });
    assert_eq!(
        rule.check_merge(&Default::default(), PopulationRelation::Unknown, false),
        Err(MetricStateError::Overlap)
    );
    let merged = WeightedState {
        weighted_sum: 10,
        weight_sum: 1,
    }
    .merge(WeightedState {
        weighted_sum: 60,
        weight_sum: 3,
    })
    .unwrap();
    assert_eq!(
        merged.finalize(ZeroWeight::Null).unwrap(),
        Some(ExactMean(17_500_000_000_000_000_000))
    );
    assert_eq!(
        WeightedState {
            weighted_sum: 0,
            weight_sum: 0
        }
        .finalize(ZeroWeight::Null)
        .unwrap(),
        None
    );
    assert_eq!(
        WeightedState {
            weighted_sum: 0,
            weight_sum: 0
        }
        .finalize(ZeroWeight::Zero)
        .unwrap(),
        Some(ExactMean(0))
    );
}

#[test]
fn distinct_merges_exact_identities_even_when_populations_overlap() {
    let rule = contract(MetricStateKind::ExactDistinct {
        identity_fields: vec!["customer_id".into()],
    });
    assert_eq!(
        rule.check_merge(&Default::default(), PopulationRelation::MayOverlap, false),
        Ok(MergeMode::UnionIdentities)
    );
    let left = ExactDistinctState([b"a".to_vec(), b"b".to_vec()].into());
    let right = ExactDistinctState([b"b".to_vec(), b"c".to_vec()].into());
    assert_eq!(left.merge(right).finalize_count(), Ok(3));
    let mut duplicate = rule;
    duplicate.state = MetricStateKind::ExactDistinct {
        identity_fields: vec!["id".into(), "id".into()],
    };
    assert_eq!(duplicate.validate(), Err(MetricStateError::Identity));
}

#[test]
fn balance_selects_latest_and_rejects_time_sums_or_ambiguous_ties() {
    let rule = contract(MetricStateKind::SnapshotBalance {
        time_field: "as_of".into(),
        tie_break_fields: vec!["revision".into()],
    });
    assert_eq!(
        rule.check_merge(&Default::default(), PopulationRelation::MayOverlap, false),
        Ok(MergeMode::SelectLatest)
    );
    assert_eq!(
        rule.check_merge(
            &Default::default(),
            PopulationRelation::ProvenDisjoint,
            true
        ),
        Err(MetricStateError::TimeAdditivity)
    );
    let old = SnapshotBalanceState {
        observed_at: 1,
        tie_break: vec!["a".into()],
        value: Some(100),
    };
    let newest = SnapshotBalanceState {
        observed_at: 2,
        tie_break: vec!["a".into()],
        value: Some(90),
    };
    assert_eq!(
        old.clone().select_latest(newest.clone()),
        Ok(newest.clone())
    );
    assert_eq!(newest.clone().select_latest(old), Ok(newest.clone()));
    let conflicting = SnapshotBalanceState {
        value: Some(91),
        ..newest.clone()
    };
    assert_eq!(
        newest.select_latest(conflicting),
        Err(MetricStateError::ConflictingSnapshot)
    );
}

#[test]
fn versions_dimensions_and_identity_are_checked_before_merge() {
    let mut rule = contract(MetricStateKind::SumCountAverage);
    assert_eq!(
        rule.check_merge(
            &["product".into()].into(),
            PopulationRelation::ProvenDisjoint,
            false
        ),
        Err(MetricStateError::Dimension)
    );
    rule.version = METRIC_STATE_VERSION + 1;
    assert_eq!(rule.validate(), Err(MetricStateError::Version));
    rule.version = METRIC_STATE_VERSION;
    rule.merge_dimensions.insert("".into());
    assert_eq!(rule.validate(), Err(MetricStateError::Identity));
}
