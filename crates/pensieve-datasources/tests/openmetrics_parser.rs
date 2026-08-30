use kyma_datasources::prometheus::parser::parse_openmetrics;

#[test]
fn parses_counter_and_gauge() {
    let text = "\
# HELP http_requests_total The total.
# TYPE http_requests_total counter
http_requests_total{method=\"GET\",code=\"200\"} 42
http_requests_total{method=\"POST\",code=\"500\"} 3
# HELP queue_depth Current depth.
# TYPE queue_depth gauge
queue_depth 17.5
";
    let samples = parse_openmetrics(text).expect("parse");
    assert_eq!(samples.len(), 3);

    let first = &samples[0];
    assert_eq!(first.name, "http_requests_total");
    assert_eq!(first.value, Some(42.0));
    assert_eq!(first.labels.get("method"), Some(&"GET".to_string()));
    assert_eq!(first.labels.get("code"), Some(&"200".to_string()));

    let gauge = samples.iter().find(|s| s.name == "queue_depth").unwrap();
    assert_eq!(gauge.value, Some(17.5));
    assert!(gauge.labels.is_empty());
}

#[test]
fn parses_histogram_exploded() {
    let text = "\
# TYPE request_duration_seconds histogram
request_duration_seconds_bucket{le=\"0.1\"} 10
request_duration_seconds_bucket{le=\"0.5\"} 20
request_duration_seconds_bucket{le=\"+Inf\"} 25
request_duration_seconds_sum 3.14
request_duration_seconds_count 25
";
    let samples = parse_openmetrics(text).expect("parse");
    assert_eq!(samples.len(), 5);
    let buckets: Vec<_> = samples
        .iter()
        .filter(|s| s.name == "request_duration_seconds_bucket")
        .collect();
    assert_eq!(buckets.len(), 3);
    assert_eq!(buckets[2].labels.get("le"), Some(&"+Inf".to_string()));
}

#[test]
fn special_float_values_become_null() {
    let text = "a_metric 1.0\nb_metric NaN\nc_metric +Inf\nd_metric -Inf\n";
    let samples = parse_openmetrics(text).expect("parse");
    assert_eq!(samples.len(), 4);
    assert_eq!(samples[0].value, Some(1.0));
    assert_eq!(samples[1].value, None, "NaN → None");
    assert_eq!(samples[2].value, None, "+Inf → None");
    assert_eq!(samples[3].value, None, "-Inf → None");
}

#[test]
fn comments_and_blank_lines_skipped() {
    let text = "\n# some comment\n\n# HELP x y\nx 5\n";
    let samples = parse_openmetrics(text).expect("parse");
    assert_eq!(samples.len(), 1);
    assert_eq!(samples[0].name, "x");
}

#[test]
fn malformed_line_errors() {
    let text = "garbled_no_value{a=\"b\"}\n";
    let err = parse_openmetrics(text).unwrap_err();
    assert!(err.to_string().contains("line 1"), "error should cite line");
}

#[test]
fn escaped_label_values() {
    let text = "m{a=\"b\\\"c\",d=\"e\\\\f\"} 1\n";
    let samples = parse_openmetrics(text).expect("parse");
    assert_eq!(samples[0].labels.get("a"), Some(&"b\"c".to_string()));
    assert_eq!(samples[0].labels.get("d"), Some(&"e\\f".to_string()));
}

#[test]
fn ignores_trailing_timestamp() {
    let t = "m{l=\"v\"} 1.5 1609459200000\nn 2.5 1609459200000\n";
    let s = parse_openmetrics(t).expect("parse");
    assert_eq!(s.len(), 2);
    assert_eq!(s[0].value, Some(1.5));
    assert_eq!(s[0].labels.get("l"), Some(&"v".to_string()));
    assert_eq!(s[1].name, "n");
    assert_eq!(s[1].value, Some(2.5));
}

#[test]
fn preserves_utf8_label_values() {
    let t = "m{region=\"München\",city=\"東京\"} 1\n";
    let s = parse_openmetrics(t).expect("parse");
    assert_eq!(s[0].labels.get("region"), Some(&"München".to_string()));
    assert_eq!(s[0].labels.get("city"), Some(&"東京".to_string()));
}
