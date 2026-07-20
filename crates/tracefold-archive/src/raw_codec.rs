use std::{
    collections::BTreeMap,
    fs::{self, File},
    io::{self, BufReader, BufWriter, Cursor, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
};

use blake3::Hash;
use tracefold_core::{CanonicalEvent, Severity, Status};

use crate::ARCHIVE_FORMAT_VERSION;

const MAGIC: &[u8; 8] = b"TFLDRAW!";
const TARGET_BLOCK_ROWS: usize = 4_096;
const TARGET_BLOCK_BYTES: usize = 4 * 1024 * 1024;
const MAX_BLOCK_BYTES: usize = 64 * 1024 * 1024;
const INDEX_ENTRY_BYTES: u64 = 68;

#[derive(Debug, Clone)]
struct RawBlockIndex {
    min_timestamp_ns: i64,
    max_timestamp_ns: i64,
    offset: u64,
    compressed_len: u32,
    uncompressed_len: u32,
    event_count: u32,
    hash: Hash,
}

pub struct RawEventWriter {
    path: PathBuf,
    blocks_path: PathBuf,
    blocks: BufWriter<File>,
    indexes: Vec<RawBlockIndex>,
    events: Vec<CanonicalEvent>,
    estimated_bytes: usize,
    zstd_level: i32,
}

impl RawEventWriter {
    pub fn create(path: impl AsRef<Path>, zstd_level: i32) -> io::Result<Self> {
        let path = path.as_ref().to_owned();
        let blocks_path = path.with_extension("tfr.blocks");
        Ok(Self {
            path,
            blocks_path: blocks_path.clone(),
            blocks: BufWriter::new(File::create(blocks_path)?),
            indexes: Vec::new(),
            events: Vec::new(),
            estimated_bytes: 0,
            zstd_level,
        })
    }

    pub fn push(&mut self, event: CanonicalEvent) -> anyhow::Result<()> {
        self.estimated_bytes = self
            .estimated_bytes
            .saturating_add(event.canonical_line()?.len());
        self.events.push(event);
        if self.events.len() >= TARGET_BLOCK_ROWS || self.estimated_bytes >= TARGET_BLOCK_BYTES {
            self.flush_block()?;
        }
        Ok(())
    }

    fn flush_block(&mut self) -> anyhow::Result<()> {
        if self.events.is_empty() {
            return Ok(());
        }
        let bytes = encode_block(&self.events)?;
        if bytes.len() > MAX_BLOCK_BYTES {
            anyhow::bail!("retained-event block exceeds 64 MiB cap");
        }
        let compressed = zstd::stream::encode_all(Cursor::new(&bytes), self.zstd_level)?;
        let min_timestamp_ns = self
            .events
            .iter()
            .map(|event| event.timestamp_ns)
            .min()
            .expect("non-empty raw block");
        let max_timestamp_ns = self
            .events
            .iter()
            .map(|event| event.timestamp_ns)
            .max()
            .expect("non-empty raw block");
        self.indexes.push(RawBlockIndex {
            min_timestamp_ns,
            max_timestamp_ns,
            offset: 0,
            compressed_len: u32::try_from(compressed.len())?,
            uncompressed_len: u32::try_from(bytes.len())?,
            event_count: u32::try_from(self.events.len())?,
            hash: blake3::hash(&bytes),
        });
        self.blocks.write_all(&compressed)?;
        self.events.clear();
        self.estimated_bytes = 0;
        Ok(())
    }

    pub fn finish(mut self) -> anyhow::Result<()> {
        self.flush_block()?;
        self.blocks.flush()?;
        self.blocks.get_ref().sync_all()?;
        drop(self.blocks);

        let header_bytes = 8_u64 + 2 + 4;
        let mut offset = header_bytes + INDEX_ENTRY_BYTES * self.indexes.len() as u64;
        for index in &mut self.indexes {
            index.offset = offset;
            offset = offset.saturating_add(index.compressed_len as u64);
        }

        let mut output = BufWriter::new(File::create(&self.path)?);
        output.write_all(MAGIC)?;
        write_u16(&mut output, ARCHIVE_FORMAT_VERSION)?;
        write_u32(&mut output, u32::try_from(self.indexes.len())?)?;
        for index in &self.indexes {
            write_index(&mut output, index)?;
        }
        io::copy(&mut File::open(&self.blocks_path)?, &mut output)?;
        output.flush()?;
        output.get_ref().sync_all()?;
        fs::remove_file(self.blocks_path)?;
        Ok(())
    }
}

pub struct RawEventReader {
    file: BufReader<File>,
    indexes: Vec<RawBlockIndex>,
}

impl RawEventReader {
    pub fn open(path: impl AsRef<Path>) -> anyhow::Result<Self> {
        let mut file = BufReader::new(File::open(path)?);
        let mut magic = [0_u8; 8];
        file.read_exact(&mut magic)?;
        if &magic != MAGIC {
            anyhow::bail!("corrupt retained events: bad magic");
        }
        let version = read_u16(&mut file)?;
        if version != ARCHIVE_FORMAT_VERSION {
            anyhow::bail!("unsupported retained-event version {version}");
        }
        let block_count = read_u32(&mut file)? as usize;
        let file_len = file.get_ref().metadata()?.len();
        let index_end = 14_u64.saturating_add(INDEX_ENTRY_BYTES * block_count as u64);
        if index_end > file_len {
            anyhow::bail!("corrupt retained events: block index exceeds file");
        }
        let mut indexes = Vec::with_capacity(block_count.min(1_000_000));
        let mut previous_end = index_end;
        for _ in 0..block_count {
            let index = read_index(&mut file)?;
            let end = index.offset.saturating_add(index.compressed_len as u64);
            if index.event_count == 0
                || index.uncompressed_len as usize > MAX_BLOCK_BYTES
                || index.min_timestamp_ns > index.max_timestamp_ns
                || index.offset < previous_end
                || end > file_len
            {
                anyhow::bail!("corrupt retained events: invalid block bounds");
            }
            previous_end = end;
            indexes.push(index);
        }
        Ok(Self { file, indexes })
    }

    pub fn events(&mut self) -> anyhow::Result<Vec<CanonicalEvent>> {
        let mut events = Vec::new();
        for index in self.indexes.clone() {
            self.file.seek(SeekFrom::Start(index.offset))?;
            let mut compressed = vec![0; index.compressed_len as usize];
            self.file.read_exact(&mut compressed)?;
            let bytes = zstd::stream::decode_all(Cursor::new(compressed))?;
            if bytes.len() != index.uncompressed_len as usize || blake3::hash(&bytes) != index.hash
            {
                anyhow::bail!("corrupt retained events: block length or hash mismatch");
            }
            let block = decode_block(bytes, index.event_count as usize)?;
            let observed_min = block.iter().map(|event| event.timestamp_ns).min();
            let observed_max = block.iter().map(|event| event.timestamp_ns).max();
            if observed_min != Some(index.min_timestamp_ns)
                || observed_max != Some(index.max_timestamp_ns)
            {
                anyhow::bail!("corrupt retained events: timestamp bounds mismatch");
            }
            events.extend(block);
        }
        Ok(events)
    }
}

fn encode_block(events: &[CanonicalEvent]) -> anyhow::Result<Vec<u8>> {
    let dictionary = block_dictionary(events);
    let ids: BTreeMap<&str, u64> = dictionary
        .iter()
        .enumerate()
        .map(|(index, value)| (value.as_str(), index as u64 + 1))
        .collect();
    let mut output = Vec::new();
    put_uvarint(&mut output, dictionary.len() as u64);
    for value in &dictionary {
        put_bytes(&mut output, value.as_bytes());
    }
    let mut previous_timestamp = 0_i64;
    for event in events {
        let delta = event
            .timestamp_ns
            .checked_sub(previous_timestamp)
            .ok_or_else(|| anyhow::anyhow!("retained-event timestamp delta overflow"))?;
        put_svarint(&mut output, delta);
        previous_timestamp = event.timestamp_ns;

        let optional_strings = [
            event.trace_id.as_deref(),
            event.span_id.as_deref(),
            event.parent_span_id.as_deref(),
            event.operation.as_deref(),
            event.error_code.as_deref(),
            event.model.as_deref(),
        ];
        let optional_numbers = [
            event.duration_ns,
            event.bytes_in,
            event.bytes_out,
            event.tokens_in,
            event.tokens_out,
        ];
        let mut presence = 0_u16;
        for (index, value) in optional_strings.iter().enumerate() {
            presence |= u16::from(value.is_some()) << index;
        }
        for (index, value) in optional_numbers.iter().enumerate() {
            presence |= u16::from(value.is_some()) << (6 + index);
        }
        put_uvarint(&mut output, u64::from(presence));

        put_string_ref(&mut output, &event.event_id, &ids);
        for value in optional_strings.iter().take(3).flatten() {
            put_string_ref(&mut output, value, &ids);
        }
        put_string_ref(&mut output, &event.service, &ids);
        if let Some(value) = event.operation.as_deref() {
            put_string_ref(&mut output, value, &ids);
        }
        put_string_ref(&mut output, &event.event_type, &ids);
        output.push(severity_code(event.severity));
        output.push(status_code(event.status));
        for value in optional_strings.iter().skip(4).flatten() {
            put_string_ref(&mut output, value, &ids);
        }
        for value in optional_numbers.iter().flatten() {
            put_svarint(&mut output, *value);
        }
        put_uvarint(&mut output, event.attributes.len() as u64);
        for (key, value) in &event.attributes {
            put_string_ref(&mut output, key, &ids);
            output.push(u8::from(value.is_some()));
            if let Some(value) = value {
                put_string_ref(&mut output, value, &ids);
            }
        }
        put_bytes(&mut output, &serde_json::to_vec(&event.body)?);
    }
    Ok(output)
}

fn decode_block(bytes: Vec<u8>, event_count: usize) -> anyhow::Result<Vec<CanonicalEvent>> {
    let mut input = Cursor::new(bytes);
    let dictionary_len = usize::try_from(get_uvarint(&mut input)?)?;
    if dictionary_len > 1_000_000 {
        anyhow::bail!("corrupt retained events: dictionary is too large");
    }
    let mut dictionary = Vec::with_capacity(dictionary_len);
    for _ in 0..dictionary_len {
        dictionary.push(get_string(&mut input)?);
    }
    if dictionary.windows(2).any(|window| window[0] >= window[1]) {
        anyhow::bail!("corrupt retained events: dictionary is not strictly sorted");
    }

    let mut events = Vec::with_capacity(event_count);
    let mut previous_timestamp = 0_i64;
    for _ in 0..event_count {
        let timestamp_ns = previous_timestamp
            .checked_add(get_svarint(&mut input)?)
            .ok_or_else(|| anyhow::anyhow!("corrupt retained events: timestamp overflow"))?;
        previous_timestamp = timestamp_ns;
        let presence = get_uvarint(&mut input)?;
        if presence > 0x07ff {
            anyhow::bail!("corrupt retained events: invalid presence bitmap");
        }
        let event_id = get_string_ref(&mut input, &dictionary)?;
        let trace_id = get_optional_string(&mut input, &dictionary, presence, 0)?;
        let span_id = get_optional_string(&mut input, &dictionary, presence, 1)?;
        let parent_span_id = get_optional_string(&mut input, &dictionary, presence, 2)?;
        let service = get_string_ref(&mut input, &dictionary)?;
        let operation = get_optional_string(&mut input, &dictionary, presence, 3)?;
        let event_type = get_string_ref(&mut input, &dictionary)?;
        let severity = decode_severity(read_byte(&mut input)?)?;
        let status = decode_status(read_byte(&mut input)?)?;
        let error_code = get_optional_string(&mut input, &dictionary, presence, 4)?;
        let model = get_optional_string(&mut input, &dictionary, presence, 5)?;
        let duration_ns = get_optional_i64(&mut input, presence, 6)?;
        let bytes_in = get_optional_i64(&mut input, presence, 7)?;
        let bytes_out = get_optional_i64(&mut input, presence, 8)?;
        let tokens_in = get_optional_i64(&mut input, presence, 9)?;
        let tokens_out = get_optional_i64(&mut input, presence, 10)?;
        let attribute_count = usize::try_from(get_uvarint(&mut input)?)?;
        if attribute_count > 1_000_000 {
            anyhow::bail!("corrupt retained events: too many attributes");
        }
        let mut attributes = BTreeMap::new();
        for _ in 0..attribute_count {
            let key = get_string_ref(&mut input, &dictionary)?;
            let value = match read_byte(&mut input)? {
                0 => None,
                1 => Some(get_string_ref(&mut input, &dictionary)?),
                _ => anyhow::bail!("corrupt retained events: invalid attribute flag"),
            };
            if attributes.insert(key, value).is_some() {
                anyhow::bail!("corrupt retained events: duplicate attribute key");
            }
        }
        let body = serde_json::from_slice(&get_bytes(&mut input)?)?;
        let event = CanonicalEvent {
            schema_version: tracefold_core::CANONICAL_SCHEMA_VERSION,
            timestamp_ns,
            event_id,
            trace_id,
            span_id,
            parent_span_id,
            service,
            operation,
            event_type,
            severity,
            status,
            error_code,
            model,
            duration_ns,
            bytes_in,
            bytes_out,
            tokens_in,
            tokens_out,
            attributes,
            body,
        };
        event.validate()?;
        events.push(event);
    }
    if input.position() != input.get_ref().len() as u64 {
        anyhow::bail!("corrupt retained events: trailing block bytes");
    }
    Ok(events)
}

fn block_dictionary(events: &[CanonicalEvent]) -> Vec<String> {
    let mut frequencies = BTreeMap::<&str, usize>::new();
    for event in events {
        let fixed = [
            Some(event.event_id.as_str()),
            event.trace_id.as_deref(),
            event.span_id.as_deref(),
            event.parent_span_id.as_deref(),
            Some(event.service.as_str()),
            event.operation.as_deref(),
            Some(event.event_type.as_str()),
            event.error_code.as_deref(),
            event.model.as_deref(),
        ];
        for value in fixed.into_iter().flatten() {
            *frequencies.entry(value).or_default() += 1;
        }
        for (key, value) in &event.attributes {
            *frequencies.entry(key).or_default() += 1;
            if let Some(value) = value {
                *frequencies.entry(value).or_default() += 1;
            }
        }
    }
    frequencies
        .into_iter()
        .filter(|(value, count)| *count >= 3 || (*count >= 2 && value.len() >= 8))
        .map(|(value, _)| value.to_owned())
        .collect()
}

fn put_string_ref(output: &mut Vec<u8>, value: &str, ids: &BTreeMap<&str, u64>) {
    if let Some(id) = ids.get(value) {
        put_uvarint(output, *id);
    } else {
        put_uvarint(output, 0);
        put_bytes(output, value.as_bytes());
    }
}

fn get_string_ref(input: &mut Cursor<Vec<u8>>, dictionary: &[String]) -> anyhow::Result<String> {
    let id = get_uvarint(input)?;
    if id == 0 {
        get_string(input)
    } else {
        dictionary
            .get(usize::try_from(id - 1)?)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("corrupt retained events: dictionary ID out of bounds"))
    }
}

