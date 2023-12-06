use bastion::context::BastionContext;
use flume::{Receiver, Sender};

pub struct SomeExampleRecord {

}

#[cfg(not(test))]
pub async fn behavior(_ctx: BastionContext, _tx: Sender<SomeExampleRecord>, _rx: Receiver<SomeExampleRecord>) -> Result<(),()> {

    Ok(())
}

#[cfg(test)]
pub async fn behavior(_ctx: BastionContext, _tx: Sender<SomeExampleRecord>, _rx: Receiver<SomeExampleRecord>) -> Result<(),()> {

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_std::task;

    #[async_std::test]
    async fn test_something() {

    }

}