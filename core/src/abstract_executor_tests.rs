#[cfg(test)]
// ss[related platform.executor-features]
mod tests {
    use std::thread;
    use std::time::Duration;
    // ss[related platform.executor-features]
    use crate::*;

    // ss[verify platform.executor-features]
    #[test]
    fn test_init_without_driver() {
        let config = ProactorConfig::InterruptDriven;
        core_exec::init(false, config, 256);
    }

    #[test]
    // ss[verify platform.executor-features]
    fn test_init_with_driver() {
        let config = ProactorConfig::InterruptDriven;
        core_exec::init(true, config, 256);
        thread::sleep(Duration::from_millis(100));
    }

    #[test]
    // ss[verify platform.executor-features]
    fn test_block_on() {
        let future = async { 42 };
        let result = core_exec::block_on(future);
        assert_eq!(result, 42);
    }
}