fn get_optional_string(
    input: &mut Cursor<Vec<u8>>,
    dictionary: &[String],
    presence: u64,
    bit: u32,
) -> anyhow::Result<Option<String>> {
    if presence & (1_u64 << bit) == 0 {
        Ok(None)
    } else {
        Ok(Some(get_string_ref(input, dictionary)?))
    }
}

fn get_optional_i64(
    input: &mut Cursor<Vec<u8>>,
    presence: u64,
    bit: u32,
) -> anyhow::Result<Option<i64>> {
    if presence & (1_u64 << bit) == 0 {
        Ok(None)
    } else {
        Ok(Some(get_svarint(input)?))
    }
}

fn put_bytes(output: &mut Vec<u8>, bytes: &[u8]) {
    put_uvarint(output, bytes.len() as u64);
    output.extend_from_slice(bytes);
}

fn get_bytes(input: &mut Cursor<Vec<u8>>) -> anyhow::Result<Vec<u8>> {
    let length = usize::try_from(get_uvarint(input)?)?;
    let remaining = input.get_ref().len() as u64 - input.position();
    if length as u64 > remaining {
        anyhow::bail!("corrupt retained events: byte string exceeds block");
    }
    let mut bytes = vec![0; length];
    input.read_exact(&mut bytes)?;
    Ok(bytes)
}

