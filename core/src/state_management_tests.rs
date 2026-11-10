// ss[related philosophy.structural-hierarchy]
use super::*;
// ss[related philosophy.structural-hierarchy]
use serde::{Deserialize, Serialize};
// ss[related state.lock-init-once]
use std::fs::File;
// ss[related philosophy.structural-hierarchy]
use std::io::BufReader;
// ss[related philosophy.structural-hierarchy]
use tempfile::tempdir;

// Define a simple state type for testing persistence
#[derive(Serialize, Deserialize, Debug, PartialEq, Clone)]
// ss[related state.lock-init-once]
struct MyState {
    value: i32,
}

// ss[verify state.lock-init-once]
// ss[verify state.steady-state-persistence]
// ss[verify state.try-lock-sync]
#[test]
// ss[related philosophy.structural-hierarchy]
fn test_basic_state() {
crate::core_exec::block_on(async {

    let state = new_state::<i32>();
    // Test that try_lock_sync fails before initialization
    assert!(state.try_lock_sync().is_none());
    {
        let guard = state.lock(|| 42).await;
        assert_eq!(*guard, 42);
    }
    {
        let guard = state.try_lock_sync().unwrap();
        assert_eq!(*guard, 42);
    }
    });
}

// ss[verify state.clone-shared]
#[test]
// ss[related philosophy.structural-hierarchy]
fn test_cloning_shared_state() {
crate::core_exec::block_on(async {

    let state1 = new_state::<i32>();
    {
        let guard = state1.lock(|| 10).await;
        assert_eq!(*guard, 10);
    }
    let state2 = state1.clone();
    {
        let mut guard = state2.lock(|| 0).await; // init closure shouldn't run
        *guard = 20;
    }
    {
        let guard = state1.lock(|| 0).await;
        assert_eq!(*guard, 20);
    }
    });
}

// ss[verify state.persistent-load]
#[test]
// ss[related philosophy.structural-hierarchy]
fn test_persistent_state_load() {
crate::core_exec::block_on(async {

    let dir = tempdir().unwrap();
    let file_path = dir.path().join("state.json");
    let initial_state = MyState { value: 100 };
    let file = File::create(&file_path).unwrap();
    serde_json::to_writer(file, &initial_state).unwrap();

    let state = new_persistent_state::<MyState, _>(&file_path);
    {
        let guard = state.lock(|| MyState { value: 0 }).await;
        assert_eq!(*guard, MyState { value: 100 });
    }
    });
}

// ss[verify state.save-on-drop]
// ss[verify state.on-drop-hook]
#[test]
// ss[related philosophy.structural-hierarchy]
fn test_persistent_state_save() {
crate::core_exec::block_on(async {

    let dir = tempdir().unwrap();
    let file_path = dir.path().join("state.json");

    let state = new_persistent_state::<MyState, _>(&file_path);
    {
        let mut guard = state.lock(|| MyState { value: 0 }).await;
        guard.value = 200;
    } // Guard dropped here, should save to file

    let file = File::open(&file_path).unwrap();
    let reader = BufReader::new(file);
    let saved_state: MyState = serde_json::from_reader(reader).unwrap();
    assert_eq!(saved_state, MyState { value: 200 });
    });
}

// ss[verify state.persistent-load]
#[test]
// ss[related philosophy.structural-hierarchy]
fn test_persistent_state_no_file() {
crate::core_exec::block_on(async {

    let dir = tempdir().unwrap();
    let file_path = dir.path().join("nonexistent.json");

    let state = new_persistent_state::<MyState, _>(&file_path);
    {
        let guard = state.lock(|| MyState { value: 50 }).await;
        assert_eq!(*guard, MyState { value: 50 });
    }
    });
}

// ss[verify state.persistent-load]
#[test]
// ss[related philosophy.structural-hierarchy]
fn test_persistent_state_invalid_file() {
crate::core_exec::block_on(async {

    let dir = tempdir().unwrap();
    let file_path = dir.path().join("invalid.json");
    std::fs::write(&file_path, "invalid json").unwrap();

    let state = new_persistent_state::<MyState, _>(&file_path);
    {
        let guard = state.lock(|| MyState { value: 75 }).await;
        assert_eq!(*guard, MyState { value: 75 });
    }
    });
}

// ss[related state.lock-init-once]
use proptest::prelude::*;
// ss[related philosophy.structural-hierarchy]
use proptest::test_runner::TestCaseError;
// ss[related philosophy.structural-hierarchy]
use serde::de::DeserializeOwned;

