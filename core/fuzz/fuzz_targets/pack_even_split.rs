#![no_main]

//! Coverage-guided fuzz of even-split packing (no OS threads).

use arbitrary::Arbitrary;
use libfuzzer_sys::fuzz_target;
use steady_state::fuzz_export::{even_split, pack_actors};

#[derive(Arbitrary, Debug)]
struct FuzzPack {
    ids: Vec<u8>,
    slots: u8,
}

fuzz_target!(|input: FuzzPack| {
    let mut ids: Vec<usize> = input.ids.into_iter().map(usize::from).take(32).collect();
    ids.sort_unstable();
    ids.dedup();
    let slots = (usize::from(input.slots) % 16) + 1;

    let clusters = even_split(&ids, slots);
    if ids.is_empty() {
        assert!(clusters.is_empty());
    } else {
        assert!(clusters.len() <= slots.min(ids.len()));
        let mut flat: Vec<usize> = clusters.iter().flatten().copied().collect();
        flat.sort_unstable();
        assert_eq!(flat, ids, "even_split must cover each actor once");
    }

    let packed = pack_actors(&ids, slots);
    assert_eq!(
        packed,
        even_split(&ids, slots),
        "empty incidence must fall back to even_split"
    );
});
