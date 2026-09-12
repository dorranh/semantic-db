use crate::{Connection, error};
use arrow_ipc::reader::StreamDecoder;
use datafusion::{
    arrow::{
        array::RecordBatch,
        buffer::Buffer,
        compute::{CastOptions, cast_with_options},
        datatypes::SchemaRef,
    },
    error::Result,
    physical_plan::{SendableRecordBatchStream, stream::RecordBatchStreamAdapter},
};
use std::sync::{Arc, atomic::Ordering};

/// One HTTP chunk can contain multiple Arrow messages; a message can also span
/// many chunks. Always retain and exhaust the decoder's remaining buffer.
#[derive(Default)]
struct Decoder(StreamDecoder);
impl Decoder {
    fn push(&mut self, mut bytes: Buffer) -> Result<Vec<RecordBatch>> {
        let mut batches = Vec::new();
        while !bytes.is_empty() {
            if let Some(batch) = self
                .0
                .decode(&mut bytes)
                .map_err(|_| error("invalid Arrow stream"))?
            {
                batches.push(batch);
            }
        }
        Ok(batches)
    }
    fn finish(&mut self) -> Result<SchemaRef> {
        self.0
            .finish()
            .map_err(|_| error("incomplete Arrow stream"))?;
        self.0
            .schema()
            .ok_or_else(|| error("Arrow stream has no schema"))
    }
}

pub(super) async fn schema(connection: &Connection, sql: &str) -> Result<SchemaRef> {
    tokio::time::timeout(connection.config.query_timeout, async {
        let mut cursor = connection.client.query(sql).fetch_bytes("ArrowStream")
            .map_err(|_| error("schema request failed; bind an ordinary table or a finalized aggregate view"))?;
        let mut decoder = Decoder::default();
        let mut received = 0usize;
        while let Some(bytes) = cursor.next().await.map_err(|_| error("schema request failed; check table access and finalize AggregateFunction columns in a view"))? {
            received = received.saturating_add(bytes.len());
            if received > connection.config.max_response_bytes.min(4 * 1024 * 1024) { return Err(error("schema response byte budget exhausted")); }
            if decoder.push(Buffer::from(bytes))?.iter().any(|b| b.num_rows() != 0) { return Err(error("schema request returned rows")); }
        }
        decoder.finish()
    }).await.map_err(|_| error("schema request timed out"))?
}

pub(super) fn execute(
    connection: Arc<Connection>,
    sql: String,
    schema: SchemaRef,
) -> SendableRecordBatchStream {
    let expected = schema.clone();
    let stream = async_stream::try_stream! {
        // No request or detached work exists before the stream is polled.
        let deadline = tokio::time::Instant::now() + connection.config.query_timeout;
        connection.counters.queries.fetch_add(1, Ordering::Relaxed);
        let mut cursor = connection.client.query(&sql).fetch_bytes("ArrowStream")
            .map_err(|_| error("query request failed"))?;
        let mut decoder = Decoder::default();
        let mut received = 0usize;
        loop {
            let chunk = tokio::time::timeout_at(deadline, cursor.next()).await
                .map_err(|_| error("remote query timed out"))?
                .map_err(|_| error("remote query failed while reading results"))?;
            let Some(bytes) = chunk else { break; };
            received = received.saturating_add(bytes.len());
            connection.counters.bytes.fetch_add(bytes.len() as u64, Ordering::Relaxed);
            if received > connection.config.max_response_bytes { Err(error("response byte budget exhausted"))?; }
            for batch in decoder.push(Buffer::from(bytes))? {
                if tokio::time::Instant::now() >= deadline { Err(error("remote query timed out"))?; }
                connection.counters.rows.fetch_add(batch.num_rows() as u64, Ordering::Relaxed);
                yield normalize(batch, &expected)?;
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
            // Fail on overflow rather than silently inserting NULL during conversion.
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
    use arrow_ipc::writer::StreamWriter;
    use datafusion::arrow::{
        array::{ArrayRef, Int64Array},
        datatypes::{DataType, Field, Schema},
    };

    fn fixture() -> (Vec<u8>, RecordBatch) {
        let schema = Arc::new(Schema::new(vec![Field::new(
            "depth_m",
            DataType::Int64,
            false,
        )]));
        let batch = RecordBatch::try_new(
            schema.clone(),
            vec![Arc::new(Int64Array::from(vec![1000, 2000])) as ArrayRef],
        )
        .unwrap();
        let mut bytes = Vec::new();
        let mut writer = StreamWriter::try_new(&mut bytes, &schema).unwrap();
        writer.write(&batch).unwrap();
        writer.write(&batch).unwrap();
        writer.finish().unwrap();
        (bytes, batch)
    }

    #[test]
    fn decoder_handles_coalesced_and_split_batches() {
        let (bytes, batch) = fixture();
        for chunk_size in [1, 3, 127, bytes.len()] {
            let mut decoder = Decoder::default();
            let mut batches = Vec::new();
            for chunk in bytes.chunks(chunk_size) {
                batches.extend(decoder.push(Buffer::from(chunk.to_vec())).unwrap());
            }
            assert_eq!(decoder.finish().unwrap(), batch.schema());
            assert_eq!(batches, vec![batch.clone(), batch.clone()]);
        }
    }

    #[test]
    fn decoder_rejects_truncated_and_missing_schema() {
        let (mut bytes, _) = fixture();
        bytes.truncate(bytes.len() - 20);
        let mut decoder = Decoder::default();
        decoder.push(Buffer::from(bytes)).unwrap();
        assert!(decoder.finish().is_err());
        assert!(Decoder::default().finish().is_err());
    }

    #[test]
    fn normalization_rejects_integer_overflow() {
        use datafusion::arrow::array::UInt64Array;
        let actual = Arc::new(Schema::new(vec![Field::new(
            "count",
            DataType::UInt64,
            false,
        )]));
        let batch = RecordBatch::try_new(actual, vec![Arc::new(UInt64Array::from(vec![u64::MAX]))])
            .unwrap();
        let expected = Arc::new(Schema::new(vec![Field::new(
            "count",
            DataType::Int64,
            false,
        )]));
        assert!(normalize(batch, &expected).is_err());
    }
}
