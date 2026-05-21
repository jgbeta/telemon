use std::collections::BTreeSet;

use crate::metrics::model::MetricSample;

pub fn encode(samples: &[MetricSample]) -> String {
    let mut sorted = samples.to_vec();
    sorted.sort_by(|left, right| {
        left.name
            .cmp(&right.name)
            .then_with(|| left.labels.cmp(&right.labels))
            .then_with(|| left.value.total_cmp(&right.value))
    });

    let mut emitted_headers = BTreeSet::new();
    let mut output = String::new();

    for sample in sorted {
        if emitted_headers.insert(sample.name.clone()) {
            output.push_str("# HELP ");
            output.push_str(&sample.name);
            output.push(' ');
            output.push_str(&escape_help(&sample.help));
            output.push('\n');
            output.push_str("# TYPE ");
            output.push_str(&sample.name);
            output.push(' ');
            output.push_str(sample.kind.as_str());
            output.push('\n');
        }

        output.push_str(&sample.name);
        if !sample.labels.is_empty() {
            output.push('{');
            for (index, (key, value)) in sample.labels.iter().enumerate() {
                if index > 0 {
                    output.push(',');
                }
                output.push_str(key);
                output.push_str("=\"");
                output.push_str(&escape_label_value(value));
                output.push('"');
            }
            output.push('}');
        }
        output.push(' ');
        output.push_str(&format_value(sample.value));
        output.push('\n');
    }

    output
}

fn escape_help(value: &str) -> String {
    value.replace('\\', "\\\\").replace('\n', "\\n")
}

fn escape_label_value(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('\n', "\\n")
        .replace('"', "\\\"")
}

fn format_value(value: f64) -> String {
    if value.is_finite() && value.fract() == 0.0 {
        format!("{value:.0}")
    } else {
        value.to_string()
    }
}

#[cfg(test)]
mod tests {
    use crate::metrics::model::{labels, MetricSample};

    use super::*;

    #[test]
    fn encodes_metric_without_labels() {
        let sample = MetricSample::gauge("telemon_test_gauge", "A test gauge.", labels(&[]), 1.0);

        assert_eq!(
            encode(&[sample]),
            "# HELP telemon_test_gauge A test gauge.\n# TYPE telemon_test_gauge gauge\ntelemon_test_gauge 1\n"
        );
    }

    #[test]
    fn encodes_metric_with_labels() {
        let sample = MetricSample::counter(
            "telemon_test_total",
            "A test counter.",
            labels(&[("collector", "example")]),
            2.0,
        );

        assert!(encode(&[sample]).contains("telemon_test_total{collector=\"example\"} 2\n"));
    }

    #[test]
    fn escapes_label_values() {
        let sample = MetricSample::gauge(
            "telemon_test_gauge",
            "A test gauge.",
            labels(&[("value", "a\\b\"c\nd")]),
            1.0,
        );

        assert!(encode(&[sample]).contains("value=\"a\\\\b\\\"c\\nd\""));
    }

    #[test]
    fn output_is_deterministic() {
        let first = MetricSample::gauge("telemon_b", "B.", labels(&[]), 1.0);
        let second = MetricSample::gauge("telemon_a", "A.", labels(&[]), 1.0);

        assert_eq!(
            encode(&[first.clone(), second.clone()]),
            encode(&[second, first])
        );
    }
}