fn get_string(input: &mut Cursor<Vec<u8>>) -> anyhow::Result<String> {
    Ok(String::from_utf8(get_bytes(input)?)?)
}

fn severity_code(value: Severity) -> u8 {
    match value {
        Severity::Trace => 0,
        Severity::Debug => 1,
        Severity::Info => 2,
        Severity::Warn => 3,
        Severity::Error => 4,
        Severity::Fatal => 5,
        Severity::Unknown => 6,
    }
}

fn decode_severity(value: u8) -> anyhow::Result<Severity> {
    Ok(match value {
        0 => Severity::Trace,
        1 => Severity::Debug,
        2 => Severity::Info,
        3 => Severity::Warn,
        4 => Severity::Error,
        5 => Severity::Fatal,
        6 => Severity::Unknown,
        _ => anyhow::bail!("corrupt retained events: invalid severity"),
    })
}

fn status_code(value: Status) -> u8 {
    match value {
        Status::Ok => 0,
        Status::Error => 1,
        Status::Unset => 2,
    }
}

fn decode_status(value: u8) -> anyhow::Result<Status> {
    Ok(match value {
        0 => Status::Ok,
        1 => Status::Error,
        2 => Status::Unset,
        _ => anyhow::bail!("corrupt retained events: invalid status"),
    })
}

