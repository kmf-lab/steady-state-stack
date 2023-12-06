use bastion::context::BastionContext;
use flume::Receiver;
use crate::actor::data_approval::ApprovedWidgets;

#[cfg(not(test))]
pub async fn behavior(_ctx: BastionContext, rx: Receiver<ApprovedWidgets>) -> Result<(),()> {

    Ok(())
}

#[cfg(test)]
pub async fn behavior(ctx: BastionContext, rx: Receiver<ApprovedWidgets>) -> Result<(),()> {

    //ctx.tell

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