//! # Debug Macro Module for Boolean Expressions
//!
//! This module provides the `i!` macro, designed to wrap boolean expressions and assist in debugging
//! by identifying which expression evaluated to `false`. It is particularly useful in scenarios like
//! checking conditions for actor shutdowns in frameworks such as `steady_state`. The macro stores
//! the identifier of the expression that caused a `false` result in thread-local storage, and reading
//! this value is destructive—meaning the storage is cleared after retrieval.
//!
//! ## Features
//!
//! - **Macro `i!`**: Evaluates a boolean expression and stores its identifier if it evaluates to `false`.
//! - **Thread-Local Storage**: Uses thread-local storage to track the last `false` expression per thread.
//! - **Destructive Read**: Reading the stored identifier clears it from storage, preparing it for the next use.
//! - **Efficient String Handling**: Uses `'static` strings for identifiers to avoid runtime allocations.
//!
// ss[related telemetry.dot-export]
use std::cell::RefCell;

/// Internal structure holding expression location information in case it is necessary to debug unclean shutdowns.
#[derive(Clone,Eq, PartialEq, Debug)]
// ss[related telemetry.dot-export]
pub struct Eye {
    /// the str expression in question
    pub expression: &'static str,
    /// the file where this expression is found
    pub file: &'static str,
    /// the line number in the file where this expression is found
    pub line: u32
}

// ss[related telemetry.dot-export]
impl Eye {

    // ss[related philosophy.structural-hierarchy]
    pub(crate) fn veto_reason(&self) -> String {
        format!("{}:{}  {}",self.file, self.line, self.expression).to_string()
    }

}

thread_local! {
    /// Thread-local storage for the last expression identifier that evaluated to `false`.
    // ss[related telemetry.dot-export]
    pub static LAST_FALSE: RefCell<Option<Eye>> = const { RefCell::new(None) };
}

/// Wraps a boolean expression and logs its identifier if it evaluates to `false`.
///
/// The macro evaluates the provided expression. If the result is `false`, the stringified form of
/// the expression (a `'static` string) is stored in thread-local storage. This storage can later
/// be retrieved and cleared using `i_take_last_false`.
///
#[macro_export]
// ss[related telemetry.dot-export]
macro_rules! i {
    ($e:expr) => {{
        let result = $e;
        if !result {
            $crate::LAST_FALSE.with(|cell| {
                *cell.borrow_mut() = Some(crate::expression_steady_eye::Eye{expression: stringify!($e), file: file!(), line: line!()  });
            });
        }
        result
    }};
}

/// Retrieves and takes ownership of the expression identifier that evaluated to `false`.
///
/// This function returns the stored identifier (if any) and clears the thread-local storage,
/// ensuring it is empty for the next use. The returned value is an `Option<&'static str>`, where
/// `None` indicates no `false` expression has been recorded since the last read.
///
/// # Returns
///
/// - `Some(&'static str)`: The identifier of the last expression that evaluated to `false`.
/// - `None`: If no `false` expression has been recorded since the last read.
///
// ss[related telemetry.dot-export]
pub fn i_take_expression() -> Option<Eye> {
    LAST_FALSE.with(|cell| {
        let mut borrowed = cell.borrow_mut();
        borrowed.take()
    })
}

#[cfg(test)]
// ss[related telemetry.dot-export]
mod tests {
    // ss[related philosophy.structural-hierarchy]
    use super::*;

    /// Tests that a `true` expression does not store anything.
    #[test]
    // ss[verify telemetry.dot-export]
    fn test_true_expression() {
        let result = i!(true);
        assert!(result, "Expression should evaluate to true");
        assert_eq!(
            i_take_expression(),
            None,
            "No identifier should be stored for true"
        );
    }

    /// Tests that a `false` expression stores its identifier and clears it on read.
    #[test]
    // ss[verify telemetry.dot-export]
    fn test_false_expression() {
        let result = i!(false);
        assert!(!result, "Expression should evaluate to false");
        assert_eq!(
            i_take_expression().expect("").expression,
            "false",
            "Identifier should be stored"
        );
        assert_eq!(
            i_take_expression(),
            None,
            "Storage should be cleared after reading"
        );
    }

    /// `Eye::veto_reason` includes file, line, and expression text.
    #[test]
    // ss[verify telemetry.dot-export]
    fn veto_reason_includes_location_and_expression() {
        let eye = Eye {
            expression: "channel.is_closed()",
            file: "actor.rs",
            line: 99,
        };
        let reason = eye.veto_reason();
        assert!(reason.contains("actor.rs:99"));
        assert!(reason.contains("channel.is_closed()"));
    }

    // ss[related telemetry.dot-export]
    use proptest::prelude::*;

    ss_proptest! {

        /// Property: i!(b) stores expression iff b is false; read clears storage.
        #[test]
        // ss[verify telemetry.dot-export]
        // ss[verify verify.process.proptest]
        fn proptest_i_macro_records_only_false(value in any::<bool>()) {
            let _ = i_take_expression();
            let result = i!(value);
            prop_assert_eq!(result, value);
            let stored = i_take_expression();
            if value {
                prop_assert!(stored.is_none());
            } else {
                prop_assert!(stored.is_some());
            }
            prop_assert!(i_take_expression().is_none());
        }

        /// Property: short-circuit && stores the first false operand, not later ones.
        #[test]
        // ss[verify telemetry.dot-export]
        // ss[verify verify.process.proptest]
        fn proptest_i_macro_short_circuit_chain(
            a in any::<bool>(),
            b in any::<bool>(),
            c in any::<bool>(),
        ) {
            let _ = i_take_expression();
            let result = i!(a) && i!(b) && i!(c);
            prop_assert_eq!(result, a && b && c);
            if result {
                prop_assert!(i_take_expression().is_none());
            } else if !a {
                let eye = i_take_expression().expect("stored");
                prop_assert_eq!(eye.expression, "a");
            } else if !b {
                let eye = i_take_expression().expect("stored");
                prop_assert_eq!(eye.expression, "b");
            } else {
                let eye = i_take_expression().expect("stored");
                prop_assert_eq!(eye.expression, "c");
            }
        }
    }

    /// Complex expression text is captured verbatim by `i!`.
    #[test]
    // ss[verify telemetry.dot-export]
    fn false_complex_expression_stored() {
        let flag = false;
        let _ = i!(flag && true);
        let eye = i_take_expression().expect("stored");
        assert!(eye.expression.contains("flag"));
    }

    /// Thread-local `LAST_FALSE` is isolated per thread.
    #[test]
    // ss[verify telemetry.dot-export]
    fn i_macro_thread_local_isolation() {
        let _ = i_take_expression();
        let child_stored = std::thread::spawn(|| {
            let _ = i!(false);
            i_take_expression().is_some()
        })
        .join()
        .expect("child join");
        assert!(child_stored);
        assert!(i_take_expression().is_none(), "parent thread unaffected");
    }
}