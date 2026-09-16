#![no_main]

//! Coverage-guided fuzz of cgroup v1/v2 CPU quota parsers.

use libfuzzer_sys::fuzz_target;
use steady_state::fuzz_export::{parse_cgroup_v1_quota_us, parse_cgroup_v2_quota};

fn split_pair(s: &str) -> (&str, &str) {
    let mut parts = s.split_whitespace();
    let quota = parts.next().unwrap_or("");
    let period = parts.next().unwrap_or("");
    (quota, period)
}

fn assert_positive(label: &str, n: Option<usize>) {
    if let Some(cores) = n {
        assert!(cores >= 1, "{label} returned {cores}");
    }
}

fuzz_target!(|data: &[u8]| {
    let slice = if data.len() > 64 { &data[..64] } else { data };
    let s = String::from_utf8_lossy(slice);
    let (quota, period) = split_pair(&s);

    assert_positive("cgroup v2", parse_cgroup_v2_quota(quota, period));
    assert_positive("cgroup v1", parse_cgroup_v1_quota_us(quota, period));

    // Also probe a second pairing with the raw string as both sides.
    assert_positive("cgroup v2 same", parse_cgroup_v2_quota(&s, &s));
    assert_positive("cgroup v1 same", parse_cgroup_v1_quota_us(&s, &s));
});
