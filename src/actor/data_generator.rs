use bastion::context::BastionContext;
use flume::Sender;

pub struct WidgetInventory {
    count: u128
}

#[cfg(not(test))]
pub async fn behavior(_ctx: BastionContext, tx_widget: Sender<WidgetInventory>) -> Result<(),()> {
    number_generator(tx_widget);
    Ok(())
}

#[cfg(test)]
pub async fn behavior(ctx: BastionContext, tx: Sender<WidgetInventory>) -> Result<(),()> {
    loop {
        ctx.recv().await;


    }
    Ok(())
}

fn number_generator(tx_widget: Sender<WidgetInventory>) -> u128 {

    let mut counter:u128 = 0;
    loop {
        // xmit

        counter += 1;


        //sleep
    }



}

#[cfg(test)]
mod tests {
    use super::*;
    use async_std::task;

    #[async_std::test]
    async fn test_something() {



    }

}