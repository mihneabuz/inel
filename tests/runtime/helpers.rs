use std::{cell::RefCell, rc::Rc, sync::Once};

use tracing_subscriber::EnvFilter;

static TRACING: Once = Once::new();

pub fn setup_tracing() {
    TRACING.call_once(|| {
        tracing_subscriber::fmt()
            .with_env_filter(EnvFilter::from_default_env())
            .init();
    });
}

pub fn output<T>(init: T) -> (Rc<RefCell<T>>, Rc<RefCell<T>>) {
    let location = Rc::new(RefCell::new(init));
    (location.clone(), location)
}
