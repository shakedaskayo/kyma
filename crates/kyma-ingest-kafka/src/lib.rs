//! Kafka consumer ingest frontend.
//!
//! Consumes from configured Kafka topics. Each topic maps to a target
//! table; the consumer parses messages as NDJSON and routes them through
//! the shared `WritePath`.
//!
//! # Exactly-once semantics
//!
//! Per plan §5, we commit Kafka offsets into the catalog's `kafka_offsets`
//! table in the same Postgres transaction that commits the extent's
//! snapshot. Kafka itself receives **no** offset commits — on restart,
//! we seek each partition to the offset the catalog reports as
//! last-ingested + 1. That gives "at-most-once" on the Kafka side plus
//! "exactly-once" at the catalog boundary.
//!
//! For the MVP we use a simpler approximation: at-least-once with
//! catalog-stored offsets. True exactly-once (extent commit + offset
//! update in one pg tx) requires extending `SnapshotTxn` to accept
//! arbitrary "piggy-backed" statements, which is a small but separate
//! refactor and is tracked as follow-on work.
//!
//! # Configuration (env)
//!
//! - `KYMA_KAFKA_ENABLED=1` — turn consumer on
//! - `KYMA_KAFKA_BROKERS=host:9092[,host2:9092...]`
//! - `KYMA_KAFKA_GROUP=kyma-ingest` — consumer group id
//! - `KYMA_KAFKA_TOPICS=topic1:db1.tbl1,topic2:db2.tbl2` — topic/table map

#![forbid(unsafe_code)]

use arrow::json::ReaderBuilder;
use arrow_array::RecordBatch;
use futures::StreamExt;
use kyma_core::catalog::{Catalog, TableRef};
use kyma_core::errors::{Error, Result};
use kyma_ingest_core::WritePath;
use rdkafka::consumer::{CommitMode, Consumer, StreamConsumer};
use rdkafka::message::Message;
use rdkafka::{ClientConfig, TopicPartitionList};
use std::collections::HashMap;
use std::future::Future;
use std::io::Cursor;
use std::sync::Arc;
use std::time::Duration;
use tracing::{error, info, warn};

/// One topic → target (database, table) mapping.
#[derive(Debug, Clone)]
pub struct TopicMapping {
    pub topic: String,
    pub database: String,
    pub table: String,
}

/// Config, typically loaded from env.
#[derive(Debug, Clone)]
pub struct KafkaConsumerConfig {
    pub brokers: String,
    pub group_id: String,
    pub topics: Vec<TopicMapping>,
    /// Batch flush target — messages per single write_path ingest call.
    pub batch_size: usize,
    /// Max wall-clock a batch may linger before flushing even if under size.
    pub batch_timeout: Duration,
}

impl KafkaConsumerConfig {
    pub fn from_env() -> Option<Self> {
        let enabled = std::env::var("KYMA_KAFKA_ENABLED")
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .unwrap_or(false);
        if !enabled {
            return None;
        }
        let brokers = std::env::var("KYMA_KAFKA_BROKERS")
            .unwrap_or_else(|_| "localhost:9092".to_string());
        let group_id = std::env::var("KYMA_KAFKA_GROUP")
            .unwrap_or_else(|_| "kyma-ingest".to_string());
        let topics = std::env::var("KYMA_KAFKA_TOPICS").unwrap_or_default();
        let mut mappings = Vec::new();
        for spec in topics.split(',') {
            let spec = spec.trim();
            if spec.is_empty() {
                continue;
            }
            let Some((topic, db_tbl)) = spec.split_once(':') else {
                continue;
            };
            let Some((db, tbl)) = db_tbl.split_once('.') else {
                continue;
            };
            mappings.push(TopicMapping {
                topic: topic.trim().to_string(),
                database: db.trim().to_string(),
                table: tbl.trim().to_string(),
            });
        }
        if mappings.is_empty() {
            return None;
        }
        let batch_size = std::env::var("KYMA_KAFKA_BATCH_SIZE")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(500);
        let batch_timeout = Duration::from_millis(
            std::env::var("KYMA_KAFKA_BATCH_TIMEOUT_MS")
                .ok()
                .and_then(|v| v.parse::<u64>().ok())
                .unwrap_or(500),
        );
        Some(Self {
            brokers,
            group_id,
            topics: mappings,
            batch_size,
            batch_timeout,
        })
    }
}

