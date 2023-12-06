use bastion::context::BastionContext;
use flume::{Receiver, Sender};
use crate::actor::data_generator::WidgetInventory;

pub struct ApprovedWidgets {
    original_count: u128,
    approved_count: u128
}

#[cfg(not(test))]
pub async fn behavior(_ctx: BastionContext, _tx: Sender<ApprovedWidgets>, _rx: Receiver<WidgetInventory>) -> Result<(),()> {

    Ok(())
}

#[cfg(test)]
pub async fn behavior(_ctx: BastionContext, _tx: Sender<ApprovedWidgets>, _rx: Receiver<WidgetInventory>) -> Result<(),()> {

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