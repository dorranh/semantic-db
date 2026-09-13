use crate::{Connection, error};
use datafusion::{
    arrow::{
        array::RecordBatch,
        compute::{CastOptions, cast_with_options},
        datatypes::SchemaRef,
    },
    error::Result,
    execution::{TaskContext, memory_pool::MemoryConsumer},
    physical_plan::{
        SendableRecordBatchStream,
        metrics::{ExecutionPlanMetricsSet, MetricBuilder},
        stream::RecordBatchStreamAdapter,
    },
};
use semantic_runtime::{QueryContext, QueryOptions, ipc::IpcDecoder};
use std::sync::{Arc, atomic::Ordering};

pub(super) async fn schema(connection: &Connection, sql: &str) -> Result<SchemaRef> {
    let query = QueryContext::new(QueryOptions {
        timeout_seconds: connection.config.query_timeout.as_secs().max(1),
        ..Default::default()
    })?;
    let id = format!("{}-schema", query.id);
    query
        .run(async {
            let _permit = connection
                .permits
                .acquire()
                .await
                .map_err(|_| error("connection closed"))?;
            let mut response =
                crate::http::request(connection, sql, "ArrowStream", &query, &id).await?;
            let limit = connection.config.max_response_bytes.min(4 * 1024 * 1024);
            let mut decoder = IpcDecoder::new(limit);
            let mut received = 0usize;
            while let Some(bytes) = response
                .chunk()
                .await
                .map_err(|_| error("schema request failed"))?
            {
                received = received.saturating_add(bytes.len());
                if received > limit {
                    return Err(error("schema response byte budget exhausted"));
                }
                decoder.push(&bytes)?;
                while let Some(batch) =
                    decoder.next_batch_admitted(|size, _| query.charge_decoded(size))?
                {
                    if batch.num_rows() != 0 {
                        return Err(error("schema request returned rows"));
                    }
                }
            }
            decoder.finish()
        })
        .await
}

pub(super) fn execute(
    connection: Arc<Connection>,
    sql: String,
    schema: SchemaRef,
) -> SendableRecordBatchStream {
    execute_context(
        connection,
        sql,
        schema,
        Arc::new(TaskContext::default()),
        false,
        ExecutionPlanMetricsSet::new(),
        vec![],
    )
}

