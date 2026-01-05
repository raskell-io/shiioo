use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// Metrics collector for system observability
pub struct MetricsCollector {
    counters: Arc<Mutex<HashMap<String, Counter>>>,
    gauges: Arc<Mutex<HashMap<String, Gauge>>>,
    histograms: Arc<Mutex<HashMap<String, Histogram>>>,
}

/// Counter - monotonically increasing value
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Counter {
    pub name: String,
    pub value: u64,
    pub labels: HashMap<String, String>,
    pub last_updated: DateTime<Utc>,
}

/// Gauge - value that can go up or down
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Gauge {
    pub name: String,
    pub value: f64,
    pub labels: HashMap<String, String>,
    pub last_updated: DateTime<Utc>,
}

/// Histogram - tracks distribution of values
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Histogram {
    pub name: String,
    pub buckets: Vec<f64>,
    pub counts: Vec<u64>,
    pub sum: f64,
    pub count: u64,
    pub labels: HashMap<String, String>,
    pub last_updated: DateTime<Utc>,
}

impl MetricsCollector {
    /// Create a new metrics collector
    pub fn new() -> Self {
        Self {
            counters: Arc::new(Mutex::new(HashMap::new())),
            gauges: Arc::new(Mutex::new(HashMap::new())),
            histograms: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Increment a counter
    pub fn increment_counter(&self, name: &str, labels: HashMap<String, String>) {
        self.increment_counter_by(name, 1, labels);
    }

    /// Increment a counter by a specific amount
    pub fn increment_counter_by(&self, name: &str, value: u64, labels: HashMap<String, String>) {
        let mut counters = self.counters.lock().unwrap();
        let key = Self::metric_key(name, &labels);

        counters
            .entry(key.clone())
            .and_modify(|c| {
                c.value += value;
                c.last_updated = Utc::now();
            })
            .or_insert_with(|| Counter {
                name: name.to_string(),
                value,
                labels,
                last_updated: Utc::now(),
            });
    }

    /// Set a gauge value
    pub fn set_gauge(&self, name: &str, value: f64, labels: HashMap<String, String>) {
        let mut gauges = self.gauges.lock().unwrap();
        let key = Self::metric_key(name, &labels);

        gauges
            .entry(key.clone())
            .and_modify(|g| {
                g.value = value;
                g.last_updated = Utc::now();
            })
            .or_insert_with(|| Gauge {
                name: name.to_string(),
                value,
                labels,
                last_updated: Utc::now(),
            });
    }

    /// Increment a gauge
    pub fn increment_gauge(&self, name: &str, delta: f64, labels: HashMap<String, String>) {
        let mut gauges = self.gauges.lock().unwrap();
        let key = Self::metric_key(name, &labels);

        gauges
            .entry(key.clone())
            .and_modify(|g| {
                g.value += delta;
                g.last_updated = Utc::now();
            })
            .or_insert_with(|| Gauge {
                name: name.to_string(),
                value: delta,
                labels,
                last_updated: Utc::now(),
            });
    }

    /// Decrement a gauge
    pub fn decrement_gauge(&self, name: &str, delta: f64, labels: HashMap<String, String>) {
        self.increment_gauge(name, -delta, labels);
    }

    /// Record a histogram observation
    pub fn observe_histogram(&self, name: &str, value: f64, labels: HashMap<String, String>) {
        let mut histograms = self.histograms.lock().unwrap();
        let key = Self::metric_key(name, &labels);

        histograms
            .entry(key.clone())
            .and_modify(|h| {
                h.sum += value;
                h.count += 1;

                // Update bucket counts
                for (i, bucket) in h.buckets.iter().enumerate() {
                    if value <= *bucket {
                        h.counts[i] += 1;
                    }
                }
                h.last_updated = Utc::now();
            })
            .or_insert_with(|| {
                // Default buckets: 0.01, 0.1, 0.5, 1, 5, 10, 30, 60, 120, 300
                let buckets = vec![0.01, 0.1, 0.5, 1.0, 5.0, 10.0, 30.0, 60.0, 120.0, 300.0];
                let mut counts = vec![0; buckets.len()];

                // Initialize counts for this first observation
                for (i, bucket) in buckets.iter().enumerate() {
                    if value <= *bucket {
                        counts[i] = 1;
                    }
                }

                Histogram {
                    name: name.to_string(),
                    buckets,
                    counts,
                    sum: value,
                    count: 1,
                    labels,
                    last_updated: Utc::now(),
                }
            });
    }

    /// Get all counters
    pub fn get_counters(&self) -> Vec<Counter> {
        self.counters.lock().unwrap().values().cloned().collect()
    }

    /// Get all gauges
    pub fn get_gauges(&self) -> Vec<Gauge> {
        self.gauges.lock().unwrap().values().cloned().collect()
    }

    /// Get all histograms
    pub fn get_histograms(&self) -> Vec<Histogram> {
        self.histograms.lock().unwrap().values().cloned().collect()
    }

    /// Get a specific counter
    pub fn get_counter(&self, name: &str, labels: &HashMap<String, String>) -> Option<Counter> {
        let key = Self::metric_key(name, labels);
        self.counters.lock().unwrap().get(&key).cloned()
    }

    /// Get a specific gauge
    pub fn get_gauge(&self, name: &str, labels: &HashMap<String, String>) -> Option<Gauge> {
        let key = Self::metric_key(name, labels);
        self.gauges.lock().unwrap().get(&key).cloned()
    }

    /// Get a specific histogram
    pub fn get_histogram(&self, name: &str, labels: &HashMap<String, String>) -> Option<Histogram> {
        let key = Self::metric_key(name, labels);
        self.histograms.lock().unwrap().get(&key).cloned()
    }

    /// Reset all metrics
    pub fn reset(&self) {
        self.counters.lock().unwrap().clear();
        self.gauges.lock().unwrap().clear();
        self.histograms.lock().unwrap().clear();
    }

    /// Generate a unique key for a metric with labels
    fn metric_key(name: &str, labels: &HashMap<String, String>) -> String {
        if labels.is_empty() {
            return name.to_string();
        }

        let mut sorted_labels: Vec<_> = labels.iter().collect();
        sorted_labels.sort_by_key(|(k, _)| *k);

        let label_str = sorted_labels
            .iter()
            .map(|(k, v)| format!("{}={}", k, v))
            .collect::<Vec<_>>()
            .join(",");

        format!("{}:{}", name, label_str)
    }
}

impl Default for MetricsCollector {
    fn default() -> Self {
        Self::new()
    }
}

impl Histogram {
    /// Calculate percentile (p50, p95, p99, etc.)
    pub fn percentile(&self, p: f64) -> Option<f64> {
        if self.count == 0 || p < 0.0 || p > 100.0 {
            return None;
        }

        let target_count = ((self.count as f64) * (p / 100.0)).ceil() as u64;

        for (i, count) in self.counts.iter().enumerate() {
            if *count >= target_count {
                return Some(self.buckets[i]);
            }
        }

        None
    }

    /// Calculate average
    pub fn average(&self) -> Option<f64> {
        if self.count == 0 {
            None
        } else {
            Some(self.sum / self.count as f64)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_counter_increment() {
        let collector = MetricsCollector::new();
        let labels = HashMap::from([("service".to_string(), "api".to_string())]);

        collector.increment_counter("requests_total", labels.clone());
        collector.increment_counter("requests_total", labels.clone());

        let counter = collector.get_counter("requests_total", &labels).unwrap();
        assert_eq!(counter.value, 2);
        assert_eq!(counter.name, "requests_total");
    }

    #[test]
    fn test_counter_increment_by() {
        let collector = MetricsCollector::new();
        let labels = HashMap::new();

        collector.increment_counter_by("bytes_sent", 100, labels.clone());
        collector.increment_counter_by("bytes_sent", 200, labels.clone());

        let counter = collector.get_counter("bytes_sent", &labels).unwrap();
        assert_eq!(counter.value, 300);
    }

    #[test]
    fn test_gauge_set() {
        let collector = MetricsCollector::new();
        let labels = HashMap::new();

        collector.set_gauge("memory_usage", 1024.5, labels.clone());
        collector.set_gauge("memory_usage", 2048.7, labels.clone());

        let gauge = collector.get_gauge("memory_usage", &labels).unwrap();
        assert_eq!(gauge.value, 2048.7);
    }

    #[test]
    fn test_gauge_increment_decrement() {
        let collector = MetricsCollector::new();
        let labels = HashMap::new();

        collector.set_gauge("active_connections", 10.0, labels.clone());
        collector.increment_gauge("active_connections", 5.0, labels.clone());
        collector.decrement_gauge("active_connections", 3.0, labels.clone());

        let gauge = collector.get_gauge("active_connections", &labels).unwrap();
        assert_eq!(gauge.value, 12.0);
    }

    #[test]
    fn test_histogram_observation() {
        let collector = MetricsCollector::new();
        let labels = HashMap::new();

        collector.observe_histogram("request_duration", 0.5, labels.clone());
        collector.observe_histogram("request_duration", 1.5, labels.clone());
        collector.observe_histogram("request_duration", 5.0, labels.clone());

        let histogram = collector.get_histogram("request_duration", &labels).unwrap();
        assert_eq!(histogram.count, 3);
        assert_eq!(histogram.sum, 7.0);
    }

    #[test]
    fn test_histogram_percentile() {
        let collector = MetricsCollector::new();
        let labels = HashMap::new();

        // Add observations
        for value in [0.1, 0.2, 0.3, 0.5, 1.0, 2.0, 5.0, 10.0] {
            collector.observe_histogram("latency", value, labels.clone());
        }

        let histogram = collector.get_histogram("latency", &labels).unwrap();

        // Check percentiles
        assert!(histogram.percentile(50.0).is_some());
        assert!(histogram.percentile(95.0).is_some());
        assert!(histogram.percentile(99.0).is_some());
    }

    #[test]
    fn test_histogram_average() {
        let collector = MetricsCollector::new();
        let labels = HashMap::new();

        collector.observe_histogram("response_time", 1.0, labels.clone());
        collector.observe_histogram("response_time", 2.0, labels.clone());
        collector.observe_histogram("response_time", 3.0, labels.clone());

        let histogram = collector.get_histogram("response_time", &labels).unwrap();
        assert_eq!(histogram.average(), Some(2.0));
    }

    #[test]
    fn test_metric_labels() {
        let collector = MetricsCollector::new();

        let labels1 = HashMap::from([
            ("method".to_string(), "GET".to_string()),
            ("path".to_string(), "/api/runs".to_string()),
        ]);

        let labels2 = HashMap::from([
            ("method".to_string(), "POST".to_string()),
            ("path".to_string(), "/api/jobs".to_string()),
        ]);

        collector.increment_counter("http_requests", labels1.clone());
        collector.increment_counter("http_requests", labels2.clone());
        collector.increment_counter("http_requests", labels1.clone());

        let counter1 = collector.get_counter("http_requests", &labels1).unwrap();
        let counter2 = collector.get_counter("http_requests", &labels2).unwrap();

        assert_eq!(counter1.value, 2);
        assert_eq!(counter2.value, 1);
    }

    #[test]
    fn test_get_all_metrics() {
        let collector = MetricsCollector::new();
        let labels = HashMap::new();

        collector.increment_counter("counter1", labels.clone());
        collector.increment_counter("counter2", labels.clone());
        collector.set_gauge("gauge1", 10.0, labels.clone());
        collector.observe_histogram("histogram1", 1.0, labels.clone());

        assert_eq!(collector.get_counters().len(), 2);
        assert_eq!(collector.get_gauges().len(), 1);
        assert_eq!(collector.get_histograms().len(), 1);
    }

    #[test]
    fn test_reset() {
        let collector = MetricsCollector::new();
        let labels = HashMap::new();

        collector.increment_counter("test_counter", labels.clone());
        collector.set_gauge("test_gauge", 10.0, labels.clone());

        assert_eq!(collector.get_counters().len(), 1);
        assert_eq!(collector.get_gauges().len(), 1);

        collector.reset();

        assert_eq!(collector.get_counters().len(), 0);
        assert_eq!(collector.get_gauges().len(), 0);
    }
}
