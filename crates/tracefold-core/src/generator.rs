use std::{collections::BTreeMap, io::Write};

use blake3::Hasher;
use rand::{Rng, SeedableRng, seq::SliceRandom};
use rand_chacha::ChaCha8Rng;
use serde::{Deserialize, Serialize};
use serde_json::json;
use thiserror::Error;

use crate::{CanonicalEvent, Severity, Status};

const START_NS: i64 = 1_784_064_000_000_000_000;
const THIRTY_DAYS_NS: i64 = 30 * 86_400_000_000_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Scenario {
    Standard,
    LowCardinality,
    HighCardinality,
    HighEntropyBody,
    ErrorBurst,
    OutOfOrder,
}

impl std::str::FromStr for Scenario {
    type Err = GeneratorError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "standard" => Ok(Self::Standard),
            "low-cardinality" => Ok(Self::LowCardinality),
            "high-cardinality" => Ok(Self::HighCardinality),
            "high-entropy-body" => Ok(Self::HighEntropyBody),
            "error-burst" => Ok(Self::ErrorBurst),
            "out-of-order" => Ok(Self::OutOfOrder),
            _ => Err(GeneratorError::Scenario(value.into())),
        }
    }
}

#[derive(Debug, Clone)]
pub struct GeneratorConfig {
    pub scenario: Scenario,
    pub events: Option<u64>,
    pub max_output_bytes: Option<u64>,
    pub seed: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeneratorMetadata {
    pub schema_version: u16,
    pub generator_version: String,
    pub scenario: Scenario,
    pub seed: u64,
    pub record_count: u64,
    pub bytes: u64,
    pub min_timestamp_ns: Option<i64>,
    pub max_timestamp_ns: Option<i64>,
    pub error_count: u64,
    pub blake3: String,
    pub field_cardinalities: BTreeMap<String, u64>,
}

#[derive(Debug, Error)]
pub enum GeneratorError {
    #[error("unknown generator scenario `{0}`")]
    Scenario(String),
    #[error("exactly one of events or max_output_bytes is required")]
    Limit,
    #[error("output cap is too small for one event")]
    CapTooSmall,
    #[error("write failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("JSON encoding failed: {0}")]
    Json(#[from] serde_json::Error),
}

pub fn generate<W: Write>(
    config: &GeneratorConfig,
    mut writer: W,
) -> Result<GeneratorMetadata, GeneratorError> {
    if config.events.is_some() == config.max_output_bytes.is_some() {
        return Err(GeneratorError::Limit);
    }
    let target = config.events.unwrap_or(u64::MAX);
    let cap = config.max_output_bytes.unwrap_or(u64::MAX);
    let timeline_target = config
        .events
        .unwrap_or_else(|| cap.saturating_div(640).max(1));
    let mut rng = ChaCha8Rng::seed_from_u64(config.seed);
    let mut hasher = Hasher::new();
    let mut stats = GenerationStats::default();
    let mut generated = 0_u64;
    let mut services = BTreeMap::<String, ()>::new();
    let mut operations = BTreeMap::<String, ()>::new();
    let mut window = Vec::with_capacity(if config.scenario == Scenario::OutOfOrder {
        10_000
    } else {
        1
    });

    while generated < target {
        let event = make_event(config.scenario, generated, timeline_target, &mut rng);
        services.insert(event.service.clone(), ());
        if let Some(operation) = &event.operation {
            operations.insert(operation.clone(), ());
        }
        window.push(event);
        generated += 1;
        let flush = config.scenario != Scenario::OutOfOrder
            || window.len() == 10_000
            || generated == target;
        if flush {
            if config.scenario == Scenario::OutOfOrder {
                window.shuffle(&mut rng);
            }
            for event in window.drain(..) {
                let mut line = serde_json::to_vec(&event)?;
                line.push(b'\n');
                let new_size = stats.bytes.saturating_add(line.len() as u64);
                if new_size > cap {
                    if stats.count == 0 {
                        return Err(GeneratorError::CapTooSmall);
                    }
                    return Ok(metadata(
                        config,
                        &stats,
                        &hasher,
                        services.len(),
                        operations.len(),
                    ));
                }
                writer.write_all(&line)?;
                hasher.update(&line);
                stats.bytes = new_size;
                stats.count += 1;
                stats.errors += u64::from(event.status == Status::Error);
                stats.min_timestamp = Some(
                    stats
                        .min_timestamp
                        .map_or(event.timestamp_ns, |old| old.min(event.timestamp_ns)),
                );
                stats.max_timestamp = Some(
                    stats
                        .max_timestamp
                        .map_or(event.timestamp_ns, |old| old.max(event.timestamp_ns)),
                );
            }
        }
    }
    Ok(metadata(
        config,
        &stats,
        &hasher,
        services.len(),
        operations.len(),
    ))
}

#[derive(Default)]
struct GenerationStats {
    count: u64,
    bytes: u64,
    errors: u64,
    min_timestamp: Option<i64>,
    max_timestamp: Option<i64>,
}

fn metadata(
    config: &GeneratorConfig,
    stats: &GenerationStats,
    hasher: &Hasher,
    services: usize,
    operations: usize,
) -> GeneratorMetadata {
    let mut field_cardinalities = BTreeMap::new();
    field_cardinalities.insert("service".into(), services as u64);
    field_cardinalities.insert("operation".into(), operations as u64);
    GeneratorMetadata {
        schema_version: 1,
        generator_version: env!("CARGO_PKG_VERSION").into(),
        scenario: config.scenario,
        seed: config.seed,
        record_count: stats.count,
        bytes: stats.bytes,
        min_timestamp_ns: stats.min_timestamp,
        max_timestamp_ns: stats.max_timestamp,
        error_count: stats.errors,
        blake3: hasher.clone().finalize().to_hex().to_string(),
        field_cardinalities,
    }
}

fn make_event(scenario: Scenario, index: u64, target: u64, rng: &mut ChaCha8Rng) -> CanonicalEvent {
    let (service_count, operation_count, error_per_10k) = match scenario {
        Scenario::LowCardinality => (4, 16, 10),
        Scenario::HighCardinality => (64, 100_000, 100),
        _ => (16, 256, 100),
    };
    let timestamp_ns =
        START_NS + ((index as u128 * THIRTY_DAYS_NS as u128) / target.max(1) as u128) as i64;
    let burst = scenario == Scenario::ErrorBurst
        && ((timestamp_ns - START_NS) / 1_800_000_000_000) % 120 == 0;
    let error = if burst {
        rng.random_range(0..4) == 0
    } else {
        rng.random_range(0..10_000) < error_per_10k
    };
    let service_id = rng.random_range(0..service_count);
    let operation_id = if scenario == Scenario::HighCardinality {
        rng.random_range(0..operation_count)
    } else {
        let a = rng.random_range(0..operation_count);
        let b = rng.random_range(0..operation_count);
        a.min(b)
    };
    let mut attributes = BTreeMap::new();
    attributes.insert(
        "region".into(),
        Some(format!("region-{}", rng.random_range(0..4))),
    );
    attributes.insert(
        "host".into(),
        Some(format!("host-{}", rng.random_range(0..128))),
    );
    let body = if scenario == Scenario::HighEntropyBody {
        let bytes: Vec<u8> = (0..512).map(|_| rng.random()).collect();
        json!({"random_hex": bytes.iter().map(|b| format!("{b:02x}")).collect::<String>()})
    } else {
        json!({"message": format!("template-{} completed", rng.random_range(0..64))})
    };
    CanonicalEvent {
        schema_version: 1,
        timestamp_ns,
        event_id: format!("evt-{index:012}"),
        trace_id: Some(format!("trace-{:010}", index / 8)),
        span_id: Some(format!("span-{index:012}")),
        parent_span_id: (index % 8 != 0).then(|| format!("span-{:012}", index - 1)),
        service: format!("service-{service_id:03}"),
        operation: Some(format!("operation-{operation_id:06}")),
        event_type: if error {
            "request.failed"
        } else {
            "request.completed"
        }
        .into(),
        severity: if error {
            Severity::Error
        } else {
            Severity::Info
        },
        status: if error { Status::Error } else { Status::Ok },
        error_code: error.then(|| format!("E{:05}", rng.random_range(0..10_000))),
        model: (index % 5 == 0).then(|| format!("model-{}", rng.random_range(0..8))),
        duration_ns: Some(500_000 + rng.random_range(0..1_500_000_000_i64)),
        bytes_in: Some(rng.random_range(64..65_536)),
        bytes_out: Some(rng.random_range(64..262_144)),
        tokens_in: (index % 5 == 0).then(|| rng.random_range(1..8_192)),
        tokens_out: (index % 5 == 0).then(|| rng.random_range(1..4_096)),
        attributes,
        body,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_seed_is_byte_identical() {
        let config = GeneratorConfig {
            scenario: Scenario::Standard,
            events: Some(100),
            max_output_bytes: None,
            seed: 7,
        };
        let mut first = Vec::new();
        let mut second = Vec::new();
        let one = generate(&config, &mut first).unwrap();
        let two = generate(&config, &mut second).unwrap();
        assert_eq!(first, second);
        assert_eq!(one.blake3, two.blake3);
    }

    #[test]
    fn byte_cap_is_not_exceeded() {
        let config = GeneratorConfig {
            scenario: Scenario::Standard,
            events: None,
            max_output_bytes: Some(10_000),
            seed: 7,
        };
        let mut output = Vec::new();
        let metadata = generate(&config, &mut output).unwrap();
        assert!(metadata.bytes <= 10_000);
        assert_eq!(metadata.bytes as usize, output.len());
    }
}
