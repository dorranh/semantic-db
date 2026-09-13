//! Framed IPC decoding with bounded decompression before Arrow sees compressed data.
use crate::failure;
use arrow_ipc::{CompressionType, MessageHeader, reader::StreamDecoder};
use datafusion::{
    arrow::{array::RecordBatch, buffer::Buffer, datatypes::SchemaRef},
    error::Result,
};
use std::io::Read;

/// Bounded codec workspace, in addition to buffers proportional to declared output.
/// LZ4 frames use up to 4 MiB blocks; Zstd windows are bounded below.
pub const DECODER_SCRATCH_BYTES: usize = 16 * 1024 * 1024;

pub struct IpcDecoder {
    decoder: StreamDecoder,
    pending: Vec<u8>,
    limit: usize,
    decoded: usize,
    finished: bool,
    retained: usize,
}
impl IpcDecoder {
    pub fn new(limit: usize) -> Self {
        Self {
            decoder: StreamDecoder::new(),
            pending: vec![],
            limit,
            decoded: 0,
            finished: false,
            retained: 0,
        }
    }
    pub fn retained_bytes(&self) -> usize {
        self.retained
    }
    pub fn decoded_bytes(&self) -> usize {
        self.decoded
    }
    pub fn push(&mut self, bytes: &[u8]) -> Result<()> {
        if self
            .pending
            .len()
            .checked_add(bytes.len())
            .is_none_or(|n| n > self.limit)
        {
            return Err(failure("IPC input buffer budget exhausted"));
        }
        if self.finished && !bytes.is_empty() {
            return Err(failure("data after Arrow end marker"));
        }
        self.pending.extend_from_slice(bytes);
        Ok(())
    }
    pub fn buffered_bytes(&self) -> usize {
        self.pending.len()
    }
    pub fn next_batch(&mut self) -> Result<Option<RecordBatch>> {
        self.next_batch_admitted(|_, _| Ok(()))
    }
    /// Admission runs before decompressing or allocating Arrow arrays.
    pub fn next_batch_admitted(
        &mut self,
        mut admit: impl FnMut(usize, usize) -> Result<()>,
    ) -> Result<Option<RecordBatch>> {
        loop {
            if self.pending.len() < 4 {
                return Ok(None);
            }
            let continuation = self.pending[..4] == [255; 4];
            let prefix = if continuation { 8 } else { 4 };
            if self.pending.len() < prefix {
                return Ok(None);
            }
            let meta_len =
                u32::from_le_bytes(self.pending[prefix - 4..prefix].try_into().unwrap()) as usize;
            if meta_len == 0 {
                if self.pending.len() != prefix {
                    return Err(failure("data after Arrow end marker"));
                }
                let mut end = Buffer::from(std::mem::take(&mut self.pending));
                self.decoder
                    .decode(&mut end)
                    .map_err(|_| failure("invalid Arrow end marker"))?;
                self.finished = true;
                return Ok(None);
            }
            if meta_len > 4 * 1024 * 1024 {
                return Err(failure("Arrow metadata budget exhausted"));
            }
            if self.pending.len() < prefix + meta_len {
                return Ok(None);
            }
            let message = arrow_ipc::root_as_message(&self.pending[prefix..prefix + meta_len])
                .map_err(|_| failure("invalid Arrow metadata"))?;
            let body_len = usize::try_from(message.bodyLength())
                .map_err(|_| failure("invalid Arrow body length"))?;
            let frame_len = (prefix + meta_len)
                .checked_add(body_len)
                .ok_or_else(|| failure("invalid Arrow length"))?;
            if frame_len > self.limit {
                return Err(failure("Arrow message budget exhausted"));
            }
            if self.pending.len() < frame_len {
                return Ok(None);
            }
            let frame = self.pending.drain(..frame_len).collect::<Vec<_>>();
            let message = arrow_ipc::root_as_message(&frame[prefix..prefix + meta_len])
                .map_err(|_| failure("invalid Arrow metadata"))?;
            let body = &frame[prefix + meta_len..];
            let record = match message.header_type() {
                MessageHeader::RecordBatch => message.header_as_record_batch(),
                MessageHeader::DictionaryBatch => {
                    message.header_as_dictionary_batch().and_then(|d| d.data())
                }
                MessageHeader::Schema => None,
                _ => return Err(failure("unsupported Arrow message")),
            };
            let retained = self.retained;
            let mut admission = |size| admit(size, retained);
            let (mut bytes, size) = if let Some(record) = record {
                if record.length() < 0 || record.length() as u64 > (self.limit / 8) as u64 {
                    return Err(failure("Arrow row count exceeds decoder budget"));
                }
                if record.compression().is_some() {
                    expand(
                        message,
                        record,
                        body,
                        self.limit.saturating_sub(self.decoded),
                        &mut admission,
                    )?
                } else {
                    admission(body_len)?;
                    (frame.clone(), body_len)
                }
            } else {
                admission(frame_len)?;
                (frame.clone(), frame_len)
            };
            if message.header_type() != MessageHeader::RecordBatch {
                self.retained = self.retained.saturating_add(size);
            }
            self.decoded = self
                .decoded
                .checked_add(size)
                .filter(|n| *n <= self.limit)
                .ok_or_else(|| failure("decoded Arrow byte budget exhausted"))?;
            let mut buffer = Buffer::from(std::mem::take(&mut bytes));
            let batch = self
                .decoder
                .decode(&mut buffer)
                .map_err(|_| failure("invalid Arrow stream"))?;
            if !buffer.is_empty() {
                return Err(failure("unconsumed Arrow frame"));
            }
            if batch.is_some() {
                return Ok(batch);
            }
        }
    }
    pub fn schema(&self) -> Option<SchemaRef> {
        self.decoder.schema()
    }
    pub fn finish(&mut self) -> Result<SchemaRef> {
        if !self.pending.is_empty() {
            return Err(failure("incomplete Arrow stream"));
        }
        self.decoder
            .finish()
            .map_err(|_| failure("incomplete Arrow stream"))?;
        self.schema()
            .ok_or_else(|| failure("Arrow stream has no schema"))
    }
}

