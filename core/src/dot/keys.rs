// ss[related telemetry.dot-export]
use std::hash::Hash;

#[derive(Hash, Eq, PartialEq, Debug, Clone, PartialOrd, Ord)]
pub(crate) struct PrimaryGroupKey {
    pub(crate) from_name: Option<&'static str>,
    pub(crate) from_suffix: Option<usize>,
    pub(crate) to_name: Option<&'static str>,
    pub(crate) to_suffix: Option<usize>,
    pub(crate) sub_capacities: Vec<usize>,
    pub(crate) type_name: String,
    pub(crate) sidecar: bool,
    pub(crate) partner: Option<&'static str>,
}

#[derive(PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct PartnerKey {
    pub(crate) from: Option<(&'static str, Option<usize>)>,
    pub(crate) to: Option<(&'static str, Option<usize>)>,
    pub(crate) partner: Option<&'static str>,
    pub(crate) bundle_index: Option<usize>,
    /// Only used if partner is None to keep edges separate.
    pub(crate) edge_id: Option<usize>,
}