pub(super) fn execute_context(
    connection: Arc<Connection>,
    sql: String,
    schema: SchemaRef,
    task: Arc<TaskContext>,
    zero_columns: bool,
    metrics: ExecutionPlanMetricsSet,
    filters: Vec<Arc<dyn datafusion::physical_expr::PhysicalExpr>>,
) -> SendableRecordBatchStream {
    let expected = schema.clone();
    let stream = async_stream::try_stream! {
        let options = QueryOptions { timeout_seconds: connection.config.query_timeout.as_secs().max(1), max_remote_bytes: connection.config.max_response_bytes, max_decoded_bytes: connection.config.max_decoded_bytes, ..Default::default() };
        let query = QueryContext::from_task(&task).map(Ok).unwrap_or_else(|| QueryContext::new(options))?;
        let id = format!("{}-{}", query.id, semantic_runtime::unique_id());
        let deadline = (tokio::time::Instant::now() + connection.config.query_timeout).min(query.deadline());
        let _permit = query.run(async { tokio::time::timeout_at(deadline, connection.permits.acquire()).await.map_err(|_| error("remote query timed out"))?.map_err(|_| error("connection closed")) }).await?;
        connection.counters.queries.fetch_add(1, Ordering::Relaxed);
        let started = std::time::Instant::now();
        let request_count = MetricBuilder::new(&metrics).counter("remote_queries", 0);
        let output_rows = MetricBuilder::new(&metrics).output_rows(0);
        let output_bytes = MetricBuilder::new(&metrics).counter("remote_bytes", 0);
        let decoded_bytes = MetricBuilder::new(&metrics).counter("decoded_bytes", 0);
        let first_batch = MetricBuilder::new(&metrics).counter("first_batch_ns", 0);
        let elapsed = MetricBuilder::new(&metrics).elapsed_compute(0);
        let _timer = elapsed.timer();
        request_count.add(1);
        let (sql,applied) = crate::runtime_filter::apply(sql,&filters,&expected,&connection);
        MetricBuilder::new(&metrics).counter("runtime_filters_applied",0).add(applied);
        MetricBuilder::new(&metrics).counter("runtime_filters_skipped",0).add(filters.len().saturating_sub(applied));
        let mut response = query.run(async { tokio::time::timeout_at(deadline, crate::http::request(&connection, &sql, "ArrowStream", &query, &id)).await.map_err(|_| error("remote query timed out"))? }).await?;
        let mut decoder = IpcDecoder::new(connection.config.max_decoded_bytes.min(query.options.max_decoded_bytes));
        let reservation = MemoryConsumer::new(format!("ClickHouse {id}")).register(task.memory_pool());
        let mut received = 0usize;
        let mut first = true;
        loop {
            let chunk = query.run(async {
                tokio::time::timeout_at(deadline, response.chunk()).await.map_err(|_| error("remote query timed out"))?.map_err(|_| error(&format!("remote query failed while reading results; query_id={id}")))
            }).await?;
            let Some(bytes) = chunk else { break; };
            received = received.saturating_add(bytes.len());
            query.charge_remote(bytes.len())?;
            connection.counters.bytes.fetch_add(bytes.len() as u64, Ordering::Relaxed);
            output_bytes.add(bytes.len());
            if received > connection.config.max_response_bytes { Err(error("response byte budget exhausted"))?; }
            reservation.try_resize(decoder.buffered_bytes().saturating_add(bytes.len()).saturating_mul(2).saturating_add(decoder.retained_bytes().saturating_mul(2)))?;
            decoder.push(&bytes)?;
            while let Some(batch) = decoder.next_batch_admitted(|size,retained| {
                query.charge_decoded(size)?; decoded_bytes.add(size);
                reservation.try_resize(size.saturating_mul(4).saturating_add(retained.saturating_mul(2)).saturating_add(semantic_runtime::ipc::DECODER_SCRATCH_BYTES))
            })? {
                query.check()?;
                if tokio::time::Instant::now() >= deadline { Err(error("remote query timed out"))?; }
                let batch = if zero_columns {
                    RecordBatch::try_new_with_options(expected.clone(), vec![], &datafusion::arrow::array::RecordBatchOptions::new().with_row_count(Some(batch.num_rows())))?
                } else { normalize(batch, &expected)? };
                if first { first_batch.add(started.elapsed().as_nanos().min(usize::MAX as u128) as usize); first = false; }
                connection.counters.rows.fetch_add(batch.num_rows() as u64, Ordering::Relaxed);
                output_rows.add(batch.num_rows());
                yield batch;
            }
        }
        decoder.finish()?;
    };
    Box::pin(RecordBatchStreamAdapter::new(schema, stream))
}