fn read_byte(input: &mut Cursor<Vec<u8>>) -> anyhow::Result<u8> {
    let mut byte = [0];
    input.read_exact(&mut byte)?;
    Ok(byte[0])
}

fn put_uvarint(output: &mut Vec<u8>, mut value: u64) {
    while value >= 0x80 {
        output.push((value as u8) | 0x80);
        value >>= 7;
    }
    output.push(value as u8);
}

fn get_uvarint(input: &mut Cursor<Vec<u8>>) -> anyhow::Result<u64> {
    let mut value = 0_u64;
    for shift in (0..=63).step_by(7) {
        let byte = read_byte(input)?;
        value |= u64::from(byte & 0x7f) << shift;
        if byte & 0x80 == 0 {
            return Ok(value);
        }
    }
    anyhow::bail!("corrupt retained events: varint overflow")
}

fn put_svarint(output: &mut Vec<u8>, value: i64) {
    put_uvarint(output, ((value << 1) ^ (value >> 63)) as u64);
}

fn get_svarint(input: &mut Cursor<Vec<u8>>) -> anyhow::Result<i64> {
    let value = get_uvarint(input)?;
    Ok(((value >> 1) as i64) ^ (-((value & 1) as i64)))
}

fn write_index(mut writer: impl Write, index: &RawBlockIndex) -> io::Result<()> {
    writer.write_all(&index.min_timestamp_ns.to_le_bytes())?;
    writer.write_all(&index.max_timestamp_ns.to_le_bytes())?;
    writer.write_all(&index.offset.to_le_bytes())?;
    write_u32(&mut writer, index.compressed_len)?;
    write_u32(&mut writer, index.uncompressed_len)?;
    write_u32(&mut writer, index.event_count)?;
    writer.write_all(index.hash.as_bytes())
}

