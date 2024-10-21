use std::time::Duration;

use inel_reactor::op::{self, Op};

use crate::GlobalReactor;

pub async fn sleep(time: Duration) {
    op::Timeout::new(time).run_on(GlobalReactor).await;
}