fn expand(
    message: arrow_ipc::Message<'_>,
    record: arrow_ipc::RecordBatch<'_>,
    body: &[u8],
    limit: usize,
    admit: &mut dyn FnMut(usize) -> Result<()>,
) -> Result<(Vec<u8>, usize)> {
    let codec = record.compression().unwrap().codec();
    let buffers = record
        .buffers()
        .ok_or_else(|| failure("Arrow record has no buffers"))?;
    // Validate all declared decompressed sizes before allocating any output.
    let mut sizes = Vec::with_capacity(buffers.len());
    let mut total = 0usize;
    for b in buffers {
        let start = usize::try_from(b.offset()).map_err(|_| failure("invalid Arrow offset"))?;
        let len = usize::try_from(b.length()).map_err(|_| failure("invalid Arrow length"))?;
        let input = body
            .get(
                start
                    ..start
                        .checked_add(len)
                        .ok_or_else(|| failure("invalid Arrow length"))?,
            )
            .ok_or_else(|| failure("invalid Arrow buffer"))?;
        let size = if input.is_empty() {
            0
        } else {
            let head: [u8; 8] = input
                .get(..8)
                .ok_or_else(|| failure("invalid compression prefix"))?
                .try_into()
                .unwrap();
            match i64::from_le_bytes(head) {
                -1 => input.len() - 8,
                n if n >= 0 => {
                    usize::try_from(n).map_err(|_| failure("invalid decompressed length"))?
                }
                _ => return Err(failure("invalid decompressed length")),
            }
        };
        total = total
            .checked_add(size)
            .and_then(|n| n.checked_add(8))
            .filter(|n| *n <= limit)
            .ok_or_else(|| failure("decoded Arrow byte budget exhausted"))?;
        sizes.push((input, size));
    }
    admit(total)?;
    let mut output = Vec::with_capacity(total);
    let mut offsets = vec![];
    for (input, size) in sizes {
        let offset = output.len();
        if !input.is_empty() {
            let raw = i64::from_le_bytes(input[..8].try_into().unwrap()) == -1;
            if raw {
                output.extend_from_slice(&input[8..]);
            } else {
                let reader: Box<dyn Read + '_> = match codec {
                    CompressionType::LZ4_FRAME => {
                        Box::new(lz4_flex::frame::FrameDecoder::new(&input[8..]))
                    }
                    CompressionType::ZSTD => {
                        let mut decoder = zstd::stream::read::Decoder::new(&input[8..])
                            .map_err(|_| failure("invalid zstd stream"))?;
                        decoder
                            .window_log_max(
                                (usize::BITS
                                    - size.max(8 * 1024 * 1024).saturating_sub(1).leading_zeros())
                                .min(30),
                            )
                            .map_err(|_| failure("invalid zstd window bound"))?;
                        Box::new(decoder)
                    }
                    _ => return Err(failure("unsupported Arrow compression")),
                };
                reader
                    .take(size as u64 + 1)
                    .read_to_end(&mut output)
                    .map_err(|_| failure("Arrow decompression failed"))?;
                if output.len() - offset != size {
                    return Err(failure("Arrow decompression length mismatch"));
                }
            }
        }
        offsets.push(arrow_ipc::Buffer::new(offset as i64, size as i64));
        while output.len() % 8 != 0 {
            output.push(0);
        }
    }
    let mut builder = flatbuffers::FlatBufferBuilder::new();
    let nodes = record
        .nodes()
        .map(|v| builder.create_vector(&v.iter().collect::<Vec<_>>()));
    let buffers = Some(builder.create_vector(&offsets));
    let variadic = record
        .variadicBufferCounts()
        .map(|v| builder.create_vector(&v.iter().collect::<Vec<_>>()));
    let data = arrow_ipc::RecordBatch::create(
        &mut builder,
        &arrow_ipc::RecordBatchArgs {
            length: record.length(),
            nodes,
            buffers,
            compression: None,
            variadicBufferCounts: variadic,
        },
    );
    let header = if let Some(dictionary) = message.header_as_dictionary_batch() {
        arrow_ipc::DictionaryBatch::create(
            &mut builder,
            &arrow_ipc::DictionaryBatchArgs {
                id: dictionary.id(),
                data: Some(data),
                isDelta: dictionary.isDelta(),
            },
        )
        .as_union_value()
    } else {
        data.as_union_value()
    };
    let rebuilt = arrow_ipc::Message::create(
        &mut builder,
        &arrow_ipc::MessageArgs {
            version: message.version(),
            header_type: message.header_type(),
            header: Some(header),
            bodyLength: output.len() as i64,
            custom_metadata: None,
        },
    );
    builder.finish(rebuilt, None);
    let meta = builder.finished_data();
    let padded = (meta.len() + 7) & !7;
    let mut result = Vec::with_capacity(8 + padded + output.len());
    result.extend_from_slice(&[255; 4]);
    result.extend_from_slice(&(padded as u32).to_le_bytes());
    result.extend_from_slice(meta);
    result.resize(8 + padded, 0);
    result.extend_from_slice(&output);
    Ok((result, total))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zstd_window_is_bounded_by_the_buffer_not_the_query_budget() {
        for (window, valid) in [(0, true), (0x88, false)] {
            // A valid one-byte raw Zstd frame with an unknown content size. The
            // second variant advertises a 128 MiB window for that single byte.
            let mut body = 1i64.to_le_bytes().to_vec();
            body.extend_from_slice(&[0x28, 0xb5, 0x2f, 0xfd, 0, window, 9, 0, 0, b'x']);
            let mut builder = flatbuffers::FlatBufferBuilder::new();
            let buffers = builder.create_vector(&[arrow_ipc::Buffer::new(0, body.len() as i64)]);
            let compression = arrow_ipc::BodyCompression::create(
                &mut builder,
                &arrow_ipc::BodyCompressionArgs {
                    codec: CompressionType::ZSTD,
                    ..Default::default()
                },
            );
            let record = arrow_ipc::RecordBatch::create(
                &mut builder,
                &arrow_ipc::RecordBatchArgs {
                    length: 1,
                    buffers: Some(buffers),
                    compression: Some(compression),
                    ..Default::default()
                },
            );
            let message = arrow_ipc::Message::create(
                &mut builder,
                &arrow_ipc::MessageArgs {
                    version: arrow_ipc::MetadataVersion::V5,
                    header_type: MessageHeader::RecordBatch,
                    header: Some(record.as_union_value()),
                    bodyLength: body.len() as i64,
                    ..Default::default()
                },
            );
            builder.finish(message, None);
            let message = arrow_ipc::root_as_message(builder.finished_data()).unwrap();
            let mut admitted = 0;
            let result = expand(
                message,
                message.header_as_record_batch().unwrap(),
                &body,
                1024 * 1024 * 1024,
                &mut |size| {
                    admitted = size;
                    Ok(())
                },
            );
            assert_eq!(admitted, 9);
            assert_eq!(result.is_ok(), valid);
        }
    }
}