fn read_index(mut reader: impl Read) -> io::Result<RawBlockIndex> {
    let mut i64_bytes = [0; 8];
    reader.read_exact(&mut i64_bytes)?;
    let min_timestamp_ns = i64::from_le_bytes(i64_bytes);
    reader.read_exact(&mut i64_bytes)?;
    let max_timestamp_ns = i64::from_le_bytes(i64_bytes);
    let mut u64_bytes = [0; 8];
    reader.read_exact(&mut u64_bytes)?;
    let offset = u64::from_le_bytes(u64_bytes);
    let compressed_len = read_u32(&mut reader)?;
    let uncompressed_len = read_u32(&mut reader)?;
    let event_count = read_u32(&mut reader)?;
    let mut hash = [0; 32];
    reader.read_exact(&mut hash)?;
    Ok(RawBlockIndex {
        min_timestamp_ns,
        max_timestamp_ns,
        offset,
        compressed_len,
        uncompressed_len,
        event_count,
        hash: Hash::from_bytes(hash),
    })
}

fn write_u16(mut writer: impl Write, value: u16) -> io::Result<()> {
    writer.write_all(&value.to_le_bytes())
}

fn write_u32(mut writer: impl Write, value: u32) -> io::Result<()> {
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

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use serde_json::json;
    use tempfile::tempdir;

    use super::*;

    #[test]
    fn retained_events_round_trip_all_fields() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("events.tfr");
        let event = CanonicalEvent {
            schema_version: tracefold_core::CANONICAL_SCHEMA_VERSION,
            timestamp_ns: -7,
            event_id: "event-α".into(),
            trace_id: Some("trace".into()),
            span_id: Some("span".into()),
            parent_span_id: Some("parent".into()),
            service: "service".into(),
            operation: Some("operation".into()),
            event_type: "type".into(),
            severity: Severity::Error,
            status: Status::Error,
            error_code: Some("E1".into()),
            model: Some("model".into()),
            duration_ns: Some(-1),
            bytes_in: Some(0),
            bytes_out: Some(2),
            tokens_in: Some(3),
            tokens_out: Some(4),
            attributes: BTreeMap::from([
                ("null".into(), None),
                ("unicode".into(), Some("λ".into())),
            ]),
            body: json!({"nested": [true, null, {"x": 1}]}),
        };
        let mut writer = RawEventWriter::create(&path, 3).unwrap();
        writer.push(event.clone()).unwrap();
        writer.push(event.clone()).unwrap();
        writer.finish().unwrap();
        assert_eq!(
            RawEventReader::open(path).unwrap().events().unwrap(),
            vec![event.clone(), event]
        );
    }
}
