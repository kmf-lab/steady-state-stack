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
use std::cell::RefCell;

#[derive(Clone,Eq, PartialEq, Debug)]
pub struct Eye {
    pub expression: &'static str,
    pub file: &'static str,
    pub line: u32
}

impl Eye {

    pub(crate) fn veto_reason(&self) -> String {
        format!("{}:{}  {}",self.file, self.line, self.expression).to_string()
    }

}

thread_local! {
    /// Thread-local storage for the last expression identifier that evaluated to `false`.
    pub static LAST_FALSE: RefCell<Option<Eye>> = const { RefCell::new(None) };
}

/// Wraps a boolean expression and logs its identifier if it evaluates to `false`.
///
/// The macro evaluates the provided expression. If the result is `false`, the stringified form of
/// the expression (a `'static` string) is stored in thread-local storage. This storage can later
/// be retrieved and cleared using `i_take_last_false`.
///
#[macro_export]
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
pub fn i_take_expression() -> Option<Eye> {
    LAST_FALSE.with(|cell| {
        let mut borrowed = cell.borrow_mut();
        borrowed.take()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Tests that a `true` expression does not store anything.
    #[test]
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


    /// Tests a chain of all `true` expressions.
    #[test]
    fn test_all_true_expressions() {
        let result = i!(true) && i!(true) && i!(true);
        assert!(result, "Result should be true");
        assert_eq!(
            i_take_expression(),
            None,
            "No identifier should be stored when all are true"
        );
    }

    /// Tests multiple `false` expressions, ensuring the last one is stored.
    #[test]
    fn test_multiple_false_expressions() {
        let condition1 = false;
        let condition2 = false;

        let result = i!(condition1) && i!(condition2);
        assert!(!result, "Result should be false");
        assert_eq!(
            i_take_expression().expect("").expression,
            "condition1",
            "condition2 should be stored as the last false"
        );
        assert_eq!(
            i_take_expression(),
            None,
            "Storage should be cleared after reading"
        );
    }
}