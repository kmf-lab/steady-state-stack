// ss[related telemetry.dot-export]
use std::hash::Hash;

#[derive(Hash, Eq, PartialEq, Debug, Clone, PartialOrd, Ord)]
// ss[related telemetry.dot-export]
pub(crate) struct PrimaryGroupKey {
    // ss[impl telemetry.dot-export]
    pub(crate) from_name: Option<&'static str>,
    // ss[impl telemetry.dot-export]
    pub(crate) from_suffix: Option<usize>,
    // ss[impl telemetry.dot-export]
    pub(crate) to_name: Option<&'static str>,
    // ss[impl telemetry.dot-export]
    pub(crate) to_suffix: Option<usize>,
    // ss[impl telemetry.dot-export]
    pub(crate) sub_capacities: Vec<usize>,
    // ss[impl telemetry.dot-export]
    pub(crate) type_name: String,
    // ss[impl telemetry.dot-export]
    pub(crate) sidecar: bool,
    // ss[impl telemetry.dot-export]
    pub(crate) partner: Option<&'static str>,
}

#[derive(PartialEq, Eq, PartialOrd, Ord)]
// ss[related telemetry.dot-export]
pub(crate) struct PartnerKey {
    // ss[impl telemetry.dot-export]
    pub(crate) from: Option<(&'static str, Option<usize>)>,
    // ss[impl telemetry.dot-export]
    pub(crate) to: Option<(&'static str, Option<usize>)>,
    // ss[impl telemetry.dot-export]
    pub(crate) partner: Option<&'static str>,
    // ss[impl telemetry.dot-export]
    pub(crate) bundle_index: Option<usize>,
    /// Only used if partner is None to keep edges separate.
    // ss[impl telemetry.dot-export]
    pub(crate) edge_id: Option<usize>,
}
