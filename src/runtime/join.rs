use futures::channel::oneshot::Receiver;

pub struct JoinHandle<T> {
    receiver: Receiver<T>,
}

impl<T> JoinHandle<T> {
    pub(crate) fn new(receiver: Receiver<T>) -> Self {
        Self { receiver }
    }

    pub(crate) fn try_join(&mut self) -> Option<T> {
        self.receiver.try_recv().ok()?
    }

    pub async fn join(self) -> Option<T> {
        self.receiver.await.ok()
    }
}