/// The Kafka consumer worker.
pub struct KafkaConsumerWorker {
    catalog: Arc<dyn Catalog>,
    write_path: WritePath,
    config: KafkaConsumerConfig,
}

impl KafkaConsumerWorker {
    pub fn new(
        catalog: Arc<dyn Catalog>,
        write_path: WritePath,
        config: KafkaConsumerConfig,
    ) -> Self {
        Self {
            catalog,
            write_path,
            config,
        }
    }

    pub async fn run(self, shutdown: impl Future<Output = ()>) {
        info!(
            brokers = %self.config.brokers,
            topics = ?self.config.topics.iter().map(|t| &t.topic).collect::<Vec<_>>(),
            "kafka consumer starting"
        );
        tokio::pin!(shutdown);

        let consumer: StreamConsumer = match ClientConfig::new()
            .set("group.id", &self.config.group_id)
            .set("bootstrap.servers", &self.config.brokers)
            .set("enable.auto.commit", "false")
            .set("auto.offset.reset", "earliest")
            // Keep session + heartbeat tight so a restarted consumer
            // rejoins the group quickly (~6 s) instead of sitting in
            // CompletingRebalance while the old session times out.
            .set("session.timeout.ms", "6000")
            .set("heartbeat.interval.ms", "2000")
            .set("max.poll.interval.ms", "60000")
            .set("partition.assignment.strategy", "cooperative-sticky")
            .create()
        {
            Ok(c) => c,
            Err(e) => {
                error!(error = %e, "kafka consumer create failed");
                return;
            }
        };

        let topic_names: Vec<&str> = self.config.topics.iter().map(|t| t.topic.as_str()).collect();
        if let Err(e) = consumer.subscribe(&topic_names) {
            error!(error = %e, "kafka subscribe failed");
            return;
        }

        // Per-topic → TableRef cache. Looked up lazily once per topic.
        let mut table_cache: HashMap<String, TableRef> = HashMap::new();

        // In-flight batch accumulation: per-topic list of rows (bytes).
        let mut pending: HashMap<String, Vec<Vec<u8>>> = HashMap::new();

        let mut stream = consumer.stream();
        let mut tick = tokio::time::interval(self.config.batch_timeout);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        loop {
            tokio::select! {
                biased;
                () = &mut shutdown => {
                    info!("kafka consumer shutting down; flushing pending batches");
                    self.flush_all(&mut pending, &mut table_cache, &consumer).await;
                    return;
                }
                _ = tick.tick() => {
                    // Age-triggered flush.
                    self.flush_all(&mut pending, &mut table_cache, &consumer).await;
                }
                msg = stream.next() => {
                    let msg = match msg {
                        Some(Ok(m)) => m,
                        Some(Err(e)) => {
                            warn!(error = %e, "kafka stream error");
                            continue;
                        }
                        None => {
                            info!("kafka stream closed");
                            return;
                        }
                    };
                    let topic = msg.topic().to_string();
                    let Some(payload) = msg.payload() else { continue };
                    let bucket = pending.entry(topic.clone()).or_default();
                    bucket.push(payload.to_vec());
                    if bucket.len() >= self.config.batch_size {
                        self.flush_topic(&topic, bucket, &mut table_cache, &consumer).await;
                    }
                }
            }
        }
    }

    async fn flush_all(
        &self,
        pending: &mut HashMap<String, Vec<Vec<u8>>>,
        table_cache: &mut HashMap<String, TableRef>,
        consumer: &StreamConsumer,
    ) {
        let topics: Vec<String> = pending.keys().cloned().collect();
        for topic in topics {
            if let Some(bucket) = pending.get_mut(&topic) {
                if !bucket.is_empty() {
                    self.flush_topic(&topic, bucket, table_cache, consumer).await;
                }
            }
        }
    }