fn normalize(batch: RecordBatch, expected: &SchemaRef) -> Result<RecordBatch> {
    if batch.num_columns() != expected.fields().len() {
        return Err(error("result column count changed"));
    }
    let columns = batch
        .columns()
        .iter()
        .zip(expected.fields())
        .map(|(column, field)| {
            use datafusion::arrow::datatypes::DataType;
            let source = column.data_type();
            let target = field.data_type();
            let string_type = |t: &DataType| {
                matches!(t, DataType::Utf8 | DataType::LargeUtf8 | DataType::Utf8View)
            };
            // ClickHouse comparison expressions are UInt8, even when the logical
            // result is Boolean. Accept only the exact Boolean representation.
            let boolean = target == &DataType::Boolean
                && column
                    .as_any()
                    .downcast_ref::<datafusion::arrow::array::UInt8Array>()
                    .is_some_and(|values| values.iter().flatten().all(|v| v <= 1));
            let dictionary_string =
                matches!(source,DataType::Dictionary(_,value) if string_type(value));
            if source != target
                && !(source.is_numeric() && target.is_numeric()
                    || string_type(source) && string_type(target)
                    || dictionary_string && string_type(target)
                    || boolean)
            {
                return Err(error("result type changed; refresh the source binding"));
            }
            let converted = cast_with_options(
                column,
                field.data_type(),
                &CastOptions {
                    safe: false,
                    ..Default::default()
                },
            )
            .map_err(|_| error("result type conversion failed"))?;
            if !field.is_nullable() && converted.null_count() != 0 {
                return Err(error("unexpected NULL in non-nullable result"));
            }
            Ok(converted)
        })
        .collect::<Result<Vec<_>>>()?;
    RecordBatch::try_new(expected.clone(), columns).map_err(|_| error("result schema mismatch"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use arrow_ipc::writer::{IpcWriteOptions, StreamWriter};
    use datafusion::arrow::{
        array::{Int64Array, StringArray},
        datatypes::{DataType, Field, Schema},
    };
    fn fixture(codec: Option<arrow_ipc::CompressionType>) -> Vec<u8> {
        let schema = Arc::new(Schema::new(vec![Field::new("text", DataType::Utf8, false)]));
        let batch = RecordBatch::try_new(
            schema.clone(),
            vec![Arc::new(StringArray::from(vec!["x".repeat(1024); 100]))],
        )
        .unwrap();
        let mut bytes = vec![];
        let mut writer = StreamWriter::try_new_with_options(
            &mut bytes,
            &schema,
            IpcWriteOptions::default()
                .try_with_compression(codec)
                .unwrap(),
        )
        .unwrap();
        writer.write(&batch).unwrap();
        writer.write(&batch).unwrap();
        writer.finish().unwrap();
        bytes
    }
    #[test]
    fn decoding_is_bounded_and_handles_split_frames() {
        for codec in [
            None,
            Some(arrow_ipc::CompressionType::LZ4_FRAME),
            Some(arrow_ipc::CompressionType::ZSTD),
        ] {
            let bytes = fixture(codec);
            for chunk_size in [1, 127, bytes.len()] {
                let mut decoder = IpcDecoder::new(1024 * 1024);
                let mut rows = 0;
                for chunk in bytes.chunks(chunk_size) {
                    decoder.push(chunk).unwrap();
                    while let Some(batch) = decoder.next_batch().unwrap() {
                        rows += batch.num_rows();
                    }
                }
                decoder.finish().unwrap();
                assert_eq!(rows, 200);
            }
            let mut decoder = IpcDecoder::new(65536);
            assert!(
                decoder
                    .push(&bytes)
                    .and_then(|_| decoder.next_batch())
                    .is_err()
            );
            let mut decoder = IpcDecoder::new(1024 * 1024);
            decoder.push(&bytes[..bytes.len() - 20]).unwrap();
            while decoder.next_batch().unwrap().is_some() {}
            assert!(decoder.finish().is_err());
        }
    }
    #[test]
    fn comparison_bytes_normalize_only_exact_boolean_values() {
        use datafusion::arrow::array::{BooleanArray, UInt8Array};
        let schema = Arc::new(Schema::new(vec![Field::new(
            "predicate",
            DataType::UInt8,
            true,
        )]));
        let expected = Arc::new(Schema::new(vec![Field::new(
            "predicate",
            DataType::Boolean,
            true,
        )]));
        let batch = RecordBatch::try_new(
            schema.clone(),
            vec![Arc::new(UInt8Array::from(vec![Some(0), Some(1), None]))],
        )
        .unwrap();
        let result = normalize(batch, &expected).unwrap();
        assert_eq!(
            result
                .column(0)
                .as_any()
                .downcast_ref::<BooleanArray>()
                .unwrap(),
            &BooleanArray::from(vec![Some(false), Some(true), None])
        );
        let invalid =
            RecordBatch::try_new(schema, vec![Arc::new(UInt8Array::from(vec![2]))]).unwrap();
        assert!(normalize(invalid, &expected).is_err());
    }
    #[test]
    fn normalization_rejects_integer_overflow() {
        let batch = RecordBatch::try_new(
            Arc::new(Schema::new(vec![Field::new("x", DataType::Int64, false)])),
            vec![Arc::new(Int64Array::from(vec![i64::MAX]))],
        )
        .unwrap();
        let expected = Arc::new(Schema::new(vec![Field::new("x", DataType::Int32, false)]));
        assert!(normalize(batch, &expected).is_err());
    }
}
