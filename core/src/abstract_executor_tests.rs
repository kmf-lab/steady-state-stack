#[cfg(test)]
mod tests {
    use crate::abstract_executor::*;
    use std::thread;
    use std::time::Duration;
    use crate::ProactorConfig;

    #[test]
    fn test_init_without_driver() {
        let config = ProactorConfig::InterruptDriven;
        init(false, config, 256);
    }

    #[test]
    fn test_init_with_driver() {
        let config = ProactorConfig::InterruptDriven;
        init(true, config, 256);
        thread::sleep(Duration::from_millis(100));
    }

    #[test]
    fn test_block_on() {
        let future = async { 42 };
        let result = block_on(future);
        assert_eq!(result, 42);
    }
}