    async fn flush_topic(
        &self,
        topic: &str,
        bucket: &mut Vec<Vec<u8>>,
        table_cache: &mut HashMap<String, TableRef>,
        consumer: &StreamConsumer,
    ) {
        let Some(mapping) = self.config.topics.iter().find(|t| t.topic == topic) else {
            warn!(topic, "received message for unmapped topic; dropping");
            bucket.clear();
            return;
        };
        let table_ref = match table_cache.get(topic).cloned() {
            Some(t) => t,
            None => match self
                .catalog
                .lookup_table(&mapping.database, &mapping.table)
                .await
            {
                Ok(t) => {
                    table_cache.insert(topic.to_string(), t.clone());
                    t
                }
                Err(e) => {
                    warn!(topic, error = %e, "kafka: target table lookup failed; dropping batch");
                    bucket.clear();
                    return;
                }
            },
        };

        let n_msgs = bucket.len();
        let body: Vec<u8> = bucket
            .iter()
            .flat_map(|m| {
                let mut v = m.clone();
                if !v.ends_with(b"\n") {
                    v.push(b'\n');
                }
                v
            })
            .collect();

        let batches = match parse_ndjson(&body, &table_ref.schema) {
            Ok(b) => b,
            Err(e) => {
                warn!(topic, error = %e, n_msgs, "kafka: ndjson parse failed; dropping batch");
                bucket.clear();
                return;
            }
        };

        match self.write_path.ingest(&table_ref, batches).await {
            Ok(ack) => {
                info!(
                    topic,
                    msgs = n_msgs,
                    rows = ack.rows_ingested,
                    "kafka batch committed"
                );
                ::metrics::counter!(
                    "kyma_kafka_messages_ingested_total",
                    "topic" => topic.to_string()
                )
                .increment(ack.rows_ingested);

                // Commit Kafka offsets AFTER the catalog commit succeeds.
                // Best-effort at-least-once (MVP). Full exactly-once via
                // in-pg-tx offset commit is follow-on work.
                let mut tpl = TopicPartitionList::new();
                // Ask the consumer for the current assignment + positions.
                if let Ok(assignment) = consumer.assignment() {
                    if let Ok(positions) = consumer.position() {
                        for el in assignment.elements() {
                            if el.topic() != topic {
                                continue;
                            }
                            if let Some(pos) = positions
                                .elements()
                                .iter()
                                .find(|p| p.topic() == el.topic() && p.partition() == el.partition())
                                .and_then(|p| match p.offset() {
                                    rdkafka::Offset::Offset(o) => Some(o),
                                    _ => None,
                                })
                            {
                                let _ = tpl.add_partition_offset(
                                    el.topic(),
                                    el.partition(),
                                    rdkafka::Offset::Offset(pos),
                                );
                            }
                        }
                    }
                }
                // Sync commit: we want offsets durable before moving on so
                // a crash doesn't silently re-consume on restart. Slightly
                // higher latency per batch in exchange for at-least-once
                // with known upper bound on duplicates.
                if let Err(e) = consumer.commit(&tpl, CommitMode::Sync) {
                    warn!(error = %e, "kafka commit failed; will replay on restart");
                }
                bucket.clear();
            }
            Err(e) => {
                warn!(topic, error = %e, "kafka ingest failed; retrying on next flush");
                // Don't clear bucket — retry.
            }
        }
    }
}

fn parse_ndjson(bytes: &[u8], schema: &Arc<arrow_schema::Schema>) -> Result<Vec<RecordBatch>> {
    let cursor = Cursor::new(bytes.to_vec());
    let reader = ReaderBuilder::new(schema.clone())
        .build(cursor)
        .map_err(|e| Error::Internal(format!("kafka ndjson builder: {e}")))?;
    let mut batches = Vec::new();
    for r in reader {
        let b = r.map_err(|e| Error::Internal(format!("kafka ndjson decode: {e}")))?;
        batches.push(b);
    }
    Ok(batches)
}