// ss[related state.lock-init-once]
fn assert_persistent_json_roundtrip<T>(value: T) -> Result<(), TestCaseError>
where
    T: Serialize + DeserializeOwned + PartialEq + std::fmt::Debug + Clone + Send + 'static,
{
    let dir = tempdir().expect("tempdir");
    let file_path = dir.path().join("state.json");
    let expected = value.clone();
    {
        let state = new_persistent_state::<T, _>(&file_path);
        crate::core_exec::block_on(async {
            let mut guard = state.lock(|| expected.clone()).await;
            *guard = expected.clone();
        });
    }
    let file = File::open(&file_path).expect("persisted file");
    let reader = BufReader::new(file);
    let loaded: T = serde_json::from_reader(reader).expect("valid json");
    prop_assert_eq!(loaded, expected);
    Ok(())
}

ss_proptest! {

    /// Property: persistent state round-trips arbitrary i32 values through JSON on disk.
    #[test]
    // ss[verify state.save-on-drop]
    // ss[verify state.persistent-load]
    // ss[verify verify.process.proptest]
    fn proptest_persistent_state_json_roundtrip_i32(value: i32) {
        assert_persistent_json_roundtrip(value)?;
    }

    /// Property: persistent state round-trips string payloads.
    #[test]
    // ss[verify state.save-on-drop]
    // ss[verify state.persistent-load]
    // ss[verify verify.process.proptest]
    fn proptest_persistent_state_json_roundtrip_string(s in "\\PC{0,48}") {
        assert_persistent_json_roundtrip(s)?;
    }

    /// Property: persistent state round-trips struct payloads.
    #[test]
    // ss[verify state.save-on-drop]
    // ss[verify state.persistent-load]
    // ss[verify verify.process.proptest]
    fn proptest_persistent_state_json_roundtrip_struct(value: i32, label in "\\PC{0,16}") {
        #[derive(Serialize, Deserialize, Debug, PartialEq, Clone)]
        // ss[related philosophy.structural-hierarchy]
        struct Labeled {
            value: i32,
            label: String,
        }
        assert_persistent_json_roundtrip(Labeled { value, label })?;
    }

    /// Property: persistent state round-trips vector payloads.
    #[test]
    // ss[verify state.save-on-drop]
    // ss[verify state.persistent-load]
    // ss[verify verify.process.proptest]
    fn proptest_persistent_state_json_roundtrip_vec(
        items in prop::collection::vec(-1_000i32..1_000, 0..16),
    ) {
        assert_persistent_json_roundtrip(items)?;
    }

    /// Property: corrupt on-disk JSON falls back to the init closure.
    #[test]
    // ss[verify state.persistent-load]
    // ss[verify verify.process.proptest]
    fn proptest_persistent_state_invalid_json_fallback(
        init_value in -1_000i32..1_000,
        garbage in "\\PC{1,64}",
    ) {
        let dir = tempdir().expect("tempdir");
        let file_path = dir.path().join("state.json");
        std::fs::write(&file_path, garbage).expect("write garbage");
        let state = new_persistent_state::<MyState, _>(&file_path);
        let got = crate::core_exec::block_on(async {
            let guard = state.lock(|| MyState { value: init_value }).await;
            guard.value
        });
        prop_assert_eq!(got, init_value);
    }

    /// Property: `persist` on non-persistent state is a no-op success.
    #[test]
    // ss[verify state.lock-init-once]
    // ss[verify verify.process.proptest]
    fn proptest_non_persistent_persist_is_noop(value: i32) {
        let state = new_state::<MyState>();
        crate::core_exec::block_on(async {
            let mut guard = state.lock(|| MyState { value: 0 }).await;
            guard.value = value;
            guard.persist().await.expect("persist noop");
        });
    }

    /// Property: structurally valid JSON with wrong shape still falls back to init.
    #[test]
    // ss[verify state.persistent-load]
    // ss[verify verify.process.proptest]
    fn proptest_persistent_state_wrong_shape_fallback(
        init_value in -500i32..500,
        extra in "\\PC{0,24}",
    ) {
        let dir = tempdir().expect("tempdir");
        let file_path = dir.path().join("state.json");
        let body = format!(r#"{{"not_value":{init_value},"label":"{extra}"}}"#);
        std::fs::write(&file_path, body).expect("write wrong shape");
        let state = new_persistent_state::<MyState, _>(&file_path);
        let got = crate::core_exec::block_on(async {
            let guard = state.lock(|| MyState { value: init_value }).await;
            guard.value
        });
        prop_assert_eq!(got, init_value);
    }

    /// Property: clone shares mutations across handles.
    #[test]
    // ss[verify state.clone-shared]
    // ss[verify verify.process.proptest]
    fn proptest_cloned_state_shares_value(a: i32, b: i32) {
        let state1 = new_state::<i32>();
        crate::core_exec::block_on(async {
            let _ = state1.lock(|| a).await;
        });
        let state2 = state1.clone();
        crate::core_exec::block_on(async {
            let mut guard = state2.lock(|| 0).await;
            *guard = b;
        });
        let read = crate::core_exec::block_on(async { *state1.lock(|| 0).await });
        prop_assert_eq!(read, b);
    }

    /// Property: empty file falls back to init closure.
    #[test]
    // ss[verify state.persistent-load]
    // ss[verify verify.process.proptest]
    fn proptest_persistent_state_empty_file_fallback(init_value in -1_000i32..1_000) {
        let dir = tempdir().expect("tempdir");
        let file_path = dir.path().join("state.json");
        std::fs::write(&file_path, "").expect("write empty");
        let state = new_persistent_state::<MyState, _>(&file_path);
        let got = crate::core_exec::block_on(async {
            let guard = state.lock(|| MyState { value: init_value }).await;
            guard.value
        });
        prop_assert_eq!(got, init_value);
    }

    /// Property: truncated JSON falls back to init closure.
    #[test]
    // ss[verify state.persistent-load]
    // ss[verify verify.process.proptest]
    fn proptest_persistent_state_truncated_json_fallback(
        init_value in -500i32..500,
        prefix in "\\PC{1,32}",
    ) {
        let dir = tempdir().expect("tempdir");
        let file_path = dir.path().join("state.json");
        let body = format!(r#"{{"value":{init_value},"extra":"{prefix}"#);
        std::fs::write(&file_path, body).expect("write truncated");
        let state = new_persistent_state::<MyState, _>(&file_path);
        let got = crate::core_exec::block_on(async {
            let guard = state.lock(|| MyState { value: init_value }).await;
            guard.value
        });
        prop_assert_eq!(got, init_value);
    }

    /// Property: valid on-disk JSON loads without calling init.
    #[test]
    // ss[verify state.persistent-load]
    // ss[verify verify.process.proptest]
    fn proptest_persistent_state_valid_json_loads(
        disk_value in -10_000i32..10_000,
        init_value in -10_000i32..10_000,
    ) {
        let dir = tempdir().expect("tempdir");
        let file_path = dir.path().join("state.json");
        let on_disk = MyState { value: disk_value };
        let file = File::create(&file_path).expect("create");
        serde_json::to_writer(file, &on_disk).expect("write");
        let state = new_persistent_state::<MyState, _>(&file_path);
        let got = crate::core_exec::block_on(async {
            let guard = state.lock(|| MyState { value: init_value }).await;
            guard.value
        });
        prop_assert_eq!(got, disk_value);
    }

    /// Property: explicit `persist` writes current state to disk.
    #[test]
    // ss[verify state.save-on-drop]
    // ss[verify verify.process.proptest]
    fn proptest_persistent_state_explicit_persist(
        value in -5_000i32..5_000,
    ) {
        let dir = tempdir().expect("tempdir");
        let file_path = dir.path().join("state.json");
        let state = new_persistent_state::<MyState, _>(&file_path);
        crate::core_exec::block_on(async {
            let mut guard = state.lock(|| MyState { value: 0 }).await;
            guard.value = value;
            guard.persist().await.expect("persist");
        });
        let file = File::open(&file_path).expect("open");
        let reader = BufReader::new(file);
        let loaded: MyState = serde_json::from_reader(reader).expect("json");
        prop_assert_eq!(loaded.value, value);
    }

    /// Property: `try_lock_sync` returns None before initialization.
    #[test]
    // ss[verify state.try-lock-sync]
    // ss[verify verify.process.proptest]
    fn proptest_try_lock_sync_before_init(_seed in 0u8..2) {
        let state = new_state::<MyState>();
        prop_assert!(state.try_lock_sync().is_none());
    }

    /// Property: JSON with the wrong scalar type falls back to the init closure.
    #[test]
    // ss[verify state.persistent-load]
    // ss[verify verify.process.proptest]
    fn proptest_persistent_state_json_type_mismatch_fallback(
        init_value in -1_000i32..1_000,
        label in "\\PC{0,16}",
    ) {
        let dir = tempdir().expect("tempdir");
        let file_path = dir.path().join("state.json");
        let body = format!(r#"{{"value":"{label}"}}"#);
        std::fs::write(&file_path, body).expect("write mismatched type");
        let state = new_persistent_state::<MyState, _>(&file_path);
        let got = crate::core_exec::block_on(async {
            let guard = state.lock(|| MyState { value: init_value }).await;
            guard.value
        });
        prop_assert_eq!(got, init_value);
    }

    /// Property: JSON `null` document falls back to the init closure.
    #[test]
    // ss[verify state.persistent-load]
    // ss[verify verify.process.proptest]
    fn proptest_persistent_state_json_null_fallback(init_value in -1_000i32..1_000) {
        let dir = tempdir().expect("tempdir");
        let file_path = dir.path().join("state.json");
        std::fs::write(&file_path, "null").expect("write null");
        let state = new_persistent_state::<MyState, _>(&file_path);
        let got = crate::core_exec::block_on(async {
            let guard = state.lock(|| MyState { value: init_value }).await;
            guard.value
        });
        prop_assert_eq!(got, init_value);
    }

    /// Property: JSON array document falls back to the init closure.
    #[test]
    // ss[verify state.persistent-load]
    // ss[verify verify.process.proptest]
    fn proptest_persistent_state_json_array_fallback(
        init_value in -500i32..500,
        extra in -500i32..500,
    ) {
        let dir = tempdir().expect("tempdir");
        let file_path = dir.path().join("state.json");
        let body = format!(r#"[{{"value":{init_value}}},{{"value":{extra}}}]"#);
        std::fs::write(&file_path, body).expect("write array");
        let state = new_persistent_state::<MyState, _>(&file_path);
        let got = crate::core_exec::block_on(async {
            let guard = state.lock(|| MyState { value: init_value }).await;
            guard.value
        });
        prop_assert_eq!(got, init_value);
    }

    /// Property: persisting to a directory path returns an I/O error.
    #[test]
    // ss[verify state.save-on-drop]
    // ss[verify verify.process.proptest]
    fn proptest_persistent_state_persist_directory_fails(value in -500i32..500) {
        let dir = tempdir().expect("tempdir");
        let state = new_persistent_state::<MyState, _>(dir.path());
        let err = crate::core_exec::block_on(async {
            let mut guard = state.lock(|| MyState { value: 0 }).await;
            guard.value = value;
            guard.persist().await
        });
        prop_assert!(err.is_err());
    }

    /// Property: JSON missing required fields falls back to init.
    #[test]
    // ss[verify state.persistent-load]
    // ss[verify verify.process.proptest]
    fn proptest_persistent_state_missing_field_fallback(init_value in -1_000i32..1_000) {
        let dir = tempdir().expect("tempdir");
        let file_path = dir.path().join("state.json");
        std::fs::write(&file_path, r#"{}"#).expect("write empty object");
        let state = new_persistent_state::<MyState, _>(&file_path);
        let got = crate::core_exec::block_on(async {
            let guard = state.lock(|| MyState { value: init_value }).await;
            guard.value
        });
        prop_assert_eq!(got, init_value);
    }

    /// Property: JSON boolean where integer expected falls back to init.
    #[test]
    // ss[verify state.persistent-load]
    // ss[verify verify.process.proptest]
    fn proptest_persistent_state_boolean_type_fallback(init_value in -500i32..500) {
        let dir = tempdir().expect("tempdir");
        let file_path = dir.path().join("state.json");
        std::fs::write(&file_path, r#"{"value":true}"#).expect("write bool");
        let state = new_persistent_state::<MyState, _>(&file_path);
        let got = crate::core_exec::block_on(async {
            let guard = state.lock(|| MyState { value: init_value }).await;
            guard.value
        });
        prop_assert_eq!(got, init_value);
    }

    /// Property: JSON float where integer expected falls back to init.
    #[test]
    // ss[verify state.persistent-load]
    // ss[verify verify.process.proptest]
    fn proptest_persistent_state_float_type_fallback(
        init_value in -500i32..500,
        fractional in 0.1f64..99.9,
    ) {
        let dir = tempdir().expect("tempdir");
        let file_path = dir.path().join("state.json");
        let body = format!(r#"{{"value":{fractional}}}"#);
        std::fs::write(&file_path, body).expect("write float");
        let state = new_persistent_state::<MyState, _>(&file_path);
        let got = crate::core_exec::block_on(async {
            let guard = state.lock(|| MyState { value: init_value }).await;
            guard.value
        });
        prop_assert_eq!(got, init_value);
    }

    /// Property: JSON numeric string where integer expected falls back to init.
    #[test]
    // ss[verify state.persistent-load]
    // ss[verify verify.process.proptest]
    fn proptest_persistent_state_numeric_string_fallback(
        init_value in -1_000i32..1_000,
        disk_digits in "\\d{1,6}",
    ) {
        let dir = tempdir().expect("tempdir");
        let file_path = dir.path().join("state.json");
        let body = format!(r#"{{"value":"{disk_digits}"}}"#);
        std::fs::write(&file_path, body).expect("write numeric string");
        let state = new_persistent_state::<MyState, _>(&file_path);
        let got = crate::core_exec::block_on(async {
            let guard = state.lock(|| MyState { value: init_value }).await;
            guard.value
        });
        prop_assert_eq!(got, init_value);
    }

    /// Property: JSON with extra unknown fields still loads valid `MyState` when shape matches.
    #[test]
    // ss[verify state.persistent-load]
    // ss[verify verify.process.proptest]
    fn proptest_persistent_state_extra_fields_ignored(
        disk_value in -10_000i32..10_000,
        extra in "\\PC{0,24}",
    ) {
        let dir = tempdir().expect("tempdir");
        let file_path = dir.path().join("state.json");
        let body = serde_json::json!({
            "value": disk_value,
            "extra": extra,
        })
        .to_string();
        std::fs::write(&file_path, body).expect("write extra fields");
        let state = new_persistent_state::<MyState, _>(&file_path);
        let got = crate::core_exec::block_on(async {
            let guard = state.lock(|| MyState { value: 0 }).await;
            guard.value
        });
        prop_assert_eq!(got, disk_value);
    }
}

// ss[verify state.persist-hooks]
// ss[verify state.persist-before-park]
#[test]
fn test_persistent_state_with_in_memory_hook() {
    crate::core_exec::block_on(async {
        let store = Arc::new(parking_lot::Mutex::new(None::<MyState>));
        let store_load = store.clone();
        let store_persist = store.clone();
        let state = new_persistent_state_with(
            move || store_load.lock().clone(),
            move |s| {
                *store_persist.lock() = Some(MyState { value: s.value });
                Ok(())
            },
        );
        {
            let mut guard = state.lock(|| MyState { value: 0 }).await;
            guard.value = 42;
            assert!(guard.is_dirty());
            guard.persist().await.expect("persist");
            assert!(!guard.is_dirty());
        }
        drop(state);
        assert_eq!(store.lock().as_ref().map(|s| s.value), Some(42));

        let store2 = store.clone();
        let state2 = new_persistent_state_with(
            move || store2.lock().clone(),
            |_| Ok(()),
        );
        let guard = state2.lock(|| MyState { value: 0 }).await;
        assert_eq!(guard.value, 42);
    });
}

// ss[verify state.dirty-at-park]
// ss[verify state.persist-before-park]
#[test]
fn test_dirty_at_park_panics_under_strict() {
    set_thread_strict_persist(true);
    let result = std::panic::catch_unwind(|| {
        crate::core_exec::block_on(async {
            let store = Arc::new(parking_lot::Mutex::new(None::<MyState>));
            let store_p = store.clone();
            let state = new_persistent_state_with(
                || None,
                move |s: &MyState| {
                    *store_p.lock() = Some(MyState { value: s.value });
                    Ok(())
                },
            );
            let mut guard = state.lock(|| MyState { value: 0 }).await;
            guard.value = 99;
            // Intentionally skip persist — next park check must fail.
            check_dirty_at_park();
        });
    });
    set_thread_strict_persist(false);
    assert!(result.is_err(), "expected dirty-at-park panic");
}

// ss[verify state.dirty-at-park]
#[test]
fn test_new_state_never_dirty_for_strict() {
    set_thread_strict_persist(true);
    crate::core_exec::block_on(async {
        let state = new_state::<i32>();
        let mut guard = state.lock(|| 0).await;
        *guard = 7;
        // No on_persist → not watched → check must not panic.
        check_dirty_at_park();
    });
    set_thread_strict_persist(false);
}
