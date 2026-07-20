use std::{
    fs::File,
    io::{self, BufReader, BufWriter, Cursor, Read, Seek, SeekFrom, Write},
    path::Path,
};

use blake3::Hash;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tracefold_core::{CellKey, CellState, Measure};

use crate::VIEW_FORMAT_VERSION;

const MAGIC: &[u8; 8] = b"TFLDVIEW";
const MAX_BLOCK_UNCOMPRESSED: usize = 16 * 1024 * 1024;
const TARGET_BLOCK_UNCOMPRESSED: usize = 4 * 1024 * 1024;
const TARGET_BLOCK_ROWS: usize = 4_096;
const INDEX_ENTRY_BYTES: u64 = 68;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ViewSchema {
    pub dimensions: Vec<String>,
    pub measures: Vec<Measure>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EncodedRow {
    pub key: CellKey,
    pub state: CellState,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockIndex {
    pub min_bucket: i64,
    pub max_bucket_exclusive: i64,
    pub offset: u64,
    pub compressed_len: u32,
    pub uncompressed_len: u32,
    pub row_count: u32,
    pub hash: Hash,
}

#[derive(Debug, Error)]
pub enum CodecError {
    #[error("view I/O error: {0}")]
    Io(#[from] io::Error),
    #[error("view schema error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("corrupt view: {0}")]
    Corrupt(String),
    #[error("unsupported view version {0}")]
    Version(u16),
}

struct CompressedBlock {
    index: BlockIndex,
    bytes: Vec<u8>,
}

pub fn write_view(
    path: &Path,
    bucket_width: i64,
    schema: &ViewSchema,
    rows: impl IntoIterator<Item = EncodedRow>,
    zstd_level: i32,
) -> Result<Vec<BlockIndex>, CodecError> {
    let schema_bytes = serde_json::to_vec(schema)?;
    let mut blocks = Vec::new();
    let mut current = Vec::new();
    let mut row_count = 0_usize;
    let mut min_bucket = 0_i64;
    let mut max_bucket = 0_i64;
    let mut previous_bucket = 0_i64;

    for row in rows {
        if row_count == 0 {
            min_bucket = row.key.bucket_ns;
            previous_bucket = 0;
        }
        encode_row(&mut current, &row, &mut previous_bucket, &schema.measures)?;
        max_bucket = row.key.bucket_ns;
        row_count += 1;
        if row_count >= TARGET_BLOCK_ROWS || current.len() >= TARGET_BLOCK_UNCOMPRESSED {
            blocks.push(compress_block(
                std::mem::take(&mut current),
                min_bucket,
                max_bucket.saturating_add(bucket_width),
                row_count,
                zstd_level,
            )?);
            row_count = 0;
        }
    }
    if row_count > 0 {
        blocks.push(compress_block(
            current,
            min_bucket,
            max_bucket.saturating_add(bucket_width),
            row_count,
            zstd_level,
        )?);
    }

    let header_bytes = 8_u64 + 2 + 8 + 4 + schema_bytes.len() as u64 + 4;
    let mut offset = header_bytes + INDEX_ENTRY_BYTES * blocks.len() as u64;
    for block in &mut blocks {
        block.index.offset = offset;
        offset = offset.saturating_add(block.bytes.len() as u64);
    }

    let mut writer = BufWriter::new(File::create(path)?);
    writer.write_all(MAGIC)?;
    write_u16(&mut writer, VIEW_FORMAT_VERSION)?;
    write_i64(&mut writer, bucket_width)?;
    write_u32(
        &mut writer,
        u32::try_from(schema_bytes.len())
            .map_err(|_| CodecError::Corrupt("schema is too large".into()))?,
    )?;
    writer.write_all(&schema_bytes)?;
    write_u32(
        &mut writer,
        u32::try_from(blocks.len()).map_err(|_| CodecError::Corrupt("too many blocks".into()))?,
    )?;
    for block in &blocks {
        write_index(&mut writer, &block.index)?;
    }
    for block in &blocks {
        writer.write_all(&block.bytes)?;
    }
    writer.flush()?;
    Ok(blocks.into_iter().map(|block| block.index).collect())
}

fn compress_block(
    bytes: Vec<u8>,
    min_bucket: i64,
    max_bucket_exclusive: i64,
    row_count: usize,
    zstd_level: i32,
) -> Result<CompressedBlock, CodecError> {
    if bytes.len() > MAX_BLOCK_UNCOMPRESSED {
        return Err(CodecError::Corrupt("block exceeds 16 MiB cap".into()));
    }
    let hash = blake3::hash(&bytes);
    let compressed = zstd::stream::encode_all(Cursor::new(&bytes), zstd_level)?;
    Ok(CompressedBlock {
        index: BlockIndex {
            min_bucket,
            max_bucket_exclusive,
            offset: 0,
            compressed_len: u32::try_from(compressed.len())
                .map_err(|_| CodecError::Corrupt("compressed block is too large".into()))?,
            uncompressed_len: u32::try_from(bytes.len())
                .map_err(|_| CodecError::Corrupt("block is too large".into()))?,
            row_count: u32::try_from(row_count)
                .map_err(|_| CodecError::Corrupt("too many rows in block".into()))?,
            hash,
        },
        bytes: compressed,
    })
}

pub struct ViewReader {
    file: BufReader<File>,
    pub bucket_width: i64,
    pub schema: ViewSchema,
    pub blocks: Vec<BlockIndex>,
}

impl ViewReader {
    pub fn open(path: &Path) -> Result<Self, CodecError> {
        let mut file = BufReader::new(File::open(path)?);
        let mut magic = [0_u8; 8];
        file.read_exact(&mut magic)?;
        if &magic != MAGIC {
            return Err(CodecError::Corrupt("bad magic".into()));
        }
        let version = read_u16(&mut file)?;
        if version != VIEW_FORMAT_VERSION {
            return Err(CodecError::Version(version));
        }
        let bucket_width = read_i64(&mut file)?;
        if bucket_width <= 0 {
            return Err(CodecError::Corrupt("invalid bucket width".into()));
        }
        let schema_len = read_u32(&mut file)? as usize;
        if schema_len > 1024 * 1024 {
            return Err(CodecError::Corrupt("schema exceeds 1 MiB".into()));
        }
        let mut schema = vec![0; schema_len];
        file.read_exact(&mut schema)?;
        let schema: ViewSchema = serde_json::from_slice(&schema)?;
        if schema.dimensions.len() > 8 {
            return Err(CodecError::Corrupt("more than eight dimensions".into()));
        }
        let block_count = read_u32(&mut file)? as usize;
        let file_len = file.get_ref().metadata()?.len();
        let mut blocks = Vec::with_capacity(block_count.min(1_000_000));
        for _ in 0..block_count {
            let block = read_index(&mut file)?;
            if block.uncompressed_len as usize > MAX_BLOCK_UNCOMPRESSED
                || block.offset.saturating_add(block.compressed_len as u64) > file_len
                || block.min_bucket >= block.max_bucket_exclusive
            {
                return Err(CodecError::Corrupt("invalid block bounds".into()));
            }
            blocks.push(block);
        }
        Ok(Self {
            file,
            bucket_width,
            schema,
            blocks,
        })
    }

    pub fn rows(
        &mut self,
        start_ns: Option<i64>,
        end_ns: Option<i64>,
    ) -> Result<Vec<EncodedRow>, CodecError> {
        let selected: Vec<BlockIndex> = self
            .blocks
            .iter()
            .filter(|block| {
                start_ns.is_none_or(|start| block.max_bucket_exclusive > start)
                    && end_ns.is_none_or(|end| block.min_bucket < end)
            })
            .cloned()
            .collect();
        let mut rows = Vec::new();
        for block in selected {
            self.file.seek(SeekFrom::Start(block.offset))?;
            let mut compressed = vec![0; block.compressed_len as usize];
            self.file.read_exact(&mut compressed)?;
            let bytes = zstd::stream::decode_all(Cursor::new(compressed))?;
            if bytes.len() != block.uncompressed_len as usize || blake3::hash(&bytes) != block.hash
            {
                return Err(CodecError::Corrupt("block length or hash mismatch".into()));
            }
            let mut cursor = Cursor::new(bytes);
            let mut previous_bucket = 0_i64;
            for _ in 0..block.row_count {
                rows.push(decode_row(
                    &mut cursor,
                    &mut previous_bucket,
                    self.schema.dimensions.len(),
                    &self.schema.measures,
                )?);
            }
            if cursor.position() != cursor.get_ref().len() as u64 {
                return Err(CodecError::Corrupt("trailing block bytes".into()));
            }
        }
        Ok(rows)
    }
}

fn encode_row(
    output: &mut Vec<u8>,
    row: &EncodedRow,
    previous_bucket: &mut i64,
    measures: &[Measure],
) -> Result<(), CodecError> {
    if row.state.metrics.len() != measures.len() {
        return Err(CodecError::Corrupt("metric count mismatch".into()));
    }
    let delta = row
        .key
        .bucket_ns
        .checked_sub(*previous_bucket)
        .ok_or_else(|| CodecError::Corrupt("bucket delta overflow".into()))?;
    put_svarint(output, delta);
    *previous_bucket = row.key.bucket_ns;
    for dimension in &row.key.dimensions {
        let id = match dimension {
            tracefold_core::ScalarValue::Null => 0,
            tracefold_core::ScalarValue::String(value) => value
                .parse::<u64>()
                .map_err(|_| CodecError::Corrupt("dimension id is not numeric".into()))?,
        };
        put_uvarint(output, id);
    }
    for (measure, metric) in measures.iter().zip(&row.state.metrics) {
        match measure.operation {
            tracefold_core::MeasureOp::Count => put_uvarint(output, metric.count),
            tracefold_core::MeasureOp::CountPresent => put_uvarint(output, metric.present_count),
            tracefold_core::MeasureOp::Sum => {
                put_optional_i64(output, (metric.present_count > 0).then_some(metric.sum))
            }
            tracefold_core::MeasureOp::Min => put_optional_i64(output, metric.min),
            tracefold_core::MeasureOp::Max => put_optional_i64(output, metric.max),
            tracefold_core::MeasureOp::Histogram => {
                if metric.histogram.len() != measure.bounds.len() + 1 {
                    return Err(CodecError::Corrupt("histogram length mismatch".into()));
                }
                for value in &metric.histogram {
                    put_uvarint(output, *value);
                }
            }
        }
    }
    Ok(())
}

fn decode_row(
    input: &mut Cursor<Vec<u8>>,
    previous_bucket: &mut i64,
    dimension_count: usize,
    measures: &[Measure],
) -> Result<EncodedRow, CodecError> {
    let delta = get_svarint(input)?;
    let bucket_ns = previous_bucket
        .checked_add(delta)
        .ok_or_else(|| CodecError::Corrupt("bucket overflow".into()))?;
    *previous_bucket = bucket_ns;
    let dimensions = (0..dimension_count)
        .map(|_| {
            get_uvarint(input).map(|id| {
                if id == 0 {
                    tracefold_core::ScalarValue::Null
                } else {
                    tracefold_core::ScalarValue::String(id.to_string())
                }
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    let mut metrics = Vec::with_capacity(measures.len());
    for measure in measures {
        let mut metric = tracefold_core::aggregate::MetricState {
            count: 0,
            present_count: 0,
            sum: 0,
            min: None,
            max: None,
            histogram: Vec::new(),
        };
        match measure.operation {
            tracefold_core::MeasureOp::Count => metric.count = get_uvarint(input)?,
            tracefold_core::MeasureOp::CountPresent => {
                metric.present_count = get_uvarint(input)?;
                metric.count = metric.present_count;
            }
            tracefold_core::MeasureOp::Sum => {
                if let Some(sum) = get_optional_i64(input)? {
                    metric.count = 1;
                    metric.present_count = 1;
                    metric.sum = sum;
                }
            }
            tracefold_core::MeasureOp::Min => {
                metric.min = get_optional_i64(input)?;
                metric.count = u64::from(metric.min.is_some());
                metric.present_count = metric.count;
            }
            tracefold_core::MeasureOp::Max => {
                metric.max = get_optional_i64(input)?;
                metric.count = u64::from(metric.max.is_some());
                metric.present_count = metric.count;
            }
            tracefold_core::MeasureOp::Histogram => {
                metric.histogram = (0..measure.bounds.len() + 1)
                    .map(|_| get_uvarint(input))
                    .collect::<Result<Vec<_>, _>>()?;
                metric.present_count = metric
                    .histogram
                    .iter()
                    .try_fold(0_u64, |sum, value| sum.checked_add(*value))
                    .ok_or_else(|| CodecError::Corrupt("histogram count overflow".into()))?;
                metric.count = metric.present_count;
            }
        }
        metrics.push(metric);
    }
    Ok(EncodedRow {
        key: CellKey {
            bucket_ns,
            dimensions,
        },
        state: CellState { metrics },
    })
}

fn put_uvarint(output: &mut Vec<u8>, mut value: u64) {
    while value >= 0x80 {
        output.push((value as u8) | 0x80);
        value >>= 7;
    }
    output.push(value as u8);
}

fn get_uvarint(input: &mut Cursor<Vec<u8>>) -> Result<u64, CodecError> {
    let mut value = 0_u64;
    for shift in (0..=63).step_by(7) {
        let mut byte = [0_u8; 1];
        input
            .read_exact(&mut byte)
            .map_err(|_| CodecError::Corrupt("truncated varint".into()))?;
        value |= u64::from(byte[0] & 0x7f) << shift;
        if byte[0] & 0x80 == 0 {
            return Ok(value);
        }
    }
    Err(CodecError::Corrupt("varint overflow".into()))
}

fn put_svarint(output: &mut Vec<u8>, value: i64) {
    put_uvarint(output, ((value << 1) ^ (value >> 63)) as u64);
}

fn get_svarint(input: &mut Cursor<Vec<u8>>) -> Result<i64, CodecError> {
    let value = get_uvarint(input)?;
    Ok(((value >> 1) as i64) ^ (-((value & 1) as i64)))
}

fn put_optional_i64(output: &mut Vec<u8>, value: Option<i64>) {
    output.push(u8::from(value.is_some()));
    if let Some(value) = value {
        put_svarint(output, value);
    }
}

fn get_optional_i64(input: &mut Cursor<Vec<u8>>) -> Result<Option<i64>, CodecError> {
    let mut flag = [0_u8; 1];
    input
        .read_exact(&mut flag)
        .map_err(|_| CodecError::Corrupt("truncated optional integer".into()))?;
    match flag[0] {
        0 => Ok(None),
        1 => Ok(Some(get_svarint(input)?)),
        _ => Err(CodecError::Corrupt("invalid optional integer flag".into())),
    }
}

fn write_index(mut writer: impl Write, index: &BlockIndex) -> io::Result<()> {
    write_i64(&mut writer, index.min_bucket)?;
    write_i64(&mut writer, index.max_bucket_exclusive)?;
    writer.write_all(&index.offset.to_le_bytes())?;
    write_u32(&mut writer, index.compressed_len)?;
    write_u32(&mut writer, index.uncompressed_len)?;
    write_u32(&mut writer, index.row_count)?;
    writer.write_all(index.hash.as_bytes())
}

fn read_index(mut reader: impl Read) -> io::Result<BlockIndex> {
    let min_bucket = read_i64(&mut reader)?;
    let max_bucket_exclusive = read_i64(&mut reader)?;
    let mut offset = [0_u8; 8];
    reader.read_exact(&mut offset)?;
    let compressed_len = read_u32(&mut reader)?;
    let uncompressed_len = read_u32(&mut reader)?;
    let row_count = read_u32(&mut reader)?;
    let mut hash = [0_u8; 32];
    reader.read_exact(&mut hash)?;
    Ok(BlockIndex {
        min_bucket,
        max_bucket_exclusive,
        offset: u64::from_le_bytes(offset),
        compressed_len,
        uncompressed_len,
        row_count,
        hash: Hash::from_bytes(hash),
    })
}

fn write_u16(mut writer: impl Write, value: u16) -> io::Result<()> {
    writer.write_all(&value.to_le_bytes())
}
fn write_u32(mut writer: impl Write, value: u32) -> io::Result<()> {
    writer.write_all(&value.to_le_bytes())
}
fn write_i64(mut writer: impl Write, value: i64) -> io::Result<()> {
    writer.write_all(&value.to_le_bytes())
}
fn read_u16(mut reader: impl Read) -> io::Result<u16> {
    let mut bytes = [0; 2];
    reader.read_exact(&mut bytes)?;
    Ok(u16::from_le_bytes(bytes))
}
fn read_u32(mut reader: impl Read) -> io::Result<u32> {
    let mut bytes = [0; 4];
    reader.read_exact(&mut bytes)?;
    Ok(u32::from_le_bytes(bytes))
}
fn read_i64(mut reader: impl Read) -> io::Result<i64> {
    let mut bytes = [0; 8];
    reader.read_exact(&mut bytes)?;
    Ok(i64::from_le_bytes(bytes))
}

#[cfg(test)]
mod tests {
    use tracefold_core::{Measure, MeasureOp, ScalarValue, aggregate::MetricState};

    use super::*;

    #[test]
    fn view_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("view.tfv");
        let schema = ViewSchema {
            dimensions: vec!["service".into()],
            measures: vec![Measure {
                field: "*".into(),
                operation: MeasureOp::Count,
                bounds: vec![],
            }],
        };
        let row = EncodedRow {
            key: CellKey {
                bucket_ns: 60,
                dimensions: vec![ScalarValue::String("1".into())],
            },
            state: CellState {
                metrics: vec![MetricState {
                    count: 2,
                    present_count: 0,
                    sum: 0,
                    min: None,
                    max: None,
                    histogram: vec![],
                }],
            },
        };
        write_view(&path, 60, &schema, [row.clone()], 3).unwrap();
        let mut reader = ViewReader::open(&path).unwrap();
        assert_eq!(reader.rows(None, None).unwrap(), vec![row]);
    }

    #[test]
    fn operation_specific_metrics_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("metrics.tfv");
        let schema = ViewSchema {
            dimensions: vec![],
            measures: vec![
                Measure {
                    field: "*".into(),
                    operation: MeasureOp::Count,
                    bounds: vec![],
                },
                Measure {
                    field: "x".into(),
                    operation: MeasureOp::CountPresent,
                    bounds: vec![],
                },
                Measure {
                    field: "x".into(),
                    operation: MeasureOp::Sum,
                    bounds: vec![],
                },
                Measure {
                    field: "x".into(),
                    operation: MeasureOp::Min,
                    bounds: vec![],
                },
                Measure {
                    field: "x".into(),
                    operation: MeasureOp::Max,
                    bounds: vec![],
                },
                Measure {
                    field: "x".into(),
                    operation: MeasureOp::Histogram,
                    bounds: vec![0, 10],
                },
            ],
        };
        let metrics = vec![
            MetricState {
                count: 7,
                present_count: 0,
                sum: 0,
                min: None,
                max: None,
                histogram: vec![],
            },
            MetricState {
                count: 3,
                present_count: 3,
                sum: 0,
                min: None,
                max: None,
                histogram: vec![],
            },
            MetricState {
                count: 1,
                present_count: 1,
                sum: -9,
                min: None,
                max: None,
                histogram: vec![],
            },
            MetricState {
                count: 1,
                present_count: 1,
                sum: 0,
                min: Some(-4),
                max: None,
                histogram: vec![],
            },
            MetricState {
                count: 1,
                present_count: 1,
                sum: 0,
                min: None,
                max: Some(12),
                histogram: vec![],
            },
            MetricState {
                count: 6,
                present_count: 6,
                sum: 0,
                min: None,
                max: None,
                histogram: vec![1, 2, 3],
            },
        ];
        let row = EncodedRow {
            key: CellKey {
                bucket_ns: 60,
                dimensions: vec![],
            },
            state: CellState { metrics },
        };
        write_view(&path, 60, &schema, [row.clone()], 3).unwrap();
        assert_eq!(
            ViewReader::open(&path).unwrap().rows(None, None).unwrap(),
            vec![row]
        );
    }
}
