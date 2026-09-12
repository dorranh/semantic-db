-- Synthetic measurements: drilled interval (m), duration (minutes), and load (kN).
-- Execute in an empty drilling database; the integration tests do this automatically.
CREATE TABLE wells (well_id Int64, basin String) ENGINE = MergeTree ORDER BY well_id;
INSERT INTO wells VALUES (1, 'North'), (2, 'South'), (3, 'Unmeasured');

CREATE TABLE drilling_samples (
    well_id Int64,
    sample_id Int64,
    drilled_m Float64,
    duration_min Float64,
    load_kn Nullable(Float64)
) ENGINE = MergeTree ORDER BY (well_id, sample_id);

CREATE TABLE drilling_totals (
    well_id Int64,
    drilled_m Float64,
    duration_min Float64
) ENGINE = SummingMergeTree ORDER BY well_id;

CREATE MATERIALIZED VIEW drilling_totals_mv TO drilling_totals AS
SELECT well_id, sum(drilled_m) AS drilled_m, sum(duration_min) AS duration_min
FROM drilling_samples GROUP BY well_id;

CREATE TABLE drilling_states (
    well_id Int64,
    distance_state AggregateFunction(sum, Float64),
    duration_state AggregateFunction(sum, Float64),
    load_state AggregateFunction(avg, Nullable(Float64)),
    samples_state AggregateFunction(count)
) ENGINE = AggregatingMergeTree ORDER BY well_id;

CREATE MATERIALIZED VIEW drilling_states_mv TO drilling_states AS
SELECT well_id, sumState(drilled_m) AS distance_state,
    sumState(duration_min) AS duration_state, avgState(load_kn) AS load_state,
    countState() AS samples_state
FROM drilling_samples GROUP BY well_id;

-- Aggregate states stay in ClickHouse. This view merges all matching states,
-- including states from parts which background compaction has not merged yet.
CREATE VIEW drilling_summary AS
SELECT well_id, sumMerge(distance_state) AS drilled_m,
    sumMerge(duration_state) AS duration_min, avgMerge(load_state) AS mean_load_kn,
    countMerge(samples_state) AS samples
FROM drilling_states GROUP BY well_id;

SYSTEM STOP MERGES drilling_totals;
SYSTEM STOP MERGES drilling_states;

-- Different batch sizes ensure averaging batch averages would be wrong:
-- well 1's average load is (10 + 20 + 90) / 3 = 40, not (15 + 90) / 2.
INSERT INTO drilling_samples VALUES (1, 1, 10, 2, 10), (1, 2, 20, 4, 20), (2, 1, 5, 1, 30);
INSERT INTO drilling_samples VALUES (1, 3, 30, 6, 90), (1, 4, 0, 1, NULL), (2, 2, 15, 3, NULL);
